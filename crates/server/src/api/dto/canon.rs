//! Canon DTOs (spec/10 §2.4, spec/06 §1, §4).
//!
//! The canon pages list entities and the timeline whole and sort client-side (D-043), so the list
//! rows carry everything the table and the filter bar need. The detail row adds the relations,
//! appearances, recent changes and the character-design link (spec/10 §2.4).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{
    AnchorKind, AppearanceRole, ChangeActor, EntityKind, EntityStatus, ForeshadowingStatus,
    NoteKind, NoteStatus, RelationStatus,
};

// ───────────────────────────── entities ─────────────────────────────

/// One row of the entity table (spec/06 §1.1).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityListItemDto {
    pub id: i32,
    pub book_id: i32,
    /// `None` for shared canon, `Some` for an isolated track's private row (spec/06 §2).
    pub track_id: Option<i32>,
    pub kind: EntityKind,
    pub name: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub importance: i32,
    pub status: EntityStatus,
    pub is_protagonist: bool,
    pub first_chapter_id: Option<i32>,
    pub last_chapter_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The entity detail (spec/10 §2.4): the row plus its relations, appearances, recent changes and
/// the linked character design.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub kind: EntityKind,
    pub name: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub description: String,
    pub state: String,
    pub attributes: BTreeMap<String, String>,
    pub importance: i32,
    pub status: EntityStatus,
    pub is_protagonist: bool,
    pub first_chapter_id: Option<i32>,
    pub last_chapter_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub relations: Vec<RelationDto>,
    pub appearances: Vec<AppearanceDto>,
    /// Newest first, at most 20 (spec/08 §8.8 history).
    pub recent_changes: Vec<super::changes::ChangeDto>,
    pub design: Option<DesignLinkDto>,
}

/// The character design an entity is linked to (spec/05 §6 `create_entity`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DesignLinkDto {
    pub design_id: i32,
    pub rank: i32,
    pub role_label: String,
    pub summary: String,
}

/// `POST /books/:id/entities` body (spec/10 §2.4).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityInput {
    pub kind: EntityKind,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default = "default_importance")]
    pub importance: i32,
    #[serde(default = "default_entity_status")]
    pub status: EntityStatus,
    #[serde(default)]
    pub is_protagonist: bool,
    /// Set only for a private row of an isolated track (spec/06 §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
    /// The author's change note (spec/06 §3); empty by default.
    #[serde(default)]
    pub note: String,
}

/// `PATCH /entities/:id` body: omitted fields are unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<EntityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<EntityStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_protagonist: Option<bool>,
    #[serde(default)]
    pub note: String,
}

/// `POST /entities/:id/merge` body (spec/06 §9).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityMergeInput {
    pub from_id: i32,
}

/// `GET /books/:id/entities` query (spec/10 §2.4). Filtering is client-side by design (D-043);
/// the query only narrows the response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EntityQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<EntityKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<EntityStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    /// Include an isolated track's private rows (spec/06 §2 track filter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
}

impl Default for EntityInput {
    fn default() -> Self {
        Self {
            kind: EntityKind::Other,
            name: String::new(),
            aliases: Vec::new(),
            summary: String::new(),
            description: String::new(),
            state: String::new(),
            attributes: BTreeMap::new(),
            importance: default_importance(),
            status: default_entity_status(),
            is_protagonist: false,
            track_id: None,
            note: String::new(),
        }
    }
}

fn default_importance() -> i32 {
    2
}

fn default_entity_status() -> EntityStatus {
    EntityStatus::Active
}

// ───────────────────────────── relations ─────────────────────────────

/// One directed relation (spec/06 §1.2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelationDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub source_id: i32,
    pub target_id: i32,
    pub kind: String,
    pub inverse_kind: String,
    pub description: String,
    pub status: RelationStatus,
    pub since_chapter_id: Option<i32>,
    pub until_chapter_id: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /books/:id/relations` body (spec/10 §2.4).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelationInput {
    pub source_id: i32,
    pub target_id: i32,
    pub kind: String,
    #[serde(default)]
    pub inverse_kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_chapter_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_chapter_id: Option<i32>,
    #[serde(default)]
    pub note: String,
}

/// `PATCH /relations/:id` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelationPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<RelationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_chapter_id: Option<Option<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_chapter_id: Option<Option<i32>>,
    #[serde(default)]
    pub note: String,
}

/// `GET /books/:id/relations` query.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelationQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<i32>,
}

// ───────────────────────────── appearances ─────────────────────────────

/// One appearance row (spec/06 §1.3).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppearanceDto {
    pub id: i32,
    pub chapter_id: i32,
    pub entity_id: i32,
    pub role: AppearanceRole,
    pub note: String,
}

/// One entry of an appearance set (spec/02 §4 `SnapshotAppearanceSet`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AppearanceInput {
    pub entity_id: i32,
    pub role: AppearanceRole,
    #[serde(default)]
    pub note: String,
}

// ───────────────────────────── timeline ─────────────────────────────

/// One timeline event (spec/06 §1.4). `position` is story chronology, not chapter order.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineEventDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub position: f64,
    pub story_time: String,
    pub title: String,
    pub description: String,
    pub chapter_id: Option<i32>,
    pub importance: i32,
    pub entity_ids: Vec<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /books/:id/timeline` body (spec/10 §2.4): `after_id` / `before_id` place the new event.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineEventInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub story_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<i32>,
    #[serde(default)]
    pub entity_ids: Vec<i32>,
    #[serde(default = "default_importance")]
    pub importance: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<i32>,
    #[serde(default)]
    pub note: String,
}

/// `PATCH /timeline-events/:id` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineEventPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<Option<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_ids: Option<Vec<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<i32>,
    #[serde(default)]
    pub note: String,
}

/// `POST /timeline-events/:id/move` body (spec/10 §2.4).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineMoveInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_id: Option<i32>,
    #[serde(default)]
    pub note: String,
}

/// `GET /books/:id/timeline` query.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TimelineQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<i32>,
}

// ───────────────────────────── memory notes ─────────────────────────────

/// One memory note (spec/06 §1.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryNoteDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub kind: NoteKind,
    pub content: String,
    pub chapter_id: Option<i32>,
    pub importance: i32,
    pub status: NoteStatus,
    pub entity_ids: Vec<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /books/:id/memory-notes` body.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryNoteInput {
    pub kind: NoteKind,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<i32>,
    #[serde(default = "default_importance")]
    pub importance: i32,
    #[serde(default)]
    pub entity_ids: Vec<i32>,
    #[serde(default)]
    pub note: String,
}

/// `PATCH /memory-notes/:id` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryNotePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<NoteKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<Option<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NoteStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_ids: Option<Vec<i32>>,
    #[serde(default)]
    pub note: String,
}

/// `GET /books/:id/memory-notes` query.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryNoteQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<NoteKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NoteStatus>,
}

// ───────────────────────────── foreshadowing ─────────────────────────────

/// One foreshadowing item (spec/06 §1.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForeshadowingDto {
    pub id: i32,
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub title: String,
    pub description: String,
    pub planned_payoff: String,
    pub status: ForeshadowingStatus,
    pub planted_chapter_id: Option<i32>,
    pub resolved_chapter_id: Option<i32>,
    pub resolution: String,
    pub importance: i32,
    pub entity_ids: Vec<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /books/:id/foreshadowing` body.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForeshadowingInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub planned_payoff: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planted_chapter_id: Option<i32>,
    #[serde(default = "default_importance")]
    pub importance: i32,
    #[serde(default)]
    pub entity_ids: Vec<i32>,
    #[serde(default)]
    pub note: String,
}

/// `PATCH /foreshadowing/:id` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForeshadowingPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_payoff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ForeshadowingStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planted_chapter_id: Option<Option<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_chapter_id: Option<Option<i32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_ids: Option<Vec<i32>>,
    #[serde(default)]
    pub note: String,
}

/// `GET /books/:id/foreshadowing` query.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ForeshadowingQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ForeshadowingStatus>,
}

// ───────────────────────────── memory snapshot (spec/06 §4) ─────────────────────────────

/// `GET /books/:id/memory` (spec/10 §2.4): the snapshot the writer of the target chapter receives
/// plus the exact `block.memory` text (spec/06 §4.5).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryViewDto {
    pub snapshot: MemorySnapshotDto,
    pub rendered: String,
}

/// One target position of the Memory page (spec/06 §4.2 `chapter`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryChapterDto {
    pub id: i32,
    pub number: i32,
    pub label: String,
    pub title: String,
    pub guidance: String,
    pub outline: String,
    pub target_min: i32,
    pub target_max: i32,
    pub has_outline: bool,
}

/// One chapter of the short-term window (spec/06 §4.2 `recent_summaries`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemorySummaryDto {
    pub chapter_id: i32,
    pub label: String,
    pub title: String,
    pub summary: String,
}

/// The previous chapter's ending (spec/06 §4.2 `previous_tail`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryTailDto {
    pub chapter_id: i32,
    pub label: String,
    pub text: String,
}

/// The protagonist's current state (spec/06 §4.2 `protagonist`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryProtagonistDto {
    pub id: i32,
    pub name: String,
    pub status: EntityStatus,
    pub state: String,
    /// A side track's newest state change comes from a main chapter after the anchor (§4.2).
    pub after_anchor: bool,
    pub attributes: BTreeMap<String, String>,
}

/// One mid-term change note (spec/06 §4.3 `recent_changes`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryChangeDto {
    pub chapter_id: Option<i32>,
    pub chapter_label: String,
    pub target: String,
    pub note: String,
}

/// One character of the mid-term window (spec/06 §4.3 `appearances`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryAppearanceDto {
    pub id: i32,
    pub name: String,
    pub count: i32,
    pub chapters: Vec<MemoryAppearanceChapterDto>,
}

/// One appearance of that character.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryAppearanceChapterDto {
    pub chapter_id: i32,
    pub label: String,
    pub role: AppearanceRole,
}

/// An important character that has not appeared lately (spec/06 §4.3 `absent_important`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryAbsenceDto {
    pub id: i32,
    pub name: String,
    pub last_chapter_id: i32,
    pub last_label: String,
}

/// One long-term index note (spec/06 §4.4 `key_notes`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryNoteItemDto {
    pub id: i32,
    pub kind: NoteKind,
    pub content: String,
}

/// One open foreshadowing (spec/06 §4.4 `open_foreshadowing`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryForeshadowItemDto {
    pub id: i32,
    pub title: String,
    pub importance: i32,
    pub planted_chapter_id: Option<i32>,
    pub planted_label: String,
}

/// One cast row of the long-term index (spec/06 §4.4 `cast`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryCastItemDto {
    pub id: i32,
    pub name: String,
    pub kind: EntityKind,
    pub summary: String,
    pub importance: i32,
    pub last_chapter_id: Option<i32>,
}

/// The archive size line of `block.memory` (spec/06 §4.4 `counts`).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryCountsDto {
    pub character: i32,
    pub organization: i32,
    pub location: i32,
    pub item: i32,
    pub other: i32,
    pub event: i32,
    pub fact: i32,
    pub world_rule: i32,
    pub note_other: i32,
    pub foreshadowing_open: i32,
}

/// A side track's time anchor (spec/06 §2, §4).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryAnchorDto {
    pub kind: AnchorKind,
    pub chapter_id: Option<i32>,
    pub label: String,
}

/// The whole tiered memory snapshot (spec/06 §4).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemorySnapshotDto {
    pub chapter: MemoryChapterDto,
    /// True when no earlier chapter of the position is official yet (spec/06 §4.2).
    pub is_first: bool,
    /// The number of chapters the mid-term window covers (M).
    pub window_mid: i32,
    pub recent_summaries: Vec<MemorySummaryDto>,
    pub previous_tail: Option<MemoryTailDto>,
    pub protagonist: Option<MemoryProtagonistDto>,
    pub recent_changes: Vec<MemoryChangeDto>,
    pub appearances: Vec<MemoryAppearanceDto>,
    pub absent_important: Vec<MemoryAbsenceDto>,
    pub key_notes: Vec<MemoryNoteItemDto>,
    pub open_foreshadowing: Vec<MemoryForeshadowItemDto>,
    pub cast: Vec<MemoryCastItemDto>,
    pub counts: MemoryCountsDto,
    pub anchor: Option<MemoryAnchorDto>,
}

/// `GET /books/:id/memory` query (spec/10 §2.4). Without `chapter_id` the position is the next
/// chapter of the main track.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MemoryQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<i32>,
}

// ───────────────────────────── revert ─────────────────────────────

/// `POST /changes/:id/revert` body: the author's optional note on the revert.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RevertInput {
    /// The revert itself is a system change with an empty note by default (spec/06 §3).
    #[serde(default)]
    pub note: String,
}

/// The actor a canon write records (spec/06 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteActor {
    User,
    Agent,
    System,
}

impl WriteActor {
    pub fn change_actor(self) -> ChangeActor {
        match self {
            WriteActor::User => ChangeActor::User,
            WriteActor::Agent => ChangeActor::Agent,
            WriteActor::System => ChangeActor::System,
        }
    }
}
