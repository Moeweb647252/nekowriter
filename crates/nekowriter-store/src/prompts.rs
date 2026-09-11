//! 系统级与项目级内建提示词覆盖的持久化。
//!
//! 本模块只负责「哪个键、哪个作用域保存了哪一对 system/user 内容」。有效内容的
//! 三级优先级解析（项目 → 系统 → 代码内建）、变量校验与渲染都属于 Agent 合同，
//! 由 App 层用 `nekowriter-agent` 完成；Store 因此不依赖 Agent crate。
//!
//! 覆盖与命令回执、持久事件在同一 SQLite 事务中提交。删除行表示重置：空字符串
//! 覆盖是真实内容，与「没有覆盖」不同，所以二者不能共用一种表示。

use nekowriter_core::{ProjectId, decimal_i64, utc_now_millis};
use sea_orm::{
    ActiveModelTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::{
    Store, StoreError, db_id,
    entity::{event, project, prompt_override},
    insert_receipt, parse_optional_id, payload_hash, replay_or_conflict, transaction_error,
};

/// 单个提示词覆盖内容的长度上限（Unicode 字符）。
///
/// 提示词是可读的 Markdown，但不应被用来存放整部小说；上限只用于拒绝明显滥用，
/// 不限制正常的提示词改写。
const MAX_PROMPT_CONTENT_CHARS: usize = 200_000;

/// 提示词覆盖的作用域。
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOverrideScope {
    System,
    Project,
}

impl PromptOverrideScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Project => "project",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "system" => Ok(Self::System),
            "project" => Ok(Self::Project),
            other => Err(StoreError::CorruptRecord(format!(
                "提示词覆盖作用域 {other:?} 无效"
            ))),
        }
    }
}

/// 一条已持久化的提示词覆盖。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptOverride {
    pub prompt_key: String,
    pub scope: PromptOverrideScope,
    pub project_id: Option<ProjectId>,
    pub system: String,
    pub user: String,
    #[serde(with = "decimal_i64")]
    pub revision: i64,
    #[serde(with = "decimal_i64")]
    pub created_at_ms: i64,
    #[serde(with = "decimal_i64")]
    pub updated_at_ms: i64,
}

/// 待保存的提示词覆盖内容。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPromptOverride {
    pub prompt_key: String,
    pub scope: PromptOverrideScope,
    pub project_id: Option<ProjectId>,
    pub system: String,
    pub user: String,
}

/// 重置（删除覆盖）的结果。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptOverrideReset {
    pub prompt_key: String,
    pub scope: PromptOverrideScope,
    pub project_id: Option<ProjectId>,
    /// 本次命令是否真的删除了覆盖；重复重置返回 `false`。
    pub removed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavePromptOverridePayload<'a> {
    expected_revision: i64,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &'a str,
    system: &'a str,
    user: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetPromptOverridePayload<'a> {
    expected_revision: i64,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &'a str,
}

impl Store {
    /// 列出某个作用域下的全部提示词覆盖。
    ///
    /// # Errors
    ///
    /// 作用域与 `project_id` 不匹配，或数据库记录损坏时返回错误。
    pub async fn list_prompt_overrides(
        &self,
        scope: PromptOverrideScope,
        project_id: Option<ProjectId>,
    ) -> Result<Vec<PromptOverride>, StoreError> {
        ensure_scope_matches_project(scope, project_id)?;
        let query = prompt_override::Entity::find()
            .filter(prompt_override::Entity::COLUMN.scope.eq(scope.as_str()))
            .order_by_asc(prompt_override::Entity::COLUMN.prompt_key);
        let query = match project_id {
            Some(project_id) => query.filter(
                prompt_override::Entity::COLUMN
                    .project_id
                    .eq(project_id.to_string()),
            ),
            None => query.filter(prompt_override::Entity::COLUMN.project_id.is_null()),
        };
        query
            .all(&self.database)
            .await?
            .into_iter()
            .map(|model| prompt_override_from_model(&model))
            .collect()
    }

    /// 读取单个键在该作用域下的一对覆盖内容；没有覆盖时返回 `None`。
    ///
    /// # Errors
    ///
    /// 提示词键无效、作用域与 `project_id` 不匹配，或数据库记录损坏时返回错误。
    pub async fn prompt_override(
        &self,
        scope: PromptOverrideScope,
        project_id: Option<ProjectId>,
        prompt_key: &str,
    ) -> Result<Option<PromptOverride>, StoreError> {
        ensure_scope_matches_project(scope, project_id)?;
        let prompt_key = normalize_prompt_key(prompt_key)?;
        let query = prompt_override::Entity::find()
            .filter(prompt_override::Entity::COLUMN.scope.eq(scope.as_str()))
            .filter(prompt_override::Entity::COLUMN.prompt_key.eq(prompt_key));
        let query = match project_id {
            Some(project_id) => query.filter(
                prompt_override::Entity::COLUMN
                    .project_id
                    .eq(project_id.to_string()),
            ),
            None => query.filter(prompt_override::Entity::COLUMN.project_id.is_null()),
        };
        query
            .one(&self.database)
            .await?
            .map(|model| prompt_override_from_model(&model))
            .transpose()
    }

    /// 保存一条提示词覆盖，并写入持久事件与命令回执。
    ///
    /// `expected_revision` 为该覆盖的乐观并发修订号，尚无覆盖时为 0。
    ///
    /// # Errors
    ///
    /// 内容超长、提示词键或作用域无效、项目不存在、修订冲突、`command_id` 复用于
    /// 不同请求或数据库操作失败时返回错误。
    pub async fn save_prompt_override(
        &self,
        command_id: Uuid,
        expected_revision: i64,
        input: NewPromptOverride,
    ) -> Result<PromptOverride, StoreError> {
        let input = normalize_prompt_override(&input)?;
        if expected_revision < 0 {
            return Err(invalid_prompt_override(
                "expected_revision 不能为负数".to_owned(),
            ));
        }
        let target = prompt_override_target(input.scope, input.project_id, &input.prompt_key)?;
        let payload = SavePromptOverridePayload {
            expected_revision,
            scope: input.scope,
            project_id: input.project_id,
            prompt_key: &input.prompt_key,
            system: &input.system,
            user: &input.user,
        };
        let payload_hash = payload_hash(&payload)?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, "PUT", &target, &payload_hash);
        }

        let input_for_transaction = input.clone();
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    ensure_project_exists(transaction, input_for_transaction.project_id).await?;
                    let existing = find_override_model(
                        transaction,
                        input_for_transaction.scope,
                        input_for_transaction.project_id,
                        &input_for_transaction.prompt_key,
                    )
                    .await?;
                    let actual_revision = existing.as_ref().map_or(0, |model| model.revision);
                    if actual_revision != expected_revision {
                        return Err(StoreError::PromptOverrideRevisionConflict {
                            expected: expected_revision,
                            actual: actual_revision,
                        });
                    }
                    let revision = actual_revision.checked_add(1).ok_or_else(|| {
                        invalid_prompt_override("提示词覆盖修订已达到 i64 上限".to_owned())
                    })?;
                    let model = match existing {
                        Some(model) => {
                            let mut active = model.into_active_model();
                            active.system_content = Set(input_for_transaction.system.clone());
                            active.user_content = Set(input_for_transaction.user.clone());
                            active.revision = Set(revision);
                            active.updated_at = Set(now);
                            active.update(transaction).await?
                        }
                        None => {
                            prompt_override::ActiveModel {
                                id: Set(db_id(Uuid::new_v4())),
                                scope: Set(input_for_transaction.scope.as_str().to_owned()),
                                project_id: Set(input_for_transaction
                                    .project_id
                                    .map(|project_id| project_id.to_string())),
                                prompt_key: Set(input_for_transaction.prompt_key.clone()),
                                system_content: Set(input_for_transaction.system.clone()),
                                user_content: Set(input_for_transaction.user.clone()),
                                revision: Set(revision),
                                created_at: Set(now),
                                updated_at: Set(now),
                            }
                            .insert(transaction)
                            .await?
                        }
                    };
                    let response = prompt_override_from_model(&model)?;
                    insert_prompt_override_event(transaction, &response, "saved", now).await?;
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "PUT",
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
            Ok(saved) => {
                self.notify_event_listeners();
                Ok(saved)
            }
            Err(StoreError::Database(error)) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    "PUT",
                    &target,
                    &payload_hash,
                    StoreError::Database(error),
                )
                .await
            }
            Err(error) => Err(error),
        }
    }

    /// 删除一条提示词覆盖，回到更低优先级的来源。
    ///
    /// 尚无覆盖时 `expected_revision` 必须为 0；此时返回 `removed = false`，重复重
    /// 置不会报错，也不会再次写入事件。
    ///
    /// # Errors
    ///
    /// 提示词键或作用域无效、项目不存在、修订冲突、`command_id` 复用于不同请求或
    /// 数据库操作失败时返回错误。
    pub async fn reset_prompt_override(
        &self,
        command_id: Uuid,
        expected_revision: i64,
        scope: PromptOverrideScope,
        project_id: Option<ProjectId>,
        prompt_key: &str,
    ) -> Result<PromptOverrideReset, StoreError> {
        ensure_scope_matches_project(scope, project_id)?;
        if expected_revision < 0 {
            return Err(invalid_prompt_override(
                "expected_revision 不能为负数".to_owned(),
            ));
        }
        let prompt_key = normalize_prompt_key(prompt_key)?;
        let target = prompt_override_target(scope, project_id, &prompt_key)?;
        let payload = ResetPromptOverridePayload {
            expected_revision,
            scope,
            project_id,
            prompt_key: &prompt_key,
        };
        let payload_hash = payload_hash(&payload)?;
        if let Some(receipt) = self.find_receipt(command_id).await? {
            return replay_or_conflict(receipt, "DELETE", &target, &payload_hash);
        }

        let prompt_key_for_transaction = prompt_key.clone();
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        let now = utc_now_millis();
        let result = self
            .database
            .transaction::<_, _, StoreError>(move |transaction| {
                Box::pin(async move {
                    ensure_project_exists(transaction, project_id).await?;
                    let existing = find_override_model(
                        transaction,
                        scope,
                        project_id,
                        &prompt_key_for_transaction,
                    )
                    .await?;
                    let actual_revision = existing.as_ref().map_or(0, |model| model.revision);
                    if actual_revision != expected_revision {
                        return Err(StoreError::PromptOverrideRevisionConflict {
                            expected: expected_revision,
                            actual: actual_revision,
                        });
                    }
                    let removed = if let Some(model) = existing {
                        let response = prompt_override_from_model(&model)?;
                        prompt_override::Entity::delete_by_id(model.id)
                            .exec(transaction)
                            .await?;
                        insert_prompt_override_event(transaction, &response, "removed", now)
                            .await?;
                        true
                    } else {
                        false
                    };
                    let response = PromptOverrideReset {
                        prompt_key: prompt_key_for_transaction.clone(),
                        scope,
                        project_id,
                        removed,
                    };
                    let response_json = serde_json::to_string(&response)?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "DELETE",
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
            Ok(reset) => {
                if reset.removed {
                    self.notify_event_listeners();
                }
                Ok(reset)
            }
            Err(StoreError::Database(error)) => {
                self.replay_after_receipt_conflict(
                    command_id,
                    "DELETE",
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

fn ensure_scope_matches_project(
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
) -> Result<(), StoreError> {
    match (scope, project_id) {
        (PromptOverrideScope::System, None) | (PromptOverrideScope::Project, Some(_)) => Ok(()),
        (PromptOverrideScope::System, Some(_)) => Err(invalid_prompt_override(
            "系统级提示词覆盖不能指定 project_id".to_owned(),
        )),
        (PromptOverrideScope::Project, None) => Err(invalid_prompt_override(
            "项目级提示词覆盖必须指定 project_id".to_owned(),
        )),
    }
}

fn prompt_override_target(
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &str,
) -> Result<String, StoreError> {
    ensure_scope_matches_project(scope, project_id)?;
    Ok(match project_id {
        Some(project_id) => format!("/api/projects/{project_id}/prompts/{prompt_key}"),
        None => format!("/api/prompts/system/{prompt_key}"),
    })
}

fn normalize_prompt_override(input: &NewPromptOverride) -> Result<NewPromptOverride, StoreError> {
    ensure_scope_matches_project(input.scope, input.project_id)?;
    let prompt_key = normalize_prompt_key(&input.prompt_key)?;
    let system = normalize_prompt_content("system", &input.system)?;
    let user = normalize_prompt_content("user", &input.user)?;
    Ok(NewPromptOverride {
        prompt_key,
        scope: input.scope,
        project_id: input.project_id,
        system,
        user,
    })
}

fn normalize_prompt_key(value: &str) -> Result<String, StoreError> {
    let value = value.trim();
    let valid = !value.is_empty()
        && value.chars().count() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'_'
        });
    if !valid {
        return Err(invalid_prompt_override(
            "提示词键必须为 1–64 个小写字母、数字、点或下划线".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

fn normalize_prompt_content(name: &str, value: &str) -> Result<String, StoreError> {
    // 与正文一致地统一到 LF，避免同一份提示词因编辑器换行符不同而产生不同哈希。
    let normalized = value.replace("\r\n", "\n");
    if normalized.chars().count() > MAX_PROMPT_CONTENT_CHARS {
        return Err(invalid_prompt_override(format!(
            "{name} 内容超过 {MAX_PROMPT_CONTENT_CHARS} 个字符上限"
        )));
    }
    Ok(normalized)
}

fn invalid_prompt_override(reason: String) -> StoreError {
    StoreError::InvalidPromptOverride(reason)
}

async fn ensure_project_exists<C: sea_orm::ConnectionTrait>(
    connection: &C,
    project_id: Option<ProjectId>,
) -> Result<(), StoreError> {
    let Some(project_id) = project_id else {
        return Ok(());
    };
    let exists = project::Entity::find_by_id(db_id(project_id))
        .filter(project::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .is_some();
    if !exists {
        return Err(StoreError::ProjectNotFound);
    }
    Ok(())
}

async fn find_override_model<C: sea_orm::ConnectionTrait>(
    connection: &C,
    scope: PromptOverrideScope,
    project_id: Option<ProjectId>,
    prompt_key: &str,
) -> Result<Option<prompt_override::Model>, StoreError> {
    let query = prompt_override::Entity::find()
        .filter(prompt_override::Entity::COLUMN.scope.eq(scope.as_str()))
        .filter(prompt_override::Entity::COLUMN.prompt_key.eq(prompt_key));
    let query = match project_id {
        Some(project_id) => query.filter(
            prompt_override::Entity::COLUMN
                .project_id
                .eq(project_id.to_string()),
        ),
        None => query.filter(prompt_override::Entity::COLUMN.project_id.is_null()),
    };
    Ok(query.one(connection).await?)
}

fn prompt_override_from_model(
    model: &prompt_override::Model,
) -> Result<PromptOverride, StoreError> {
    let scope = PromptOverrideScope::parse(&model.scope)?;
    let project_id = parse_optional_id(model.project_id.clone(), "prompt_overrides.project_id")?;
    ensure_scope_matches_project(scope, project_id)?;
    let prompt_key = normalize_prompt_key(&model.prompt_key)?;
    if model.revision <= 0 || model.updated_at < model.created_at {
        return Err(StoreError::CorruptRecord(
            "提示词覆盖的修订或时间戳无效".to_owned(),
        ));
    }
    Ok(PromptOverride {
        prompt_key,
        scope,
        project_id,
        system: model.system_content.clone(),
        user: model.user_content.clone(),
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    })
}

async fn insert_prompt_override_event(
    transaction: &sea_orm::DatabaseTransaction,
    response: &PromptOverride,
    action: &str,
    now: i64,
) -> Result<(), StoreError> {
    event::ActiveModel {
        version_id: Set(None),
        event_type: Set("settings.changed".to_owned()),
        entity_id: Set(db_id(Uuid::new_v4())),
        revision: Set(response.revision.to_string()),
        payload_json: Set(serde_json::to_string(&json!({
            "scope": response.scope.as_str(),
            "project_id": response.project_id.map(|id| id.to_string()),
            "prompt_key": response.prompt_key,
            "action": action,
            "revision": response.revision.to_string(),
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
    use tempfile::TempDir;

    use super::*;
    use nekowriter_core::{NewNovelVersion, NewProject};

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
    async fn prompt_overrides_round_trip_and_reset_by_scope() {
        let (_data_dir, store, project_id) = store_with_project().await;

        let saved = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: "chapter.write".to_owned(),
                    scope: PromptOverrideScope::System,
                    project_id: None,
                    system: "系统覆盖 {{chapter_content}}".to_owned(),
                    user: "用户覆盖 {{chapter_guidance}}".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.project_id, None);

        let project_saved = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: "chapter.write".to_owned(),
                    scope: PromptOverrideScope::Project,
                    project_id: Some(project_id),
                    system: "项目覆盖 {{chapter_content}}".to_owned(),
                    user: "项目用户覆盖 {{chapter_guidance}}".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(project_saved.project_id, Some(project_id));

        // 同一稳定键在两个作用域下各自独立。
        assert_eq!(
            store
                .prompt_override(PromptOverrideScope::System, None, "chapter.write")
                .await
                .unwrap()
                .unwrap()
                .system,
            "系统覆盖 {{chapter_content}}"
        );
        assert_eq!(
            store
                .prompt_override(
                    PromptOverrideScope::Project,
                    Some(project_id),
                    "chapter.write"
                )
                .await
                .unwrap()
                .unwrap()
                .system,
            "项目覆盖 {{chapter_content}}"
        );
        assert_eq!(
            store
                .list_prompt_overrides(PromptOverrideScope::System, None)
                .await
                .unwrap()
                .len(),
            1
        );

        let conflict = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: "chapter.write".to_owned(),
                    scope: PromptOverrideScope::System,
                    project_id: None,
                    system: "过期写入".to_owned(),
                    user: "过期写入".to_owned(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            StoreError::PromptOverrideRevisionConflict {
                expected: 0,
                actual: 1
            }
        ));

        let reset = store
            .reset_prompt_override(
                Uuid::new_v4(),
                1,
                PromptOverrideScope::System,
                None,
                "chapter.write",
            )
            .await
            .unwrap();
        assert!(reset.removed);
        assert!(
            store
                .prompt_override(PromptOverrideScope::System, None, "chapter.write")
                .await
                .unwrap()
                .is_none()
        );
        // 项目级覆盖不受系统级重置影响。
        assert!(
            store
                .prompt_override(
                    PromptOverrideScope::Project,
                    Some(project_id),
                    "chapter.write"
                )
                .await
                .unwrap()
                .is_some()
        );

        let repeated = store
            .reset_prompt_override(
                Uuid::new_v4(),
                0,
                PromptOverrideScope::System,
                None,
                "chapter.write",
            )
            .await
            .unwrap();
        assert!(!repeated.removed);
    }

    #[tokio::test]
    async fn prompt_override_commands_replay_by_command_id() {
        let (_data_dir, store, _project_id) = store_with_project().await;
        let command_id = Uuid::new_v4();
        let input = NewPromptOverride {
            prompt_key: "preparation.generate".to_owned(),
            scope: PromptOverrideScope::System,
            project_id: None,
            system: "覆盖 {{novel_brief}}".to_owned(),
            user: "用户 {{novel_brief}}".to_owned(),
        };
        let first = store
            .save_prompt_override(command_id, 0, input.clone())
            .await
            .unwrap();
        let replayed = store
            .save_prompt_override(command_id, 0, input)
            .await
            .unwrap();
        assert_eq!(first, replayed);

        let conflicting = store
            .save_prompt_override(
                command_id,
                1,
                NewPromptOverride {
                    prompt_key: "preparation.generate".to_owned(),
                    scope: PromptOverrideScope::System,
                    project_id: None,
                    system: "不同内容".to_owned(),
                    user: "不同内容".to_owned(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(conflicting, StoreError::IdempotencyConflict));
    }

    #[tokio::test]
    async fn prompt_overrides_reject_scope_and_key_mismatches() {
        let (_data_dir, store, project_id) = store_with_project().await;
        let missing_project = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: "chapter.write".to_owned(),
                    scope: PromptOverrideScope::Project,
                    project_id: Some(Uuid::new_v4()),
                    system: "内容".to_owned(),
                    user: "内容".to_owned(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(missing_project, StoreError::ProjectNotFound));

        let system_with_project = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: "chapter.write".to_owned(),
                    scope: PromptOverrideScope::System,
                    project_id: Some(project_id),
                    system: "内容".to_owned(),
                    user: "内容".to_owned(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            system_with_project,
            StoreError::InvalidPromptOverride(_)
        ));

        let empty_key = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: " ".to_owned(),
                    scope: PromptOverrideScope::System,
                    project_id: None,
                    system: "内容".to_owned(),
                    user: "内容".to_owned(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(empty_key, StoreError::InvalidPromptOverride(_)));

        // 空字符串是合法的实际覆盖内容，不是「没有覆盖」。
        let saved = store
            .save_prompt_override(
                Uuid::new_v4(),
                0,
                NewPromptOverride {
                    prompt_key: "chapter.outline".to_owned(),
                    scope: PromptOverrideScope::Project,
                    project_id: Some(project_id),
                    system: String::new(),
                    user: String::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.system, "");
        assert_eq!(saved.revision, 1);
    }

    #[tokio::test]
    async fn prompt_overrides_resolve_their_project_from_a_version() {
        let (_data_dir, store, project_id) = store_with_project().await;
        let version = store
            .create_version(
                project_id,
                Uuid::new_v4(),
                NewNovelVersion::new("第二个稿本", "雨夜的来客").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .project_id_for_version(version.version.id)
                .await
                .unwrap(),
            project_id
        );
    }
}
