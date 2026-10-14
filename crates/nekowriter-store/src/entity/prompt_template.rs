use nekowriter_core::{
    ids::PromptTemplateId,
    prompt_template::{TemplateTitle, TemplateVariables},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// A user prompt template (§13.2). Deleting it is soft.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "prompt_templates")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: PromptTemplateId,
    pub title: TemplateTitle,
    #[sea_orm(column_type = "Text")]
    pub body: String,
    pub variables: TemplateVariables,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub deleted_at: Option<Timestamp>,
}

impl ActiveModelBehavior for ActiveModel {}
