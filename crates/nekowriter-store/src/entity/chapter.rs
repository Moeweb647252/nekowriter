use sea_orm::entity::prelude::*;

/// 已审核通过的粗纲章节计划所物化的章节。
///
/// `plan_item_id` 指向粗纲修订中的类型化计划项；正文开始后该对应关系保持稳定。
/// 有效章节的顺序唯一性由 schema.rs 中的 partial index 约束，以便软删除的番外
/// 不会占住原顺序。
#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "chapters")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub version_id: TextUuid,
    pub kind: String,
    pub plan_item_id: Option<String>,
    pub order_index: i64,
    pub title: String,
    pub status: String,
    pub approved_revision_id: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[sea_orm(belongs_to, from = "version_id", to = "id")]
    pub version: BelongsTo<super::novel_version::Entity>,
    #[sea_orm(
        belongs_to,
        relation_enum = "ApprovedRevision",
        from = "approved_revision_id",
        to = "id"
    )]
    pub approved_revision: BelongsTo<Option<super::document_revision::Entity>>,
    #[sea_orm(has_many)]
    pub documents: HasMany<super::document::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
