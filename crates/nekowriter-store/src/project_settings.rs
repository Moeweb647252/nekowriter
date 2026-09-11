//! 项目级创作设置的持久化与乐观并发控制。
//!
//! 项目设置与项目本身分开保存修订号：`projects.revision` 属于项目实体（名称、当前
//! 选中稿本），`projects.settings_revision` 只随设置变化，避免保存设置让正在进行的
//! 稿本切换命令意外过期。
//!
//! 设置内容存在 `settings_json` 中；缺失字段按 [`ProjectSettings`] 的字段默认值解析，
//! 因此较早写入的 `{}` 表示「未显式设置」而不是损坏记录。

use nekowriter_core::{
    MAX_AUXILIARY_TOOL_BUDGET, MAX_PROJECT_MODEL_ID_LEN, NewProjectSettings, ProjectId,
    ProjectSettings, utc_now_millis,
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QueryFilter, Set, TransactionTrait};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use super::{
    Store, StoreError, db_id,
    entity::{event, project},
    insert_receipt, payload_hash, replay_or_conflict, transaction_error,
};

const SAVE_PROJECT_SETTINGS_METHOD: &str = "PUT";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveProjectSettingsPayload<'a> {
    expected_revision: i64,
    model_id: Option<&'a str>,
    thinking_level: nekowriter_core::ThinkingLevel,
    auxiliary_tool_budget: i64,
}

impl Store {
    /// 读取项目设置；未显式设置过的字段返回默认值。
    ///
    /// # Errors
    ///
    /// 项目不存在或已删除，或已保存的设置越界、JSON 损坏时返回错误。
    pub async fn get_project_settings(
        &self,
        project_id: ProjectId,
    ) -> Result<ProjectSettings, StoreError> {
        let model = project::Entity::find_by_id(db_id(project_id))
            .filter(project::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .ok_or(StoreError::ProjectNotFound)?;
        project_settings_from_model(&model)
    }

    /// 保存项目设置，并写入持久事件与命令回执。
    ///
    /// # Errors
    ///
    /// 设置越界、项目不存在、修订冲突、`command_id` 复用于不同请求或数据库操作
    /// 失败时返回错误。
    pub async fn save_project_settings(
        &self,
        project_id: ProjectId,
        command_id: Uuid,
        expected_revision: i64,
        input: NewProjectSettings,
    ) -> Result<ProjectSettings, StoreError> {
        let input = normalize_new_project_settings(input)?;
        if expected_revision < 0 {
            return Err(invalid_project_settings("expected_revision 不能为负数"));
        }
        let target = format!("/api/projects/{project_id}/settings");
        let payload_hash = payload_hash(&SaveProjectSettingsPayload {
            expected_revision,
            model_id: input.model_id.as_deref(),
            thinking_level: input.thinking_level,
            auxiliary_tool_budget: input.auxiliary_tool_budget,
        })?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(
                receipt,
                SAVE_PROJECT_SETTINGS_METHOD,
                &target,
                &payload_hash,
            );
        }

        let input_for_transaction = input.clone();
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    let model = project::Entity::find_by_id(db_id(project_id))
                        .filter(project::Entity::COLUMN.deleted_at.is_null())
                        .one(transaction)
                        .await?
                        .ok_or(StoreError::ProjectNotFound)?;
                    let actual_revision = model.settings_revision;
                    if actual_revision != expected_revision {
                        return Err(StoreError::ProjectSettingsRevisionConflict {
                            expected: expected_revision,
                            actual: actual_revision,
                        });
                    }
                    let revision = actual_revision
                        .checked_add(1)
                        .ok_or_else(|| invalid_project_settings("项目设置修订已达到 i64 上限"))?;
                    let settings = ProjectSettings {
                        model_id: input_for_transaction.model_id.clone(),
                        thinking_level: input_for_transaction.thinking_level,
                        auxiliary_tool_budget: input_for_transaction.auxiliary_tool_budget,
                        revision,
                    };
                    let mut active = model.into_active_model();
                    active.settings_json = Set(serde_json::to_string(&settings)?);
                    active.settings_revision = Set(revision);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    let response = project_settings_from_model(&updated)?;
                    insert_project_settings_event(transaction, project_id, &response, now).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        SAVE_PROJECT_SETTINGS_METHOD,
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &response_json,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
            .map_err(transaction_error);

        match result {
            Ok(settings) => {
                self.notify_event_listeners();
                Ok(settings)
            }
            Err(StoreError::Database(error)) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    SAVE_PROJECT_SETTINGS_METHOD,
                    &target,
                    &payload_hash,
                    StoreError::Database(error),
                )
                .await
            }
            Err(error) => Err(error),
        }
    }
}

fn normalize_new_project_settings(
    mut input: NewProjectSettings,
) -> Result<NewProjectSettings, StoreError> {
    input.model_id = match input.model_id {
        Some(model_id) => {
            let model_id = model_id.trim().to_owned();
            if model_id.is_empty() {
                // 空字符串与 `null` 在界面上是同一个选择：沿用系统默认模型。
                None
            } else if model_id.chars().count() > MAX_PROJECT_MODEL_ID_LEN {
                return Err(invalid_project_settings(format!(
                    "项目模型 ID 必须为 1–{MAX_PROJECT_MODEL_ID_LEN} 个字符"
                )));
            } else {
                Some(model_id)
            }
        }
        None => None,
    };
    if !(0..=MAX_AUXILIARY_TOOL_BUDGET).contains(&input.auxiliary_tool_budget) {
        return Err(invalid_project_settings(format!(
            "辅助工具预算必须为 0–{MAX_AUXILIARY_TOOL_BUDGET} 的整数"
        )));
    }
    Ok(input)
}

fn project_settings_from_model(model: &project::Model) -> Result<ProjectSettings, StoreError> {
    let parsed: ProjectSettings = serde_json::from_str(&model.settings_json)
        .map_err(|error| StoreError::CorruptRecord(format!("项目设置 JSON 损坏：{error}")))?;
    // 修订以列为准，JSON 里的值只用于自描述。
    let settings = ProjectSettings {
        revision: model.settings_revision,
        ..parsed
    };
    normalize_new_project_settings(NewProjectSettings {
        model_id: settings.model_id.clone(),
        thinking_level: settings.thinking_level,
        auxiliary_tool_budget: settings.auxiliary_tool_budget,
    })?;
    if settings.revision < 0 {
        return Err(StoreError::CorruptRecord(
            "项目设置的修订不能为负数".to_owned(),
        ));
    }
    Ok(settings)
}

fn invalid_project_settings(reason: impl Into<String>) -> StoreError {
    StoreError::InvalidProjectSettings(reason.into())
}

async fn insert_project_settings_event(
    transaction: &sea_orm::DatabaseTransaction,
    project_id: ProjectId,
    settings: &ProjectSettings,
    now: i64,
) -> Result<(), StoreError> {
    event::ActiveModel {
        version_id: Set(None),
        event_type: Set("settings.changed".to_owned()),
        entity_id: Set(db_id(project_id)),
        revision: Set(settings.revision.to_string()),
        payload_json: Set(serde_json::to_string(&json!({
            "scope": "project",
            "project_id": project_id.to_string(),
            "revision": settings.revision.to_string(),
            "model_id": settings.model_id.clone(),
            "thinking_level": settings.thinking_level.as_str(),
            "auxiliary_tool_budget": settings.auxiliary_tool_budget.to_string(),
        }))?),
        occurred_at: Set(now),
        ..Default::default()
    }
    .insert(transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{NewProject, ThinkingLevel};
    use tempfile::TempDir;

    use super::*;

    async fn store_with_project() -> (TempDir, Store, ProjectId) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        let bundle = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("雨夜计划", "雨夜的来客", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let project_id = bundle.project.id;
        (data_dir, store, project_id)
    }

    #[tokio::test]
    async fn project_settings_start_from_documented_defaults() {
        let (_data_dir, store, project_id) = store_with_project().await;
        let settings = store.get_project_settings(project_id).await.unwrap();
        assert_eq!(settings, ProjectSettings::default());
        assert_eq!(settings.auxiliary_tool_budget, 10);
        assert_eq!(settings.thinking_level, ThinkingLevel::Off);
        assert_eq!(settings.model_id, None);
        assert_eq!(settings.revision, 0);
    }

    #[tokio::test]
    async fn project_settings_save_bumps_only_the_settings_revision() {
        let (_data_dir, store, project_id) = store_with_project().await;
        let before = store.get_project(project_id).await.unwrap().project;
        let saved = store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                0,
                NewProjectSettings {
                    model_id: Some(" project-model ".to_owned()),
                    thinking_level: ThinkingLevel::High,
                    auxiliary_tool_budget: 25,
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.model_id.as_deref(), Some("project-model"));
        assert_eq!(saved.thinking_level, ThinkingLevel::High);
        assert_eq!(saved.auxiliary_tool_budget, 25);
        assert_eq!(saved.revision, 1);
        assert_eq!(store.get_project_settings(project_id).await.unwrap(), saved);
        // 项目实体修订不变，因此正在进行的稿本切换命令不会意外过期。
        assert_eq!(
            store
                .get_project(project_id)
                .await
                .unwrap()
                .project
                .revision,
            before.revision
        );

        let conflict = store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                0,
                NewProjectSettings {
                    model_id: None,
                    thinking_level: ThinkingLevel::Off,
                    auxiliary_tool_budget: 0,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            StoreError::ProjectSettingsRevisionConflict {
                expected: 0,
                actual: 1
            }
        ));
    }

    #[tokio::test]
    async fn project_settings_commands_replay_and_reject_invalid_values() {
        let (_data_dir, store, project_id) = store_with_project().await;
        let command_id = Uuid::new_v4();
        let input = NewProjectSettings {
            model_id: None,
            thinking_level: ThinkingLevel::Medium,
            auxiliary_tool_budget: 5,
        };
        let first = store
            .save_project_settings(project_id, command_id, 0, input.clone())
            .await
            .unwrap();
        let replayed = store
            .save_project_settings(project_id, command_id, 0, input)
            .await
            .unwrap();
        assert_eq!(first, replayed);

        let negative = store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                1,
                NewProjectSettings {
                    model_id: None,
                    thinking_level: ThinkingLevel::Off,
                    auxiliary_tool_budget: -1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(negative, StoreError::InvalidProjectSettings(_)));

        let too_large = store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                1,
                NewProjectSettings {
                    model_id: None,
                    thinking_level: ThinkingLevel::Off,
                    auxiliary_tool_budget: MAX_AUXILIARY_TOOL_BUDGET + 1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(too_large, StoreError::InvalidProjectSettings(_)));

        // 空白模型 ID 与「沿用系统默认」等价。
        let blank_model = store
            .save_project_settings(
                project_id,
                Uuid::new_v4(),
                1,
                NewProjectSettings {
                    model_id: Some("   ".to_owned()),
                    thinking_level: ThinkingLevel::Off,
                    auxiliary_tool_budget: 5,
                },
            )
            .await
            .unwrap();
        assert_eq!(blank_model.model_id, None);
    }

    #[tokio::test]
    async fn existing_empty_settings_json_parses_as_defaults() {
        let (_data_dir, store, project_id) = store_with_project().await;
        // 项目创建时写入的是 `{}`；它表示未显式设置，而不是损坏记录。
        let model = project::Entity::find_by_id(db_id(project_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(model.settings_json, "{}");
        assert_eq!(
            store.get_project_settings(project_id).await.unwrap(),
            ProjectSettings::default()
        );
    }
}
