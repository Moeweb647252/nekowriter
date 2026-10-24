//! Canon services (spec/06 §1–§3, §9; PLAN M5.1).
//!
//! Canon is the structured state of the story: entities, relations, appearances, timeline events,
//! memory notes, foreshadowing and chapter summaries. Every write in this module
//!
//! 1. checks the caller's visibility (spec/06 §2): an isolated side track may read shared canon
//!    but its writes go to private rows, and it may never modify a shared row (D-014, D-036);
//! 2. writes one `change_log` row **in the same transaction** with the full typed `Snapshot`
//!    before/after (spec/02 §4, spec/06 §3);
//! 3. returns the `canon.changed` event, which the caller publishes after the commit.
//!
//! `note` only ever holds written text (the author's change note, the agent's note, or the
//! language-neutral `#B → #A` of a merge): model-facing renderings include non-empty notes only,
//! and no UI language reaches a model (spec/06 §3, D-032). Labels are rendered by the prompt
//! registry; nothing here writes model-facing text.

pub mod appearance;
pub mod entity;
pub mod foreshadow;
pub mod note;
pub mod relation;
pub mod revert;
pub mod timeline;

use std::collections::{HashMap, HashSet};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    Set, Statement,
};

use crate::db::entities::{appearances, chapters, entities, relations, tracks};
use crate::db::json::{
    AppearanceRole, AppearanceSnapshot, ChangeActor, ChangeOp, ChangeTargetType, EntityKind,
    EntityStatus, ForeshadowingStatus, NoteKind, NoteStatus, RelationStatus, Snapshot,
};
use crate::error::AppError;
use crate::events::ServerEvent;

pub(crate) use super::change_log as change_log_service;
use super::{field_error, now};

// ───────────────────────────── errors ─────────────────────────────

/// A canon write failure.
///
/// Almost every failure is an ordinary [`AppError`]. The exception is the isolated-track write
/// rule of spec/06 §2: `error.isolated_readonly` is a **tool** error code (spec/09 §2), and the
/// agent tools must be able to render it as such rather than as a generic validation failure, so
/// it travels as its own variant. The author's API never raises it (the UI edits every row).
#[derive(Debug, Clone)]
pub enum CanonError {
    App(AppError),
    /// An isolated side track tried to modify a shared row (spec/06 §2, D-014).
    IsolatedReadonly {
        target: String,
    },
}

impl CanonError {
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        CanonError::App(AppError::not_found(what))
    }

    pub fn validation(message: impl Into<String>) -> Self {
        CanonError::App(AppError::validation(message))
    }

    pub fn field(field: &str, message: impl Into<String>) -> Self {
        CanonError::App(field_error(field, message))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        CanonError::App(AppError::internal(message))
    }

    pub fn isolated(target: impl Into<String>) -> Self {
        CanonError::IsolatedReadonly {
            target: target.into(),
        }
    }

    /// The `AppError` an HTTP handler answers with.
    pub fn into_app_error(self) -> AppError {
        match self {
            CanonError::App(error) => error,
            // The author's API has no isolated-track session, so this is unreachable there; a 400
            // is still a truthful answer if a future caller reaches it.
            CanonError::IsolatedReadonly { target } => AppError::validation(format!(
                "{target} belongs to shared canon and cannot be modified here"
            )),
        }
    }
}

impl From<AppError> for CanonError {
    fn from(error: AppError) -> Self {
        CanonError::App(error)
    }
}

impl From<sea_orm::DbErr> for CanonError {
    fn from(error: sea_orm::DbErr) -> Self {
        CanonError::App(AppError::from(error))
    }
}

impl From<CanonError> for AppError {
    fn from(error: CanonError) -> Self {
        error.into_app_error()
    }
}

impl std::fmt::Display for CanonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanonError::App(error) => write!(f, "{error}"),
            CanonError::IsolatedReadonly { target } => {
                write!(f, "isolated_readonly: {target}")
            }
        }
    }
}

impl std::error::Error for CanonError {}

/// The result every canon service returns.
pub type CanonResult<T> = Result<T, CanonError>;

// ───────────────────────────── visibility (spec/06 §2) ─────────────────────────────

/// Which canon rows a caller may see **and** write (spec/06 §2).
///
/// `private_track: Some(T)` is an isolated side track: it reads `track_id IS NULL OR track_id = T`
/// and writes only `track_id = T` rows. `None` is the main story, a canon-mode side story and the
/// author's UI: shared canon only, fully writable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub book_id: i32,
    pub private_track: Option<i32>,
}

impl Visibility {
    /// Shared canon (main track, canon-mode side track, or the author's UI).
    pub fn shared(book_id: i32) -> Self {
        Self {
            book_id,
            private_track: None,
        }
    }

    /// An isolated side track's view.
    pub fn isolated(book_id: i32, track_id: i32) -> Self {
        Self {
            book_id,
            private_track: Some(track_id),
        }
    }

    /// The visibility of a runtime session's track (the runner's own `Visibility`).
    pub fn of_session(book_id: i32, visibility: crate::agent::tools::Visibility) -> Self {
        match visibility {
            crate::agent::tools::Visibility::Isolated { track_id } => {
                Self::isolated(book_id, track_id)
            }
            crate::agent::tools::Visibility::Shared => Self::shared(book_id),
        }
    }

    /// `track_id IS NULL OR track_id = T` (D-014). Every canon query that feeds tools, memory or
    /// search goes through this helper, and tests cover it.
    pub fn visible_condition<C: ColumnTrait>(&self, column: C) -> Condition {
        match self.private_track {
            Some(track_id) => Condition::any()
                .add(column.is_null())
                .add(column.eq(track_id)),
            None => Condition::all().add(column.is_null()),
        }
    }

    /// The `track_id` a new row gets (spec/06 §2 "Writes go to").
    pub fn new_row_track(&self) -> Option<i32> {
        self.private_track
    }

    /// True when this visibility may modify a row with that `track_id`.
    pub fn may_modify(&self, row_track: Option<i32>) -> bool {
        match (self.private_track, row_track) {
            (None, None) => true,
            (None, Some(_)) => false,
            (Some(track_id), Some(row_track)) => track_id == row_track,
            // A shared row is read-only for an isolated track (spec/06 §2).
            (Some(_), None) => false,
        }
    }

    /// True when this visibility can see a row with that `track_id`.
    pub fn sees(&self, row_track: Option<i32>) -> bool {
        match (self.private_track, row_track) {
            (_, None) => true,
            (Some(track_id), Some(row_track)) => track_id == row_track,
            (None, Some(_)) => false,
        }
    }

    /// Rejects a write to a shared row from an isolated track (spec/06 §2, D-014).
    pub fn ensure_writable(&self, row_track: Option<i32>, target: &str) -> CanonResult<()> {
        if self.may_modify(row_track) {
            Ok(())
        } else {
            Err(CanonError::isolated(target))
        }
    }

    /// Rejects a row this visibility cannot see, as `not_found`.
    pub fn ensure_visible(&self, row_track: Option<i32>) -> CanonResult<()> {
        if self.sees(row_track) {
            Ok(())
        } else {
            Err(CanonError::not_found("canon row"))
        }
    }
}

/// Who is writing canon, and which rows they may change (spec/06 §2, §3).
#[derive(Debug, Clone)]
pub struct CanonWrite {
    /// `None` for the author's UI: every row of the book, no track restriction.
    pub visibility: Option<Visibility>,
    pub actor: ChangeActor,
    /// The agent session that performed the write, when there is one.
    pub session_id: Option<i32>,
    /// The chapter the change belongs to (the agent's session chapter).
    pub chapter_id: Option<i32>,
    /// Set while a revert performs the inverse operation: the change row the operation writes is
    /// the revert itself (spec/06 §3, D-079), so `record_change` stamps it with `reverts_id` and
    /// the `system` actor instead of writing a second row.
    pub revert: Option<RevertRecord>,
}

/// A revert in progress.
#[derive(Debug, Clone, Copy)]
pub struct RevertRecord {
    /// The change being reverted.
    pub of_change: i32,
}

impl CanonWrite {
    /// The author editing canon in the UI.
    pub fn author() -> Self {
        Self {
            visibility: None,
            actor: ChangeActor::User,
            session_id: None,
            chapter_id: None,
            revert: None,
        }
    }

    /// An agent writing canon through the tools of its session.
    pub fn agent(visibility: Visibility, session_id: i32, chapter_id: Option<i32>) -> Self {
        Self {
            visibility: Some(visibility),
            actor: ChangeActor::Agent,
            session_id: Some(session_id),
            chapter_id,
            revert: None,
        }
    }

    /// Marks this write as the inverse of `of_change`: the row it records is the revert.
    ///
    /// A revert is a system change and its note is empty (spec/06 §3: "reverts, reorders, and
    /// other system changes: empty"), so unlike an ordinary write it carries no note.
    pub fn as_revert(mut self, of_change: i32) -> Self {
        self.revert = Some(RevertRecord { of_change });
        self
    }

    pub fn with_chapter(mut self, chapter_id: Option<i32>) -> Self {
        self.chapter_id = chapter_id;
        self
    }

    /// The `track_id` a new row gets.
    pub fn new_row_track(&self) -> Option<i32> {
        self.visibility
            .and_then(|visibility| visibility.new_row_track())
    }

    /// The isolated track a new row belongs to.
    pub fn isolated_track(&self) -> Option<i32> {
        self.visibility
            .and_then(|visibility| visibility.private_track)
    }

    /// The visibility of a book for the author's API (shared canon; a UI track filter only reads).
    pub fn shared(book_id: i32) -> Self {
        Self::author().with_visibility(Visibility::shared(book_id))
    }

    /// Restricts this writer to one visibility.
    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = Some(visibility);
        self
    }

    /// Rejects a write to a shared row from an isolated track (spec/06 §2, D-014).
    pub fn ensure_writable(&self, row_track: Option<i32>, target: &str) -> CanonResult<()> {
        match self.visibility {
            None => Ok(()),
            Some(visibility) if visibility.may_modify(row_track) => Ok(()),
            Some(_) => Err(CanonError::isolated(target)),
        }
    }

    /// Rejects a row this session cannot see, as `not_found` (it does not exist for it).
    pub fn ensure_visible(&self, row_track: Option<i32>) -> CanonResult<()> {
        match self.visibility {
            None => Ok(()),
            Some(visibility) if visibility.sees(row_track) => Ok(()),
            Some(_) => Err(CanonError::not_found("canon row")),
        }
    }

    /// The visibility condition for a query, or `None` for the author's unfiltered view.
    pub fn condition<C: ColumnTrait>(&self, column: C) -> Option<Condition> {
        self.visibility
            .map(|visibility| visibility.visible_condition(column))
    }
}

// ───────────────────────────── labels ─────────────────────────────

/// The prompt set of a book, for callers that only have a connection.
pub async fn prompts_for(
    state: &crate::state::AppState,
    book_id: Option<i32>,
) -> CanonResult<std::sync::Arc<crate::prompts::PromptSet>> {
    state
        .prompts()
        .for_book(state.db(), book_id)
        .await
        .map_err(CanonError::from)
}

/// A chapter's full label, rendered by `shared.chapter_label` (spec/05 §1.5).
pub async fn chapter_label(
    conn: &impl ConnectionTrait,
    prompts: &crate::prompts::PromptSet,
    chapter_id: Option<i32>,
) -> CanonResult<String> {
    let Some(chapter_id) = chapter_id else {
        return Ok(String::new());
    };
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(String::new());
    };
    let Some(track) = tracks::Entity::find_by_id(chapter.track_id)
        .one(conn)
        .await?
    else {
        return Ok(String::new());
    };
    Ok(prompts.chapter_label(
        &track.kind,
        &track.title,
        i64::from(chapter.ord),
        &chapter.title,
        true,
    ))
}

/// A chapter's label without the title, for lines that append `《title》` themselves.
pub async fn chapter_label_bare(
    conn: &impl ConnectionTrait,
    prompts: &crate::prompts::PromptSet,
    chapter_id: i32,
) -> CanonResult<String> {
    let Some(chapter) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return Ok(String::new());
    };
    let Some(track) = tracks::Entity::find_by_id(chapter.track_id)
        .one(conn)
        .await?
    else {
        return Ok(String::new());
    };
    Ok(prompts.chapter_label(
        &track.kind,
        &track.title,
        i64::from(chapter.ord),
        &chapter.title,
        false,
    ))
}

// ───────────────────────────── chapter order (D-059) ─────────────────────────────

/// Where a chapter sits, for the appearance bounds and the memory windows (spec/06 §1.1, §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChapterOrder {
    pub track_id: i32,
    /// The track's `ord`: the main track is 0 (spec/02 §2).
    pub track_ord: i32,
    pub chapter_ord: i32,
    /// True for the main track and canon-mode side tracks: the chapters shared canon counts
    /// (spec/06 §1.1, D-036).
    pub shared_canon: bool,
}

impl ChapterOrder {
    /// The `(track ord, chapter ord)` ordering key of spec/06 §1.1.
    pub fn key(&self) -> (i32, i32) {
        (self.track_ord, self.chapter_ord)
    }
}

/// Every chapter of a book with its ordering key.
pub async fn chapter_orders(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> CanonResult<HashMap<i32, ChapterOrder>> {
    let tracks = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    let track_info: HashMap<i32, (i32, bool)> = tracks
        .iter()
        .map(|track| {
            (
                track.id,
                (
                    track.ord,
                    track.kind == "main" || track.canon_mode == "canon",
                ),
            )
        })
        .collect();
    let mut orders = HashMap::new();
    let rows = chapters::Entity::find()
        .filter(chapters::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    for chapter in rows {
        if let Some((track_ord, shared_canon)) = track_info.get(&chapter.track_id) {
            orders.insert(
                chapter.id,
                ChapterOrder {
                    track_id: chapter.track_id,
                    track_ord: *track_ord,
                    chapter_ord: chapter.ord,
                    shared_canon: *shared_canon,
                },
            );
        }
    }
    Ok(orders)
}

// ───────────────────────────── snapshots (spec/02 §4) ─────────────────────────────

pub fn entity_snapshot(row: &entities::Model) -> CanonResult<Snapshot> {
    Ok(Snapshot::Entity {
        kind: super::enum_of::<EntityKind>("entities.kind", &row.kind)?,
        name: row.name.clone(),
        aliases: aliases_of(row)?,
        summary: row.summary.clone(),
        description: row.description.clone(),
        state: row.state.clone(),
        attributes: parse_json("entities.attributes", &row.attributes)?,
        importance: row.importance,
        status: super::enum_of::<EntityStatus>("entities.status", &row.status)?,
        is_protagonist: row.is_protagonist != 0,
        track_id: row.track_id,
    })
}

pub fn relation_snapshot(row: &relations::Model) -> CanonResult<Snapshot> {
    Ok(Snapshot::Relation {
        source_id: row.source_id,
        target_id: row.target_id,
        kind: row.kind.clone(),
        inverse_kind: row.inverse_kind.clone(),
        description: row.description.clone(),
        status: super::enum_of::<RelationStatus>("relations.status", &row.status)?,
        since_chapter_id: row.since_chapter_id,
        until_chapter_id: row.until_chapter_id,
        track_id: row.track_id,
    })
}

pub fn timeline_snapshot(
    row: &crate::db::entities::timeline_events::Model,
    entity_ids: Vec<i32>,
) -> Snapshot {
    Snapshot::TimelineEvent {
        position: row.position,
        story_time: row.story_time.clone(),
        title: row.title.clone(),
        description: row.description.clone(),
        chapter_id: row.chapter_id,
        importance: row.importance,
        entity_ids,
        track_id: row.track_id,
    }
}

pub fn note_snapshot(
    row: &crate::db::entities::memory_notes::Model,
    entity_ids: Vec<i32>,
) -> CanonResult<Snapshot> {
    Ok(Snapshot::Note {
        kind: super::enum_of::<NoteKind>("memory_notes.kind", &row.kind)?,
        content: row.content.clone(),
        chapter_id: row.chapter_id,
        importance: row.importance,
        status: super::enum_of::<NoteStatus>("memory_notes.status", &row.status)?,
        entity_ids,
        track_id: row.track_id,
    })
}

pub fn foreshadowing_snapshot(
    row: &crate::db::entities::foreshadowing::Model,
    entity_ids: Vec<i32>,
) -> CanonResult<Snapshot> {
    Ok(Snapshot::Foreshadowing {
        title: row.title.clone(),
        description: row.description.clone(),
        planned_payoff: row.planned_payoff.clone(),
        status: super::enum_of::<ForeshadowingStatus>("foreshadowing.status", &row.status)?,
        planted_chapter_id: row.planted_chapter_id,
        resolved_chapter_id: row.resolved_chapter_id,
        resolution: row.resolution.clone(),
        importance: row.importance,
        entity_ids,
        track_id: row.track_id,
    })
}

/// The snapshot of one chapter's appearance set (spec/02 §4 `SnapshotAppearanceSet`).
pub fn appearance_set_snapshot(rows: &[appearances::Model]) -> Snapshot {
    let mut sorted: Vec<&appearances::Model> = rows.iter().collect();
    sorted.sort_by_key(|row| row.entity_id);
    Snapshot::AppearanceSet {
        appearances: sorted
            .into_iter()
            .map(|row| AppearanceSnapshot {
                entity_id: row.entity_id,
                role: AppearanceRole::from_str_opt(&row.role).unwrap_or(AppearanceRole::Mentioned),
                note: row.note.clone(),
            })
            .collect(),
    }
}

pub fn aliases_of(row: &entities::Model) -> CanonResult<Vec<String>> {
    parse_json("entities.aliases", &row.aliases)
}

/// The normalized alias list: trimmed, non-empty, unique, in insertion order.
pub fn clean_aliases(aliases: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for alias in aliases {
        let alias = alias.trim();
        if alias.is_empty() || !seen.insert(alias.to_owned()) {
            continue;
        }
        result.push(alias.to_owned());
    }
    result
}

// ───────────────────────────── entity links (spec/02 §2) ─────────────────────────────

/// One of the three `*_entities` join tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkTable {
    TimelineEvent,
    MemoryNote,
    Foreshadowing,
}

impl LinkTable {
    pub fn table(self) -> &'static str {
        match self {
            LinkTable::TimelineEvent => "timeline_event_entities",
            LinkTable::MemoryNote => "memory_note_entities",
            LinkTable::Foreshadowing => "foreshadowing_entities",
        }
    }

    pub fn owner_column(self) -> &'static str {
        match self {
            LinkTable::TimelineEvent => "event_id",
            LinkTable::MemoryNote => "note_id",
            LinkTable::Foreshadowing => "foreshadowing_id",
        }
    }
}

/// The entity ids linked to a timeline event, note or foreshadowing, ascending.
pub async fn linked_entity_ids(
    conn: &impl ConnectionTrait,
    table: LinkTable,
    owner_id: i32,
) -> CanonResult<Vec<i32>> {
    let sql = format!(
        "SELECT entity_id AS value FROM {} WHERE {} = ? ORDER BY entity_id",
        table.table(),
        table.owner_column()
    );
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            sql,
            vec![owner_id.into()],
        ))
        .await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(row.try_get::<i32>("", "value")?);
    }
    Ok(ids)
}

/// Replaces the entity links of one target (spec/05: `entities` replaces the set).
pub async fn set_linked_entities(
    tx: &impl ConnectionTrait,
    table: LinkTable,
    owner_id: i32,
    entity_ids: &[i32],
) -> CanonResult<()> {
    let owner_column = table.owner_column();
    let delete = format!("DELETE FROM {} WHERE {owner_column} = ?", table.table());
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        delete,
        vec![owner_id.into()],
    ))
    .await?;
    let mut seen = HashSet::new();
    for entity_id in entity_ids {
        if !seen.insert(*entity_id) {
            continue;
        }
        let insert = format!(
            "INSERT OR IGNORE INTO {}({owner_column}, entity_id) VALUES (?, ?)",
            table.table()
        );
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            insert,
            vec![owner_id.into(), (*entity_id).into()],
        ))
        .await?;
    }
    Ok(())
}

// ───────────────────────────── loading ─────────────────────────────

/// Loads one visible entity or reports `not_found` (spec/06 §2).
pub async fn load_visible_entity(
    conn: &impl ConnectionTrait,
    visibility: Visibility,
    entity_id: i32,
) -> CanonResult<entities::Model> {
    let row = entities::Entity::find_by_id(entity_id)
        .filter(entities::Column::BookId.eq(visibility.book_id))
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("entity #{entity_id}")))?;
    if !visibility.sees(row.track_id) {
        return Err(CanonError::not_found(format!("entity #{entity_id}")));
    }
    Ok(row)
}

/// Loads one entity of a book by id, whatever its track (the author's API).
pub async fn load_entity(
    conn: &impl ConnectionTrait,
    entity_id: i32,
) -> CanonResult<entities::Model> {
    entities::Entity::find_by_id(entity_id)
        .one(conn)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("entity #{entity_id}")))
}

// ───────────────────────────── change recording ─────────────────────────────

/// The parts every canon change needs (spec/06 §3, spec/02 §4).
pub struct ChangeWrite<'a> {
    pub book_id: i32,
    pub target_type: ChangeTargetType,
    pub target_id: i32,
    pub op: ChangeOp,
    pub before: Option<&'a Snapshot>,
    pub after: Option<&'a Snapshot>,
    pub note: &'a str,
    pub reverts_id: Option<i32>,
    /// The `track_id` of the target row: an isolated track's history goes with the track.
    pub track_id: Option<i32>,
    /// The chapter the change belongs to (falls back to the writer's session chapter).
    pub chapter_id: Option<i32>,
}

/// Records one canon change and returns its id.
pub async fn record_change(
    tx: &impl ConnectionTrait,
    write: &CanonWrite,
    change: ChangeWrite<'_>,
) -> CanonResult<i32> {
    // A revert's inverse operation writes the revert row itself: it gets the `system` actor, the
    // author's revert note and `reverts_id`, so no second row is written for one operation
    // (D-079).
    let revert = write.revert.as_ref();
    Ok(change_log_service::record(
        tx,
        change_log_service::NewChange {
            book_id: change.book_id,
            track_id: change.track_id,
            chapter_id: change.chapter_id.or(write.chapter_id),
            session_id: write.session_id,
            actor: match revert {
                Some(_) => ChangeActor::System,
                None => write.actor,
            },
            target_type: change.target_type,
            target_id: change.target_id,
            op: change.op,
            before: change.before,
            after: change.after,
            note: match revert {
                Some(_) => "",
                None => change.note,
            },
            reverts_id: change.reverts_id.or(revert.map(|revert| revert.of_change)),
        },
    )
    .await?)
}

/// The id a recreated row must take (spec/02 §5.10: a revert of a delete uses a **new** id).
///
/// SQLite reuses the rowid of a deleted maximum row, so letting the insert allocate the id can
/// hand a recreated row the id the deleted one had — and then a target id names two different
/// rows, which breaks `change.not_latest` and the history pages. The change log is the
/// high-water mark: every canon row ever written has a `create` change naming its id, so the
/// next id is one past the larger of the live table's max and the logged max for this target.
pub async fn fresh_id(
    conn: &impl ConnectionTrait,
    table: &str,
    target_type: ChangeTargetType,
) -> CanonResult<i32> {
    // `table` is a compile-time constant at every call site, never user input.
    let row = conn
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!("SELECT max(id) AS value FROM {table}"),
        ))
        .await?;
    let live = match row {
        Some(row) => row.try_get::<Option<i64>>("", "value")?.unwrap_or(0),
        None => 0,
    };
    let logged = crate::db::entities::change_log::Entity::find()
        .filter(crate::db::entities::change_log::Column::TargetType.eq(target_type.as_str()))
        .order_by_desc(crate::db::entities::change_log::Column::TargetId)
        .one(conn)
        .await?
        .map(|row| row.target_id)
        .unwrap_or(0);
    Ok((live.max(i64::from(logged)) + 1) as i32)
}

/// The `canon.changed` event of one committed change (spec/10 §3).
pub fn changed_event(
    book_id: i32,
    target_type: ChangeTargetType,
    target_id: i32,
    op: ChangeOp,
    change_id: i32,
) -> ServerEvent {
    ServerEvent::CanonChanged {
        book_id,
        target_type,
        target_id,
        op,
        change_id,
    }
}

/// The timestamp helper of the canon services.
pub fn stamp() -> chrono::DateTime<chrono::Utc> {
    now()
}

// ───────────────────────────── chapter summary (spec/06 §1.6) ─────────────────────────────

/// Restores a chapter's summary from a snapshot (the revert of a `chapter_summary` change,
/// spec/06 §3). The summary lives on the chapter row; the change row is keyed by the chapter id.
pub async fn restore_summary_inner(
    tx: &impl ConnectionTrait,
    chapter_id: i32,
    before: &Snapshot,
) -> CanonResult<()> {
    use sea_orm::ActiveModelTrait;
    let Snapshot::ChapterSummary { summary } = before else {
        return Err(CanonError::internal(
            "a summary revert needs a chapter-summary snapshot",
        ));
    };
    let chapter = chapters::Entity::find_by_id(chapter_id)
        .one(tx)
        .await?
        .ok_or_else(|| CanonError::not_found(format!("chapter #{chapter_id}")))?;
    if chapter.summary == *summary {
        return Ok(());
    }
    crate::db::entities::chapters::ActiveModel {
        id: Set(chapter_id),
        summary: Set(summary.clone()),
        updated_at: Set(now()),
        ..Default::default()
    }
    .update(tx)
    .await?;
    Ok(())
}

/// The snapshot of a chapter summary (spec/02 §4 `SnapshotChapterSummary`).
pub fn summary_snapshot(summary: &str) -> Snapshot {
    Snapshot::ChapterSummary {
        summary: summary.to_owned(),
    }
}

// ───────────────────────────── shared conversions ─────────────────────────────

/// Reads a plain-TEXT enum column into its Rust enum (the canon services' spelling of
/// [`crate::services::enum_of`]).
pub(crate) fn enum_of<T: std::str::FromStr<Err = String>>(
    column: &str,
    raw: &str,
) -> CanonResult<T> {
    crate::services::enum_of(column, raw).map_err(CanonError::from)
}

/// Deserializes a typed JSON column.
pub(crate) fn parse_json<T: for<'de> serde::Deserialize<'de>>(
    column: &str,
    raw: &str,
) -> CanonResult<T> {
    crate::services::parse_json(column, raw).map_err(CanonError::from)
}

/// Serializes a typed JSON column.
pub(crate) fn json<T: serde::Serialize>(value: &T) -> CanonResult<String> {
    crate::services::json(value).map_err(CanonError::from)
}

/// A `conflict.exists` carrying the existing row's id (spec/10 §4).
pub(crate) fn conflict(message: impl Into<String>, id: i32) -> CanonError {
    CanonError::App(crate::services::conflict(
        crate::error::ErrorCode::ConflictExists,
        message,
        serde_json::json!({ "id": id }),
    ))
}

/// The maximum importance a canon row may carry (spec/02 §2 `CHECK (importance BETWEEN 1 AND 5)`).
pub fn validate_importance(importance: i32) -> CanonResult<()> {
    if (1..=5).contains(&importance) {
        Ok(())
    } else {
        Err(CanonError::field(
            "importance",
            "importance must be between 1 and 5",
        ))
    }
}

/// The number of recent changes the entity detail carries (spec/08 §8.8 history).
pub const DETAIL_CHANGE_LIMIT: usize = 20;

// ───────────────────────────── transactional writes ─────────────────────────────

/// Encodes a [`CanonError`] inside `DbErr::Custom` so it survives [`crate::db::Db::write`]
/// without losing the `isolated_readonly` distinction (the technique of `services::write`,
/// D-064).
const ERROR_MARKER: &str = "nekowriter-canon-error:";

#[derive(Serialize, Deserialize)]
enum WireError {
    App {
        code: crate::error::ErrorCode,
        message: String,
        details: Option<serde_json::Value>,
    },
    IsolatedReadonly {
        target: String,
    },
}

/// Runs one canon write transaction with typed errors.
///
/// Every canon write gets exactly one transaction: the row, its join rows and its `change_log`
/// record commit together, so a failure can never leave canon changed without its history
/// (spec/06 §3).
pub async fn write<T, F>(db: &crate::db::Db, f: F) -> CanonResult<(T, Vec<ServerEvent>)>
where
    T: Send + 'static,
    F: Send
        + 'static
        + for<'c> FnOnce(
            &'c sea_orm::DatabaseTransaction,
        ) -> BoxFuture<'c, CanonResult<(T, Vec<ServerEvent>)>>,
{
    db.write(|tx| {
        Box::pin(async move {
            f(tx).await.map_err(|error| {
                let wire = match error {
                    CanonError::App(error) => WireError::App {
                        code: error.code,
                        message: error.message,
                        details: error.details,
                    },
                    CanonError::IsolatedReadonly { target } => {
                        WireError::IsolatedReadonly { target }
                    }
                };
                match serde_json::to_string(&wire) {
                    Ok(json) => sea_orm::DbErr::Custom(format!("{ERROR_MARKER}{json}")),
                    Err(_) => sea_orm::DbErr::Custom("canon write failed".to_owned()),
                }
            })
        })
    })
    .await
    .map_err(decode_error)
}

/// Recovers a [`CanonError`] encoded by [`write`], or converts a genuine database error.
fn decode_error(error: sea_orm::DbErr) -> CanonError {
    if let sea_orm::DbErr::Custom(message) = &error
        && let Some(json) = message.strip_prefix(ERROR_MARKER)
        && let Ok(wire) = serde_json::from_str::<WireError>(json)
    {
        return match wire {
            WireError::App {
                code,
                message,
                details,
            } => CanonError::App(AppError {
                code,
                message,
                details,
            }),
            WireError::IsolatedReadonly { target } => CanonError::IsolatedReadonly { target },
        };
    }
    CanonError::from(error)
}

/// The visibility of a track (main story and canon-mode side tracks see shared canon only).
pub fn visibility_of_track(book_id: i32, track: Option<&tracks::Model>) -> Visibility {
    match track {
        Some(track) if track.canon_mode == "isolated" => Visibility::isolated(book_id, track.id),
        _ => Visibility::shared(book_id),
    }
}

/// The entities a caller can name, newest-importance first (used by the tool resolver).
pub async fn visible_entities(
    conn: &impl ConnectionTrait,
    visibility: Visibility,
) -> CanonResult<Vec<entities::Model>> {
    Ok(entities::Entity::find()
        .filter(entities::Column::BookId.eq(visibility.book_id))
        .filter(visibility.visible_condition(entities::Column::TrackId))
        .order_by_asc(entities::Column::Id)
        .all(conn)
        .await?)
}
