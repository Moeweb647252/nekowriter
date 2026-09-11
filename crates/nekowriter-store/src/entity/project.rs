use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, Eq, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "projects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: TextUuid,
    pub name: String,
    // 这是当前选中稿本的领域标识。项目与其首个稿本在同一事务中创建，二者都是
    // 非空字段；SeaORM schema 不支持为该必填环形关系生成延迟外键，因此归属一致性
    // 由仓储事务校验，真正的 project_id 外键由 novel_versions 维护。
    pub selected_version_id: TextUuid,
    pub settings_json: String,
    pub settings_revision: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    #[sea_orm(has_many)]
    pub versions: HasMany<super::novel_version::Entity>,
    #[sea_orm(has_many)]
    pub runs: HasMany<super::run::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
