//! User prompt templates (`docs/product-spec.md` §13.2).

use nekowriter_core::{
    conflict::Resource,
    event::Event,
    ids::{CommandId, PromptTemplateId},
    prompt_template::{TemplateContent, TemplateVariable},
    time::Timestamp,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    Store, StoreError,
    command::{Begun, CommandTx},
    entity::prompt_template,
    projects::check_revision,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptTemplateView {
    pub id: PromptTemplateId,
    pub title: String,
    pub body: String,
    pub variables: Vec<TemplateVariable>,
    pub revision: i64,
    pub updated_at: Timestamp,
}

impl From<prompt_template::Model> for PromptTemplateView {
    fn from(template: prompt_template::Model) -> Self {
        Self {
            id: template.id,
            title: template.title.as_str().to_owned(),
            body: template.body,
            variables: template.variables.0,
            revision: template.revision,
            updated_at: template.updated_at,
        }
    }
}

impl Store {
    /// The templates that are not deleted, by title.
    pub async fn list_templates(&self) -> Result<Vec<PromptTemplateView>, StoreError> {
        Ok(prompt_template::Entity::find()
            .filter(prompt_template::Column::DeletedAt.is_null())
            .order_by_asc(prompt_template::Column::Title)
            .all(&self.db)
            .await?
            .into_iter()
            .map(PromptTemplateView::from)
            .collect())
    }

    pub async fn create_template(
        &self,
        command: CommandId,
        content: TemplateContent,
    ) -> Result<PromptTemplateView, StoreError> {
        let mut tx = match self
            .begin_command(command, "template.create", &content)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let now = tx.now();
        let template = prompt_template::ActiveModel {
            id: Set(PromptTemplateId::generate()),
            title: Set(content.title),
            body: Set(content.body.as_str().to_owned()),
            variables: Set(content.variables),
            revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(tx.db())
        .await?;
        tx.emit(Event::TemplatesChanged);
        tx.commit(template.into()).await
    }

    pub async fn update_template(
        &self,
        command: CommandId,
        id: PromptTemplateId,
        expected_revision: i64,
        content: TemplateContent,
    ) -> Result<PromptTemplateView, StoreError> {
        let input = (id, expected_revision, &content);
        let tx = match self
            .begin_command(command, "template.update", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let template = live_template(tx.db(), id).await?;
        check_revision(Resource::Template, expected_revision, template.revision)?;
        change(tx, template, |model, _| {
            model.title = Set(content.title);
            model.body = Set(content.body.as_str().to_owned());
            model.variables = Set(content.variables);
        })
        .await
    }

    pub async fn delete_template(
        &self,
        command: CommandId,
        id: PromptTemplateId,
        expected_revision: i64,
    ) -> Result<PromptTemplateView, StoreError> {
        let input = (id, expected_revision);
        let tx = match self
            .begin_command(command, "template.delete", &input)
            .await?
        {
            Begun::Replayed(view) => return Ok(view),
            Begun::Fresh(tx) => tx,
        };
        let template = live_template(tx.db(), id).await?;
        check_revision(Resource::Template, expected_revision, template.revision)?;
        change(tx, template, |model, now| model.deleted_at = Set(Some(now))).await
    }
}

async fn live_template<C: ConnectionTrait>(
    db: &C,
    id: PromptTemplateId,
) -> Result<prompt_template::Model, StoreError> {
    prompt_template::Entity::find_by_id(id)
        .filter(prompt_template::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or(StoreError::NotFound(Resource::Template))
}

async fn change(
    mut tx: CommandTx,
    template: prompt_template::Model,
    apply: impl FnOnce(&mut prompt_template::ActiveModel, Timestamp),
) -> Result<PromptTemplateView, StoreError> {
    let now = tx.now();
    let revision = template.revision + 1;
    let mut model = template.into_active_model();
    apply(&mut model, now);
    model.revision = Set(revision);
    model.updated_at = Set(now);
    let template = model.update(tx.db()).await?;
    tx.emit(Event::TemplatesChanged);
    tx.commit(template.into()).await
}
