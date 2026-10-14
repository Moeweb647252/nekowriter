use nekowriter_core::{
    ids::{ProjectId, VersionId},
    model::{ModelProfile, ThinkingLevel},
    project::{AuxiliaryBudget, ProjectName, WritingLanguage},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "projects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: ProjectId,
    pub name: ProjectName,
    pub writing_language: WritingLanguage,
    /// The project's own model; `None` uses the system default.
    pub model: Option<ModelProfile>,
    pub thinking_level: ThinkingLevel,
    /// Charged auxiliary tool calls each budget scope allows (§12.1).
    pub auxiliary_budget: AuxiliaryBudget,
    /// The version the workspace shows. It always belongs to this project; there is no
    /// foreign key because versions also reference their project.
    pub selected_version_id: VersionId,
    pub revision: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    #[sea_orm(indexed)]
    pub deleted_at: Option<Timestamp>,
    #[sea_orm(has_many)]
    pub versions: HasMany<super::version::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
