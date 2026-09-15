//! 用户提示词模板的持久化。
//!
//! 模板是独立可复用内容，不会自动获得 system 消息权限。保存与修改都在 Store
//! 边界重新执行一次受限模板语言校验，因此客户端不能靠本地校验绕过未知变量、
//! 缺失必需占位符或不完整括号的限制。删除为软删除。

use nekowriter_core::{NewPromptTemplate, PromptTemplate, TemplateVariable, utc_now_millis};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde::Serialize;
use uuid::Uuid;

use super::{
    Command, Notify, Store, StoreError, db_id, entity::prompt_template, insert_receipt,
    payload_hash, preparation::insert_app_event,
};

/// 模板保存与修改的请求体。创建与更新使用同一形状，更新另带预期修订。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TemplateContentPayload<'a> {
    title: &'a str,
    body: &'a str,
    variables: &'a [TemplateVariable],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTemplatePayload<'a> {
    expected_revision: i64,
    title: &'a str,
    body: &'a str,
    variables: &'a [TemplateVariable],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteTemplatePayload {
    expected_revision: i64,
}

impl Store {
    /// 列出全部未删除的模板，按更新时间倒序。
    ///
    /// # Errors
    ///
    /// 数据库读取失败或记录损坏时返回错误。
    pub async fn list_prompt_templates(&self) -> Result<Vec<PromptTemplate>, StoreError> {
        let models = prompt_template::Entity::find()
            .filter(prompt_template::Entity::COLUMN.deleted_at.is_null())
            .order_by_desc(prompt_template::Entity::COLUMN.updated_at)
            .order_by_desc(prompt_template::Entity::COLUMN.id)
            .all(&self.database)
            .await?;
        models.into_iter().map(prompt_template_from_model).collect()
    }

    /// 读取一份模板。
    ///
    /// # Errors
    ///
    /// 模板不存在或已删除、记录损坏时返回错误。
    pub async fn get_prompt_template(
        &self,
        template_id: Uuid,
    ) -> Result<PromptTemplate, StoreError> {
        let model = active_prompt_template(&self.database, template_id).await?;
        prompt_template_from_model(model)
    }

    /// 新建一份模板。
    ///
    /// # Errors
    ///
    /// 标题或正文无效、正文引用了未声明变量、缺少必需占位符、
    /// `command_id` 复用于不同请求或数据库操作失败时返回错误。
    pub async fn create_prompt_template(
        &self,
        command_id: Uuid,
        input: NewPromptTemplate,
    ) -> Result<PromptTemplate, StoreError> {
        let draft = validate_draft(input)?;
        let target = "/api/prompt-templates".to_owned();
        let payload_hash = payload_hash(&TemplateContentPayload {
            title: &draft.title,
            body: &draft.body,
            variables: &draft.variables,
        })?;
        let command = Command::new(command_id, "POST", &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = prompt_template::ActiveModel {
                        id: Set(db_id(Uuid::new_v4())),
                        title: Set(draft.title.clone()),
                        body: Set(draft.body.clone()),
                        variables_json: Set(serde_json::to_string(&draft.variables)?),
                        revision: Set(1),
                        created_at: Set(now),
                        updated_at: Set(now),
                        deleted_at: Set(None),
                    }
                    .insert(transaction)
                    .await?;
                    let response = prompt_template_from_model(model)?;
                    insert_app_event(
                        transaction,
                        "prompt_template.changed",
                        response.id,
                        response.revision,
                        serde_json::json!({ "deleted": false }),
                        now,
                    )
                    .await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "POST",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        201,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 替换一份模板的内容；变量声明必须与正文一起提交。
    ///
    /// # Errors
    ///
    /// 模板不存在、修订冲突、内容无效、`command_id` 复用于不同请求或数据库操作
    /// 失败时返回错误。
    pub async fn update_prompt_template(
        &self,
        template_id: Uuid,
        command_id: Uuid,
        expected_revision: i64,
        input: NewPromptTemplate,
    ) -> Result<PromptTemplate, StoreError> {
        let draft = validate_draft(input)?;
        let target = format!("/api/prompt-templates/{template_id}");
        let payload_hash = payload_hash(&UpdateTemplatePayload {
            expected_revision,
            title: &draft.title,
            body: &draft.body,
            variables: &draft.variables,
        })?;
        let command = Command::new(command_id, "PATCH", &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = active_prompt_template(transaction, template_id).await?;
                    if model.revision != expected_revision {
                        return Err(StoreError::PromptTemplateRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    let mut active = model.into_active_model();
                    active.title = Set(draft.title.clone());
                    active.body = Set(draft.body.clone());
                    active.variables_json = Set(serde_json::to_string(&draft.variables)?);
                    active.revision = Set(expected_revision + 1);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    let response = prompt_template_from_model(updated)?;
                    insert_app_event(
                        transaction,
                        "prompt_template.changed",
                        response.id,
                        response.revision,
                        serde_json::json!({ "deleted": false }),
                        now,
                    )
                    .await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "PATCH",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }

    /// 软删除一份模板。
    ///
    /// # Errors
    ///
    /// 模板不存在、修订冲突、`command_id` 复用于不同请求或数据库操作失败时返回错误。
    pub async fn delete_prompt_template(
        &self,
        template_id: Uuid,
        command_id: Uuid,
        expected_revision: i64,
    ) -> Result<PromptTemplate, StoreError> {
        let target = format!("/api/prompt-templates/{template_id}");
        let payload_hash = payload_hash(&DeleteTemplatePayload { expected_revision })?;
        let command = Command::new(command_id, "DELETE", &target, &payload_hash);
        let target_for_transaction = target.clone();
        let payload_hash_for_transaction = payload_hash.clone();
        command
            .commit(self, Notify::Listeners, move |transaction| {
                Box::pin(async move {
                    let now = utc_now_millis();
                    let model = active_prompt_template(transaction, template_id).await?;
                    if model.revision != expected_revision {
                        return Err(StoreError::PromptTemplateRevisionConflict {
                            expected: expected_revision,
                            actual: model.revision,
                        });
                    }
                    let mut active = model.into_active_model();
                    active.deleted_at = Set(Some(now));
                    active.revision = Set(expected_revision + 1);
                    active.updated_at = Set(now);
                    let updated = active.update(transaction).await?;
                    let response = prompt_template_from_model(updated)?;
                    insert_app_event(
                        transaction,
                        "prompt_template.changed",
                        response.id,
                        response.revision,
                        serde_json::json!({ "deleted": true }),
                        now,
                    )
                    .await?;
                    // 模板进入回收站，正在打开的回收站页面必须跟着刷新。
                    insert_app_event(
                        transaction,
                        "recycle.changed",
                        response.id,
                        response.revision,
                        serde_json::json!({ "kind": "prompt_template", "action": "deleted" }),
                        now,
                    )
                    .await?;
                    insert_receipt(
                        transaction,
                        command_id,
                        "DELETE",
                        &target_for_transaction,
                        &payload_hash_for_transaction,
                        &serde_json::to_string(&response)?,
                        200,
                        now,
                    )
                    .await?;
                    Ok(response)
                })
            })
            .await
    }
}

fn validate_draft(input: NewPromptTemplate) -> Result<NewPromptTemplate, StoreError> {
    input
        .validated()
        .map_err(|error| StoreError::InvalidPromptTemplate(error.to_string()))
}

async fn active_prompt_template<C: ConnectionTrait>(
    connection: &C,
    template_id: Uuid,
) -> Result<prompt_template::Model, StoreError> {
    prompt_template::Entity::find_by_id(db_id(template_id))
        .filter(prompt_template::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::PromptTemplateNotFound)
}

fn prompt_template_from_model(model: prompt_template::Model) -> Result<PromptTemplate, StoreError> {
    let variables: Vec<TemplateVariable> = serde_json::from_str(&model.variables_json)
        .map_err(|error| StoreError::CorruptRecord(format!("模板变量声明不是 JSON：{error}")))?;
    Ok(PromptTemplate {
        id: model.id.0,
        title: model.title,
        body: model.body,
        variables,
        revision: model.revision,
        created_at_ms: model.created_at,
        updated_at_ms: model.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nekowriter_core::{TEMPLATE_VALUE_TYPE_TEXT, render_template};
    use tempfile::TempDir;

    use super::*;

    async fn open() -> (TempDir, Store) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        (data_dir, store)
    }

    fn variable(name: &str) -> TemplateVariable {
        TemplateVariable {
            name: name.to_owned(),
            description: format!("{name} 的说明"),
            value_type: TEMPLATE_VALUE_TYPE_TEXT.to_owned(),
            required: true,
            example: "示例".to_owned(),
        }
    }

    fn draft(title: &str, body: &str) -> NewPromptTemplate {
        NewPromptTemplate {
            title: title.to_owned(),
            body: body.to_owned(),
            variables: vec![variable("tone")],
        }
    }

    #[tokio::test]
    async fn template_crud_round_trips_with_optimistic_revision() {
        let (_data_dir, store) = open().await;
        let created = store
            .create_prompt_template(Uuid::new_v4(), draft("语气", "保持{{tone}}。"))
            .await
            .unwrap();
        assert_eq!(created.revision, 1);
        assert_eq!(created.variables.len(), 1);

        let listed = store.list_prompt_templates().await.unwrap();
        assert_eq!(listed.len(), 1);
        let fetched = store.get_prompt_template(created.id).await.unwrap();
        assert_eq!(fetched.body, "保持{{tone}}。");
        // 渲染必须使用保存的声明，而不是客户端提供的文本。
        let rendered = render_template(
            &fetched.body,
            &fetched.variables,
            &BTreeMap::from([("tone".to_owned(), "克制".to_owned())]),
        )
        .unwrap();
        assert_eq!(rendered, "保持克制。");

        let updated = store
            .update_prompt_template(
                created.id,
                Uuid::new_v4(),
                1,
                draft("语气二", "更{{tone}}。"),
            )
            .await
            .unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.body, "更{{tone}}。");

        // 过期修订被拒绝，不能覆盖较新的内容。
        let error = store
            .update_prompt_template(created.id, Uuid::new_v4(), 1, draft("语气三", "{{tone}}。"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            StoreError::PromptTemplateRevisionConflict {
                expected: 1,
                actual: 2
            }
        ));

        store
            .delete_prompt_template(created.id, Uuid::new_v4(), 2)
            .await
            .unwrap();
        assert_eq!(
            store.list_prompt_templates().await.unwrap(),
            [] as [nekowriter_core::PromptTemplate; 0]
        );
        assert!(matches!(
            store.get_prompt_template(created.id).await.unwrap_err(),
            StoreError::PromptTemplateNotFound
        ));
    }

    #[tokio::test]
    async fn template_writes_reject_undeclared_variables_and_missing_placeholders() {
        let (_data_dir, store) = open().await;
        // 正文引用了未声明的变量。
        let error = store
            .create_prompt_template(
                Uuid::new_v4(),
                NewPromptTemplate {
                    title: "非法".to_owned(),
                    body: "{{unknown}}".to_owned(),
                    variables: Vec::new(),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::InvalidPromptTemplate(_)));

        // 声明的必需变量没有出现在正文中。
        let error = store
            .create_prompt_template(
                Uuid::new_v4(),
                NewPromptTemplate {
                    title: "非法".to_owned(),
                    body: "没有占位符".to_owned(),
                    variables: vec![variable("tone")],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, StoreError::InvalidPromptTemplate(_)));

        // 被拒绝的写入没有留下任何记录。
        assert_eq!(
            store.list_prompt_templates().await.unwrap(),
            [] as [nekowriter_core::PromptTemplate; 0]
        );
    }
}
