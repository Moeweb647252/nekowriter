//! 辅助工具预算账本（细则 §7.1）。
//!
//! 作用域在准备阶段 Run 与每章细纲 Run 创建时建立，只记录已用数；有效上限每次执行
//! 前从项目最新设置读取。收费以 `(scope_id, tool_call_id)` 唯一键登记，因此同一次
//! 工具调用被重复处理时不会重复扣除。

use nekowriter_core::{AuxiliaryToolCharge, QuotaBalance, RunId, utc_now_millis};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
    TransactionTrait,
};
use uuid::Uuid;

use super::{
    Store, StoreError, db_id, entity::quota_charge, entity::quota_scope, parse_id,
    transaction_error,
};

impl Store {
    /// 读取某个顶层写作 Run 的辅助工具预算余额。
    ///
    /// 上限取项目当前设置，因此调整预算后立即生效；没有作用域的 Run（例如审核子 Run）
    /// 返回上限为 0 的余额。
    ///
    /// # Errors
    ///
    /// Run 不存在，或数据库读取失败时返回错误。
    pub async fn quota_balance(&self, run_id: RunId) -> Result<QuotaBalance, StoreError> {
        let run = self.run_model(run_id).await?;
        let Some(scope_id) = run.quota_scope_id else {
            return Ok(QuotaBalance::new(0, 0));
        };
        let used = quota_scope::Entity::find_by_id(db_id(parse_id(&scope_id, "quota_scope_id")?))
            .one(&self.database)
            .await?
            .ok_or_else(|| StoreError::CorruptRecord("Run 引用的预算作用域不存在".to_owned()))?
            .used_count;
        let limit = self
            .get_project_settings(run.project_id.into())
            .await?
            .auxiliary_tool_budget;
        Ok(QuotaBalance::new(used, limit))
    }

    /// 为一次收费工具执行登记消耗。
    ///
    /// 余额为 0 时不登记任何记录，返回 [`AuxiliaryToolCharge::Exhausted`]，由调用方
    /// 形成 `QUOTA_EXHAUSTED` 工具结果。同一个 `provider_call_id` 重复到达时返回
    /// [`AuxiliaryToolCharge::AlreadyCharged`]，不重复扣除。
    ///
    /// # Errors
    ///
    /// Run 不存在、缺少预算作用域、工具名不是收费工具，或数据库操作失败时返回错误。
    pub async fn charge_auxiliary_tool(
        &self,
        run_id: RunId,
        provider_call_id: &str,
        tool_name: &str,
    ) -> Result<AuxiliaryToolCharge, StoreError> {
        if provider_call_id.trim().is_empty() {
            return Err(StoreError::InvalidModelRequest(
                "收费工具的 provider_call_id 不能为空".to_owned(),
            ));
        }
        if !nekowriter_core::is_chargeable_auxiliary_tool(tool_name) {
            return Err(StoreError::InvalidModelRequest(format!(
                "{tool_name} 不消耗辅助工具预算"
            )));
        }
        self.database
            .transaction::<_, _, StoreError>(|transaction| {
                let provider_call_id = provider_call_id.to_owned();
                let tool_name = tool_name.to_owned();
                Box::pin(async move {
                    let run = run_for_update(transaction, run_id).await?;
                    let scope_id = run.quota_scope_id.ok_or_else(|| {
                        StoreError::CorruptRecord("该 Run 没有辅助工具预算作用域".to_owned())
                    })?;
                    let scope_id = parse_id(&scope_id, "quota_scope_id")?;
                    let limit = project_auxiliary_tool_budget(transaction, &run.project_id).await?;
                    let scope_model = quota_scope::Entity::find_by_id(db_id(scope_id))
                        .one(transaction)
                        .await?
                        .ok_or_else(|| StoreError::CorruptRecord("预算作用域不存在".to_owned()))?;
                    let charged = quota_charge::Entity::find()
                        .filter(quota_charge::Entity::COLUMN.scope_id.eq(db_id(scope_id)))
                        .filter(
                            quota_charge::Entity::COLUMN
                                .tool_call_id
                                .eq(provider_call_id.clone()),
                        )
                        .one(transaction)
                        .await?
                        .is_some();
                    if charged {
                        return Ok(AuxiliaryToolCharge::AlreadyCharged(QuotaBalance::new(
                            scope_model.used_count,
                            limit,
                        )));
                    }
                    let balance = QuotaBalance::new(scope_model.used_count, limit);
                    if balance.remaining == 0 {
                        return Ok(AuxiliaryToolCharge::Exhausted(balance));
                    }
                    let now = utc_now_millis();
                    quota_charge::ActiveModel {
                        id: Set(db_id(Uuid::new_v4())),
                        scope_id: Set(db_id(scope_id)),
                        tool_call_id: Set(provider_call_id),
                        tool_name: Set(tool_name),
                        charged_at: Set(now),
                    }
                    .insert(transaction)
                    .await?;
                    let mut active_scope = scope_model.into_active_model();
                    active_scope.used_count = Set(balance.used + 1);
                    active_scope.updated_at = Set(now);
                    let updated = active_scope.update(transaction).await?;
                    Ok(AuxiliaryToolCharge::Charged(QuotaBalance::new(
                        updated.used_count,
                        limit,
                    )))
                })
            })
            .await
            .map_err(transaction_error)
    }

    async fn run_model(&self, run_id: RunId) -> Result<super::entity::run::Model, StoreError> {
        super::entity::run::Entity::find_by_id(db_id(run_id))
            .one(&self.database)
            .await?
            .ok_or(StoreError::RunNotFound)
    }
}

/// 建立或复用 `(version_id, stage_key)` 的预算作用域，返回它的 ID。
///
/// 阶段修订、暂停/继续与显式重试不会创建新作用域，因此这里按唯一键复用已有记录。
pub(crate) async fn ensure_quota_scope<C: ConnectionTrait>(
    connection: &C,
    version_id: sea_orm::prelude::TextUuid,
    stage_key: &str,
) -> Result<String, StoreError> {
    if let Some(existing) = quota_scope::Entity::find()
        .filter(quota_scope::Entity::COLUMN.version_id.eq(version_id))
        .filter(
            quota_scope::Entity::COLUMN
                .stage_key
                .eq(stage_key.to_owned()),
        )
        .one(connection)
        .await?
    {
        return Ok(existing.id.to_string());
    }
    let now = utc_now_millis();
    let id = Uuid::new_v4();
    quota_scope::ActiveModel {
        id: Set(db_id(id)),
        version_id: Set(version_id),
        stage_key: Set(stage_key.to_owned()),
        used_count: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(connection)
    .await?;
    Ok(id.to_string())
}

async fn run_for_update<C: ConnectionTrait>(
    connection: &C,
    run_id: RunId,
) -> Result<super::entity::run::Model, StoreError> {
    super::entity::run::Entity::find_by_id(db_id(run_id))
        .one(connection)
        .await?
        .ok_or(StoreError::RunNotFound)
}

async fn project_auxiliary_tool_budget<C: ConnectionTrait>(
    connection: &C,
    project_id: &sea_orm::prelude::TextUuid,
) -> Result<i64, StoreError> {
    let project = super::entity::project::Entity::find_by_id(*project_id)
        .filter(super::entity::project::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::ProjectNotFound)?;
    Ok(
        serde_json::from_str::<nekowriter_core::ProjectSettings>(&project.settings_json)
            .map_or_else(
                |_| nekowriter_core::DEFAULT_AUXILIARY_TOOL_BUDGET,
                |settings| settings.auxiliary_tool_budget,
            ),
    )
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{
        NewNovelBrief, NewProject, NewProjectSettings, TOOL_ASK_USER, TOOL_QUERY_KNOWLEDGE,
        ThinkingLevel,
    };
    use tempfile::TempDir;

    use super::*;

    async fn preparation_run(store: &Store) -> (nekowriter_core::ProjectId, RunId) {
        let created = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("预算账本", "辅助工具预算", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        store
            .save_novel_brief(
                created.version.id,
                Uuid::new_v4(),
                NewNovelBrief::new("写一部关于灯塔的短篇小说").unwrap(),
            )
            .await
            .unwrap();
        let workflow = store
            .get_version_snapshot(created.version.id)
            .await
            .unwrap()
            .workflow;
        let started = store
            .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
            .await
            .unwrap();
        (created.project.id, started.workflow.active_run_id.unwrap())
    }

    async fn set_budget(store: &Store, project_id: nekowriter_core::ProjectId, budget: i64) {
        let settings = store.get_project_settings(project_id).await.unwrap();
        store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                settings.revision,
                NewProjectSettings {
                    model: settings.model,
                    thinking_level: ThinkingLevel::default(),
                    auxiliary_tool_budget: budget,
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn quota_charges_once_per_tool_call_and_never_below_zero() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (project_id, run_id) = preparation_run(&store).await;
        set_budget(&store, project_id, 1).await;

        assert_eq!(store.quota_balance(run_id).await.unwrap().remaining, 1);

        // 首次调用扣除一次，重放同一个 provider_call_id 不再扣除。
        let first = store
            .charge_auxiliary_tool(run_id, "call-1", TOOL_QUERY_KNOWLEDGE)
            .await
            .unwrap();
        assert_eq!(first, AuxiliaryToolCharge::Charged(QuotaBalance::new(1, 1)));
        let replay = store
            .charge_auxiliary_tool(run_id, "call-1", TOOL_QUERY_KNOWLEDGE)
            .await
            .unwrap();
        assert_eq!(
            replay,
            AuxiliaryToolCharge::AlreadyCharged(QuotaBalance::new(1, 1))
        );

        // 余额耗尽时既不登记消耗，也不改变已用数。
        let exhausted = store
            .charge_auxiliary_tool(run_id, "call-2", TOOL_QUERY_KNOWLEDGE)
            .await
            .unwrap();
        assert_eq!(
            exhausted,
            AuxiliaryToolCharge::Exhausted(QuotaBalance::new(1, 1))
        );
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 1);

        // 上调预算立即生效，且不清空已用数；下调到已用数以下时剩余为 0。
        set_budget(&store, project_id, 3).await;
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 1);
        assert_eq!(store.quota_balance(run_id).await.unwrap().remaining, 2);
        set_budget(&store, project_id, 0).await;
        assert_eq!(store.quota_balance(run_id).await.unwrap().remaining, 0);
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 1);
    }

    #[tokio::test]
    async fn free_tools_and_other_runs_never_charge_the_scope() {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let (project_id, run_id) = preparation_run(&store).await;
        set_budget(&store, project_id, 5).await;

        let error = store
            .charge_auxiliary_tool(run_id, "call-ask", TOOL_ASK_USER)
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::InvalidModelRequest(_)));
        assert_eq!(store.quota_balance(run_id).await.unwrap().used, 0);

        let unknown = store
            .charge_auxiliary_tool(Uuid::new_v4(), "call-1", TOOL_QUERY_KNOWLEDGE)
            .await
            .unwrap_err();
        assert!(matches!(unknown, StoreError::RunNotFound));
    }
}
