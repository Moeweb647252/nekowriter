//! Timeline tree rules (spec/06 §1.4, D-170).
//!
//! The tree groups story entries; `position` stays book-wide chronology. These helpers own the
//! parent/scope/consistency rules shared by the service, tools, delete and revert paths.

use std::collections::{BTreeSet, VecDeque};

use sea_orm::{ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::db::entities::timeline_events;
use crate::db::json::{TimelineKind, TimelineStatus};

use super::{CanonError, CanonResult, CanonWrite};

/// The book's contentless root.
pub async fn root_of(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> CanonResult<timeline_events::Model> {
    timeline_events::Entity::find()
        .filter(timeline_events::COLUMN.book_id.eq(book_id))
        .filter(timeline_events::COLUMN.kind.eq(TimelineKind::Root.as_str()))
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::internal(format!("book {book_id} has no timeline root")))
}

/// Every descendant id of `entry_id`, including the entry itself.
pub async fn subtree_ids(conn: &impl ConnectionTrait, entry_id: i32) -> CanonResult<BTreeSet<i32>> {
    let mut seen = BTreeSet::from([entry_id]);
    let mut frontier = vec![entry_id];
    while !frontier.is_empty() {
        let children = timeline_events::Entity::find()
            .filter(timeline_events::COLUMN.parent_id.is_in(frontier.clone()))
            .all(conn)
            .await?;
        frontier.clear();
        for child in children {
            if seen.insert(child.id) {
                frontier.push(child.id);
            }
        }
    }
    Ok(seen)
}

/// True when `candidate` is `entry_id` or one of its descendants.
pub async fn in_subtree(
    conn: &impl ConnectionTrait,
    entry_id: i32,
    candidate: i32,
) -> CanonResult<bool> {
    Ok(subtree_ids(conn, entry_id).await?.contains(&candidate))
}

/// Resolves and validates a parent for a new or moved entry (T6). `None` means the root.
pub async fn parent_for(
    conn: &impl ConnectionTrait,
    write: &CanonWrite,
    book_id: i32,
    child_track: Option<i32>,
    parent: Option<i32>,
    moving: Option<i32>,
) -> CanonResult<i32> {
    let root = root_of(conn, book_id).await?;
    let Some(parent_id) = parent else {
        return Ok(root.id);
    };
    let row = timeline_events::Entity::find_by_id(parent_id)
        .one(conn)
        .await?
        .filter(|row| row.book_id == book_id && row.kind == TimelineKind::Entry)
        .ok_or_else(|| {
            CanonError::field(
                "parent",
                format!("timeline event #{parent_id} was not found"),
            )
        })?;
    if row.status == TimelineStatus::Abandoned {
        return Err(CanonError::field(
            "parent",
            "an abandoned entry cannot receive children",
        ));
    }
    if let Some(moving) = moving
        && in_subtree(conn, moving, parent_id).await?
    {
        return Err(CanonError::field(
            "parent",
            "an entry cannot be moved under itself or its descendants",
        ));
    }
    // A private parent only accepts private children of the same track; a shared parent accepts
    // either (a shared entry is never placed under a private one).
    if let Some(parent_track) = row.track_id
        && child_track != Some(parent_track)
    {
        return Err(CanonError::field(
            "parent",
            "the parent is in another track",
        ));
    }
    let _ = write;
    Ok(parent_id)
}

/// The position for a child with no explicit neighbour: right after its parent's subtree.
pub async fn append_position(
    conn: &impl ConnectionTrait,
    book_id: i32,
    parent_id: i32,
) -> CanonResult<f64> {
    let ids = subtree_ids(conn, parent_id).await?;
    let last = timeline_events::Entity::find()
        .filter(
            timeline_events::COLUMN
                .id
                .is_in(ids.into_iter().collect::<Vec<_>>()),
        )
        .filter(
            timeline_events::COLUMN
                .kind
                .eq(TimelineKind::Entry.as_str()),
        )
        .order_by_desc(timeline_events::COLUMN.position)
        .order_by_desc(timeline_events::COLUMN.id)
        .one(conn)
        .await?;
    let after = last.map(|row| row.id);
    super::timeline::position_for(conn, book_id, after, None, None).await
}

/// T5: a planned entry may not sit under an `occurred` same-scope ancestor.
pub async fn check_planned_ancestors(
    conn: &impl ConnectionTrait,
    parent_id: i32,
    child_track: Option<i32>,
) -> CanonResult<()> {
    let mut cursor = Some(parent_id);
    while let Some(id) = cursor {
        let Some(row) = timeline_events::Entity::find_by_id(id).one(conn).await? else {
            break;
        };
        if row.kind == TimelineKind::Root {
            break;
        }
        if row.status == TimelineStatus::Occurred && row.track_id == child_track {
            return Err(CanonError::field(
                "status",
                format!(
                    "a planned entry cannot descend from occurred entry #{} {}",
                    row.id, row.title
                ),
            ));
        }
        cursor = row.parent_id;
    }
    Ok(())
}

/// The descendants of `entry_id` in its **own scope** (any depth), in id order.
///
/// A shared entry (no track) owns only shared descendants; a private entry owns only its track's
/// descendants. A private child under a shared entry lives in another scope, so T5 and T7 never
/// look at it from the shared entry (D-170 T5/T7, M22-06).
async fn scoped_descendants(
    conn: &impl ConnectionTrait,
    entry_id: i32,
    status: TimelineStatus,
) -> CanonResult<Vec<timeline_events::Model>> {
    let Some(entry) = timeline_events::Entity::find_by_id(entry_id)
        .one(conn)
        .await?
    else {
        return Ok(Vec::new());
    };
    let ids = subtree_ids(conn, entry_id).await?;
    let mut query = timeline_events::Entity::find()
        .filter(
            timeline_events::COLUMN
                .id
                .is_in(ids.into_iter().collect::<Vec<_>>()),
        )
        .filter(timeline_events::COLUMN.status.eq(status.as_str()));
    query = match entry.track_id {
        Some(track_id) => query.filter(timeline_events::COLUMN.track_id.eq(track_id)),
        None => query.filter(timeline_events::COLUMN.track_id.is_null()),
    };
    let mut rows: Vec<timeline_events::Model> = query.all(conn).await?;
    rows.retain(|row| row.id != entry_id);
    rows.sort_by_key(|row| row.id);
    Ok(rows)
}

/// The planned descendants of `entry_id` in its own scope (any depth), in id order.
pub async fn planned_descendants(
    conn: &impl ConnectionTrait,
    entry_id: i32,
) -> CanonResult<Vec<timeline_events::Model>> {
    scoped_descendants(conn, entry_id, TimelineStatus::Planned).await
}

/// The occurred descendants of `entry_id` in its own scope (any depth), in id order.
pub async fn occurred_descendants(
    conn: &impl ConnectionTrait,
    entry_id: i32,
) -> CanonResult<Vec<timeline_events::Model>> {
    scoped_descendants(conn, entry_id, TimelineStatus::Occurred).await
}

/// A breadth-first list of the entries in `book_id` with their parents, used by `query_timeline`
/// to prune abandoned subtrees and show visible orphans at the top level.
pub async fn tree_rows(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> CanonResult<Vec<timeline_events::Model>> {
    Ok(timeline_events::Entity::find()
        .filter(timeline_events::COLUMN.book_id.eq(book_id))
        .filter(
            timeline_events::COLUMN
                .kind
                .eq(TimelineKind::Entry.as_str()),
        )
        .order_by_asc(timeline_events::COLUMN.position)
        .order_by_asc(timeline_events::COLUMN.id)
        .all(conn)
        .await?)
}

/// The raw parent chain of `entry_id` (nearest first), stopping at the root.
pub fn ancestor_chain(
    rows: &std::collections::BTreeMap<i32, timeline_events::Model>,
    entry_id: i32,
) -> Vec<&timeline_events::Model> {
    let mut chain = Vec::new();
    let mut cursor = rows.get(&entry_id).and_then(|row| row.parent_id);
    while let Some(id) = cursor {
        let Some(row) = rows.get(&id) else {
            break;
        };
        chain.push(row);
        cursor = row.parent_id;
    }
    chain
}

/// A simple breadth-first traversal helper for tests and callers that need the nodes of one
/// subtree in position order.
pub fn order_by_position(
    mut rows: Vec<timeline_events::Model>,
) -> VecDeque<timeline_events::Model> {
    rows.sort_by(|a, b| {
        a.position
            .partial_cmp(&b.position)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.id.cmp(&b.id))
    });
    rows.into()
}
