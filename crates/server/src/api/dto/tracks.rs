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

/// `POST /books/:id/tracks` body (spec/08 §6). Side stories arrive in M10; M1 only restores a
/// missing main track, so the field defaults describe a main track.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrackInput {
    #[serde(default = "main_kind")]
    pub kind: TrackKind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub synopsis: String,
    #[serde(default = "canon_mode")]
    pub canon_mode: CanonMode,
    #[serde(default = "anchor_none")]
    pub anchor_kind: AnchorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_chapter_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_entity_id: Option<i32>,
}

fn main_kind() -> TrackKind {
    TrackKind::Main
}

fn canon_mode() -> CanonMode {
    CanonMode::Canon
}

fn anchor_none() -> AnchorKind {
    AnchorKind::None
}

/// `PATCH /tracks/:id` body. The main track accepts only its synopsis.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
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
}
