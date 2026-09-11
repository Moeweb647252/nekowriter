use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "novel_versions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub project_id: TextUuid,
    pub label: String,
    pub novel_title: String,
    pub source_version_id: Option<String>,
    pub body_completed_at: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[sea_orm(belongs_to, from = "project_id", to = "id")]
    pub project: BelongsTo<super::project::Entity>,
    #[sea_orm(
        self_ref,
        relation_enum = "SourceVersion",
        from = "source_version_id",
        to = "id"
    )]
    pub source_version: BelongsTo<Option<Entity>>,
    #[sea_orm(has_many)]
    pub documents: HasMany<super::document::Entity>,
    #[sea_orm(has_one)]
    pub workflow: HasOne<super::workflow_instance::Entity>,
    #[sea_orm(has_many)]
    pub runs: HasMany<super::run::Entity>,
    #[sea_orm(has_many)]
    pub reviews: HasMany<super::review::Entity>,
    #[sea_orm(has_many)]
    pub events: HasMany<super::event::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
