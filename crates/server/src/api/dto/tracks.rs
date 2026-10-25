//! Track DTOs (spec/10 §2.2, spec/06 §2, spec/08 §6).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::json::{AnchorKind, CanonMode, TrackKind};

/// One track of a book. The main track's `title` is empty and its `ord` is 0 (spec/02 §2).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrackDto {
    pub id: i32,
    pub book_id: i32,
    pub kind: TrackKind,
    pub title: String,
    pub synopsis: String,
    pub canon_mode: CanonMode,
    pub anchor_kind: AnchorKind,
    pub anchor_chapter_id: Option<i32>,
    pub focus_entity_id: Option<i32>,
    pub ord: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `POST /books/:id/tracks` body (spec/08 §6).
///
/// Without `kind` the endpoint creates a **side track** (spec/08 §6: the body is a side story);
/// `kind: "main"` restores a missing main track (M1, spec/10 §2.2) and takes no side-story
/// settings. The side-track defaults are applied by the service because they depend on the
/// book's chapters: `canon_mode = isolated`, and an `after_chapter` anchor on the latest official
/// main chapter (`before_start` when there is none).
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(optional_fields = nullable)]
#[ts(export)]
pub struct TrackInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TrackKind>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub synopsis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canon_mode: Option<CanonMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_kind: Option<AnchorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_chapter_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_entity_id: Option<i32>,
}

/// `PATCH /tracks/:id` body. The main track accepts only its synopsis (D-037).
///
/// The focus character is set with `focus_entity_id` and removed with `clear_focus_entity`:
/// `Option<Option<i32>>` would need a serde extension to tell "absent" from "null", and the
/// explicit flag keeps the wire type plain.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(optional_fields = nullable)]
#[ts(export)]
pub struct TrackPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synopsis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canon_mode: Option<CanonMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_kind: Option<AnchorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_chapter_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_entity_id: Option<i32>,
    /// Removes the focus character; `focus_entity_id` must be absent then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_focus_entity: bool,
}
