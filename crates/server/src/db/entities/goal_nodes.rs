//! `goal_nodes` entity (spec/02 §2, spec/06 §11).

use sea_orm::entity::prelude::*;

use crate::db::json::{GoalCondition, GoalLevel, GoalStatus, JsonText};

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "goal_nodes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub level: GoalLevel,
    pub parent_id: Option<i32>,
    pub title: String,
    pub purpose: String,
    pub status: GoalStatus,
    pub conditions: JsonText<Vec<GoalCondition>>,
    pub chapter_from: Option<i32>,
    pub chapter_to: Option<i32>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    #[sea_orm(belongs_to, from = "book_id", to = "id")]
    pub book: BelongsTo<super::books::Entity>,
    #[sea_orm(belongs_to, from = "track_id", to = "id")]
    pub track: BelongsTo<Option<super::tracks::Entity>>,
    #[sea_orm(
        self_ref,
        relation_enum = "Parent",
        relation_reverse = "Children",
        from = "parent_id",
        to = "id"
    )]
    pub parent: BelongsTo<Option<Entity>>,
    #[sea_orm(self_ref, relation_enum = "Children", relation_reverse = "Parent")]
    pub children: HasMany<Entity>,
    #[sea_orm(has_many, via = "goal_node_entities")]
    pub entities: HasMany<super::entities::Entity>,
    #[sea_orm(has_many, via = "goal_node_events")]
    pub timeline_events: HasMany<super::timeline_events::Entity>,
    #[sea_orm(has_many, via = "goal_node_foreshadowing")]
    pub foreshadowing: HasMany<super::foreshadowing::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
