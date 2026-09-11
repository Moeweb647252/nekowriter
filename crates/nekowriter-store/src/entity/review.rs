use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "reviews")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub version_id: TextUuid,
    pub batch_id: Option<String>,
    #[sea_orm(column_name = "type")]
    pub review_type: String,
    pub target_revision_id: Option<String>,
    pub target_tool_call_id: Option<String>,
    pub dependencies_json: String,
    pub status: String,
    pub attempt_id: Option<String>,
    pub decision_source: Option<String>,
    pub source_review_id: Option<String>,
    pub passed: Option<bool>,
    pub confidence: Option<f64>,
    pub issues_json: Option<String>,
    pub user_input_json: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "TargetRevision",
        from = "target_revision_id",
        to = "id"
    )]
    pub target_revision: BelongsTo<Option<super::document_revision::Entity>>,
    #[sea_orm(
        self_ref,
        relation_enum = "SourceReview",
        from = "source_review_id",
        to = "id"
    )]
    pub source_review: BelongsTo<Option<Entity>>,
}

impl ActiveModelBehavior for ActiveModel {}
