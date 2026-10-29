//! Per-system canon totals for the Book settings switches page (spec/06 §10, D-150).
//!
//! Every scope and every status is counted: these numbers describe the rows that stay in the
//! database when a system is switched off, so filtering by visibility or status would understate
//! what the author keeps.

use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::api::dto::goals::{
    CanonCountsDto, CanonEntityCountsDto, CanonForeshadowingCountsDto, CanonGoalCountsDto,
    CanonMemoryCountsDto, CanonTimelineCountsDto,
};
use crate::db::entities::{
    appearances, chapters, entities, foreshadowing, goal_nodes, memory_notes, relations,
    timeline_events,
};
use crate::error::AppError;

/// Counts the rows of every canon system for one book.
pub async fn counts(conn: &impl ConnectionTrait, book_id: i32) -> Result<CanonCountsDto, AppError> {
    Ok(CanonCountsDto {
        entities: CanonEntityCountsDto {
            entities: entities::Entity::find()
                .filter(entities::COLUMN.book_id.eq(book_id))
                .count(conn)
                .await? as i32,
            relations: relations::Entity::find()
                .filter(relations::COLUMN.book_id.eq(book_id))
                .count(conn)
                .await? as i32,
            appearances: {
                let rows = appearances::Entity::find()
                    .find_also_related(chapters::Entity)
                    .all(conn)
                    .await?;
                rows.into_iter()
                    .filter(|(_, chapter)| {
                        chapter
                            .as_ref()
                            .is_some_and(|chapter| chapter.book_id == book_id)
                    })
                    .count() as i32
            },
        },
        timeline: CanonTimelineCountsDto {
            events: timeline_events::Entity::find()
                .filter(timeline_events::COLUMN.book_id.eq(book_id))
                .count(conn)
                .await? as i32,
        },
        memory: CanonMemoryCountsDto {
            notes: memory_notes::Entity::find()
                .filter(memory_notes::COLUMN.book_id.eq(book_id))
                .count(conn)
                .await? as i32,
            summaries: chapters::Entity::find()
                .filter(chapters::COLUMN.book_id.eq(book_id))
                .filter(chapters::COLUMN.summary.ne(""))
                .count(conn)
                .await? as i32,
        },
        foreshadowing: CanonForeshadowingCountsDto {
            items: foreshadowing::Entity::find()
                .filter(foreshadowing::COLUMN.book_id.eq(book_id))
                .count(conn)
                .await? as i32,
        },
        goals: CanonGoalCountsDto {
            nodes: goal_nodes::Entity::find()
                .filter(goal_nodes::COLUMN.book_id.eq(book_id))
                .count(conn)
                .await? as i32,
        },
    })
}
