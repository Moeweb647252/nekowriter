//! `search_docs`/`knowledge_chunks` maintenance (spec/07 §2, §1.1): one index row per indexable
//! item, upserted in the same transaction as the source change, plus the hash diff that lets
//! unchanged chunks keep their row, FTS row and vector (D-039).
//!
//! FTS columns hold jieba-segmented, space-joined tokens (spec/07 §5); the searchable text is
//! never copied into the FTS table unsegmented, or CJK queries could not match.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter,
    QueryOrder, Set, Statement,
};

use crate::db::entities::{
    chapters, entities, foreshadowing, knowledge_chunks, knowledge_entries, memory_notes,
    search_docs, timeline_events, tracks,
};
use crate::db::json::{
    CanonMode, ChangeTargetType, ChapterContent, ChapterStatus, EmbedState, JsonText, NoteStatus,
    SearchDocKind,
};
use crate::error::AppError;
use crate::events::ServerEvent;

use super::chunk::{self, Chunk};
use super::fts;

/// The entity description cap in its search body (spec/07 §2).
pub const ENTITY_DESCRIPTION_CHARS: usize = 1_500;

/// The memory-note title is the first 40 characters of its content (spec/07 §2).
pub const NOTE_TITLE_CHARS: usize = 40;

/// One `search_docs` row to upsert.
pub struct NewDoc<'a> {
    pub book_id: i32,
    pub track_id: Option<i32>,
    pub kind: SearchDocKind,
    pub ref_id: i32,
    pub ref_part: i32,
    pub title: &'a str,
    pub body: &'a str,
    pub meta: &'a str,
}

/// The content hash of a book document (spec/07 §2). The body is what gets embedded and
/// searched; the title is searchable through the FTS title column and changing it does not
/// re-embed an otherwise unchanged document.
pub fn doc_hash(body: &str) -> String {
    blake3::hash(body.as_bytes()).to_hex().to_string()
}

/// Upserts one `search_docs` row and its FTS row. A changed hash becomes `pending` again; the old
/// vector stays until the indexer replaces it (spec/07 §2).
pub async fn upsert_doc(conn: &impl ConnectionTrait, doc: NewDoc<'_>) -> Result<(), AppError> {
    let hash = doc_hash(doc.body);
    let existing = search_docs::Entity::find()
        .filter(search_docs::Column::Kind.eq(doc.kind))
        .filter(search_docs::Column::RefId.eq(doc.ref_id))
        .filter(search_docs::Column::RefPart.eq(doc.ref_part))
        .one(conn)
        .await?;
    match existing {
        Some(row) => {
            let mut active = search_docs::ActiveModel {
                id: Set(row.id),
                book_id: Set(doc.book_id),
                track_id: Set(doc.track_id),
                title: Set(doc.title.to_owned()),
                body: Set(doc.body.to_owned()),
                meta: Set(JsonText(
                    serde_json::from_str(doc.meta).unwrap_or(serde_json::Value::Null),
                )),
                content_hash: Set(hash.clone()),
                updated_at: Set(chrono::Utc::now()),
                ..Default::default()
            };
            if row.content_hash != hash {
                active.embed_state = Set(EmbedState::Pending);
            }
            active.update(conn).await?;
            replace_fts(conn, row.id, doc.title, doc.body).await?;
        }
        None => {
            let row = search_docs::Entity::insert(search_docs::ActiveModel {
                book_id: Set(doc.book_id),
                track_id: Set(doc.track_id),
                kind: Set(doc.kind.to_owned()),
                ref_id: Set(doc.ref_id),
                ref_part: Set(doc.ref_part),
                title: Set(doc.title.to_owned()),
                body: Set(doc.body.to_owned()),
                meta: Set(JsonText(
                    serde_json::from_str(doc.meta).unwrap_or(serde_json::Value::Null),
                )),
                content_hash: Set(hash),
                embed_state: Set(EmbedState::Pending),
                updated_at: Set(chrono::Utc::now()),
                ..Default::default()
            })
            .exec_with_returning(conn)
            .await?;
            replace_fts(conn, row.id, doc.title, doc.body).await?;
        }
    }
    Ok(())
}

/// Removes every document of one source row (a deleted or retired item).
pub async fn delete_ref_docs(
    conn: &impl ConnectionTrait,
    kind: &str,
    ref_id: i32,
) -> Result<(), AppError> {
    search_docs::Entity::delete_many()
        .filter(search_docs::Column::Kind.eq(kind))
        .filter(search_docs::Column::RefId.eq(ref_id))
        .exec(conn)
        .await?;
    Ok(())
}

/// Writes one FTS row, skipped when the segmented columns already match (D-039: an unchanged
/// chunk keeps its FTS row; its delete trigger only fires from `search_docs`).
async fn replace_fts(
    conn: &impl ConnectionTrait,
    id: i32,
    title: &str,
    body: &str,
) -> Result<(), AppError> {
    let title_tokens = fts::segment(title);
    let body_tokens = fts::segment(body);
    if let Some(row) = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT title, body FROM fts_book WHERE rowid = ?",
            vec![(id as i64).into()],
        ))
        .await?
    {
        let current_title: String = row.try_get("", "title")?;
        let current_body: String = row.try_get("", "body")?;
        if current_title == title_tokens && current_body == body_tokens {
            return Ok(());
        }
    }
    conn.execute_unprepared(&format!("DELETE FROM fts_book WHERE rowid = {id}"))
        .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO fts_book(rowid, title, body) VALUES (?, ?, ?)",
        vec![(id as i64).into(), title_tokens.into(), body_tokens.into()],
    ))
    .await?;
    Ok(())
}

// ───────────────────────────── per-kind sync ─────────────────────────────

/// Keeps an entity's document in step with its row (deleted rows remove their document).
pub async fn sync_entity(
    conn: &impl ConnectionTrait,
    book_id: i32,
    entity_id: i32,
) -> Result<(), AppError> {
    let Some(row) = entities::Entity::find_by_id(entity_id).one(conn).await? else {
        return delete_ref_docs(conn, "entity", entity_id).await;
    };
    let body = entity_body(&row)?;
    upsert_doc(
        conn,
        NewDoc {
            book_id,
            track_id: row.track_id,
            kind: SearchDocKind::Entity,
            ref_id: row.id,
            ref_part: 0,
            title: &row.name,
            body: &body,
            meta: "{}",
        },
    )
    .await
}

/// The entity body of spec/07 §2: summary · aliases · state · attributes (`k: v`) · description.
pub fn entity_body(row: &entities::Model) -> Result<String, AppError> {
    let mut parts: Vec<String> = Vec::new();
    let summary = row.summary.trim();
    if !summary.is_empty() {
        parts.push(summary.to_owned());
    }
    let aliases: Vec<String> = row.aliases.clone().into_inner();
    if !aliases.is_empty() {
        parts.push(aliases.join(" / "));
    }
    let state = row.state.trim();
    if !state.is_empty() {
        parts.push(state.to_owned());
    }
    let attributes: std::collections::BTreeMap<String, String> =
        row.attributes.clone().into_inner();
    if !attributes.is_empty() {
        parts.push(
            attributes
                .iter()
                .map(|(key, value)| format!("{key}: {value}"))
                .collect::<Vec<_>>()
                .join(" · "),
        );
    }
    let description = row.description.trim();
    if !description.is_empty() {
        parts.push(description.chars().take(ENTITY_DESCRIPTION_CHARS).collect());
    }
    Ok(parts.join(" · "))
}

/// Keeps a memory note's document in step; retired and deleted notes remove theirs (spec/07 §2).
pub async fn sync_note(
    conn: &impl ConnectionTrait,
    book_id: i32,
    note_id: i32,
) -> Result<(), AppError> {
    let Some(row) = memory_notes::Entity::find_by_id(note_id).one(conn).await? else {
        return delete_ref_docs(conn, "note", note_id).await;
    };
    if row.status != NoteStatus::Active {
        return delete_ref_docs(conn, "note", note_id).await;
    }
    let title: String = row.content.trim().chars().take(NOTE_TITLE_CHARS).collect();
    upsert_doc(
        conn,
        NewDoc {
            book_id,
            track_id: row.track_id,
            kind: SearchDocKind::Note,
            ref_id: row.id,
            ref_part: 0,
            title: &title,
            body: &row.content,
            meta: "{}",
        },
    )
    .await
}

/// Keeps a foreshadowing item's document in step.
pub async fn sync_foreshadowing(
    conn: &impl ConnectionTrait,
    book_id: i32,
    item_id: i32,
) -> Result<(), AppError> {
    let Some(row) = foreshadowing::Entity::find_by_id(item_id).one(conn).await? else {
        return delete_ref_docs(conn, "foreshadowing", item_id).await;
    };
    let mut parts = Vec::new();
    for value in [&row.description, &row.planned_payoff, &row.resolution] {
        let value = value.trim();
        if !value.is_empty() {
            parts.push(value.to_owned());
        }
    }
    upsert_doc(
        conn,
        NewDoc {
            book_id,
            track_id: row.track_id,
            kind: SearchDocKind::Foreshadowing,
            ref_id: row.id,
            ref_part: 0,
            title: &row.title,
            body: &parts.join(" · "),
            meta: "{}",
        },
    )
    .await
}

/// Keeps a timeline event's document in step.
pub async fn sync_timeline_event(
    conn: &impl ConnectionTrait,
    book_id: i32,
    event_id: i32,
) -> Result<(), AppError> {
    let Some(row) = timeline_events::Entity::find_by_id(event_id)
        .one(conn)
        .await?
    else {
        return delete_ref_docs(conn, "event", event_id).await;
    };
    let mut parts = Vec::new();
    for value in [&row.story_time, &row.description] {
        let value = value.trim();
        if !value.is_empty() {
            parts.push(value.to_owned());
        }
    }
    upsert_doc(
        conn,
        NewDoc {
            book_id,
            track_id: row.track_id,
            kind: SearchDocKind::Event,
            ref_id: row.id,
            ref_part: 0,
            title: &row.title,
            body: &parts.join(" · "),
            meta: "{}",
        },
    )
    .await
}

/// The track visibility of a chapter's summary and passage documents (spec/07 §2): `NULL` for the
/// main and canon-mode tracks, the track id for an isolated one.
async fn chapter_doc_track(
    conn: &impl ConnectionTrait,
    track_id: i32,
) -> Result<Option<i32>, AppError> {
    let Some(track) = tracks::Entity::find_by_id(track_id).one(conn).await? else {
        return Ok(None);
    };
    Ok(if track.canon_mode == CanonMode::Isolated {
        Some(track.id)
    } else {
        None
    })
}

/// Keeps the `summary` document of an official chapter in step (spec/07 §2).
pub async fn sync_chapter_summary(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<(), AppError> {
    let Some(row) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return delete_ref_docs(conn, "summary", chapter_id).await;
    };
    // Only official chapters are indexed (spec/07 §2), and an empty summary has nothing to find.
    if row.status != ChapterStatus::Official || row.summary.trim().is_empty() {
        return delete_ref_docs(conn, "summary", chapter_id).await;
    }
    let track_id = chapter_doc_track(conn, row.track_id).await?;
    upsert_doc(
        conn,
        NewDoc {
            book_id: row.book_id,
            track_id,
            kind: SearchDocKind::Summary,
            ref_id: row.id,
            ref_part: 0,
            title: &row.title,
            body: &row.summary,
            meta: "{}",
        },
    )
    .await
}

/// Rebuilds an official chapter's passage documents from its current text (spec/07 §2), with the
/// same hash diff as knowledge chunks so unchanged passages keep their vectors (D-039, spec/07
/// §1.1).
pub async fn sync_passages(conn: &impl ConnectionTrait, chapter_id: i32) -> Result<(), AppError> {
    let Some(row) = chapters::Entity::find_by_id(chapter_id).one(conn).await? else {
        return delete_ref_docs(conn, "passage", chapter_id).await;
    };
    if row.status != ChapterStatus::Official {
        return delete_ref_docs(conn, "passage", chapter_id).await;
    }
    let track_id = chapter_doc_track(conn, row.track_id).await?;
    let content: ChapterContent = row.content.clone().into_inner();
    let paragraphs: Vec<(String, String)> = content
        .paragraphs
        .iter()
        .map(|paragraph| (paragraph.pid.clone(), paragraph.text.clone()))
        .collect();
    let passages = chunk::chunk_passages(&paragraphs);
    sync_chunk_rows(
        conn,
        SyncRows {
            book_id: row.book_id,
            track_id,
            kind: SearchDocKind::Passage,
            ref_id: row.id,
            title: &row.title,
            entries: passages
                .into_iter()
                .map(|passage| SyncEntry {
                    text: passage.text,
                    meta: serde_json::json!({ "pids": passage.pids }).to_string(),
                })
                .collect(),
        },
    )
    .await
}

/// Keeps an official chapter's summary and passages in step (approval, title/summary edits and
/// content changes, spec/07 §2).
pub async fn sync_chapter(conn: &impl ConnectionTrait, chapter_id: i32) -> Result<(), AppError> {
    sync_chapter_summary(conn, chapter_id).await?;
    sync_passages(conn, chapter_id).await
}

/// Re-chunks one knowledge entry in the same transaction as the entry change (spec/07 §1.1,
/// §2): unchanged chunks keep their id, FTS row, vector and `embed_state`; only `ord` changes.
pub async fn sync_knowledge_entry(
    conn: &impl ConnectionTrait,
    entry_id: i32,
) -> Result<(), AppError> {
    let Some(entry) = knowledge_entries::Entity::find_by_id(entry_id)
        .one(conn)
        .await?
    else {
        knowledge_chunks::Entity::delete_many()
            .filter(knowledge_chunks::Column::EntryId.eq(entry_id))
            .exec(conn)
            .await?;
        return Ok(());
    };
    let chunks = chunk::chunk_markdown(&entry.content);
    sync_knowledge_chunk_rows(conn, &entry, chunks).await
}

/// The hash diff of spec/07 §1.1 (item 4) for one knowledge entry.
async fn sync_knowledge_chunk_rows(
    conn: &impl ConnectionTrait,
    entry: &knowledge_entries::Model,
    chunks: Vec<Chunk>,
) -> Result<(), AppError> {
    let existing = knowledge_chunks::Entity::find()
        .filter(knowledge_chunks::Column::EntryId.eq(entry.id))
        .order_by_asc(knowledge_chunks::Column::Ord)
        .all(conn)
        .await?;
    // Match by content hash, one to one, so repeated identical chunks are counted (spec/07 §1.1).
    let mut pool: std::collections::HashMap<String, Vec<knowledge_chunks::Model>> =
        std::collections::HashMap::new();
    for row in existing {
        pool.entry(row.content_hash.clone()).or_default().push(row);
    }
    // `kept` records each matched old row **with the new chunk index it matched**, because a chunk
    // that has no old row takes no entry here: only the chunk's own index may select its row, or
    // inserting chunks in front of unchanged ones would re-label the wrong rows (spec/07 §1.1).
    let mut kept: Vec<(i32, bool, usize)> = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let hash = chunk::content_hash(&chunk.heading, &chunk.text);
        if let Some(row) = pool.get_mut(&hash).and_then(|rows| rows.pop()) {
            // The chunk is identical: `ord` is the only column that may change.
            kept.push((row.id, row.ord != index as i32, index));
        }
    }
    let stale: Vec<i32> = pool.into_values().flatten().map(|row| row.id).collect();

    // Two-phase renumbering: park every kept row on a unique negative `ord` so no intermediate
    // value collides with a live one (spec/02 §1, like chapter `ord`).
    for (position, (id, changed, _)) in kept.iter().enumerate() {
        if *changed {
            conn.execute_unprepared(&format!(
                "UPDATE knowledge_chunks SET ord = {} WHERE id = {id}",
                -(position as i64) - 1
            ))
            .await?;
        }
    }
    for id in &stale {
        knowledge_chunks::Entity::delete_by_id(*id)
            .exec(conn)
            .await?;
    }
    // Write the final positions: reuse the kept rows where they match and insert the new chunks.
    let kept_ids: std::collections::HashMap<usize, i32> =
        kept.iter().map(|(id, _, index)| (*index, *id)).collect();
    for (index, chunk) in chunks.iter().enumerate() {
        let hash = chunk::content_hash(&chunk.heading, &chunk.text);
        match kept_ids.get(&index) {
            Some(id) => {
                conn.execute_unprepared(&format!(
                    "UPDATE knowledge_chunks SET ord = {index}, heading = {}, text = {}, \
                     content_hash = {} WHERE id = {id}",
                    sql_string(&chunk.heading),
                    sql_string(&chunk.text),
                    sql_string(&hash)
                ))
                .await?;
            }
            None => {
                let row = knowledge_chunks::Entity::insert(knowledge_chunks::ActiveModel {
                    entry_id: Set(entry.id),
                    scope_id: Set(entry.scope_id),
                    ord: Set(index as i32),
                    heading: Set(chunk.heading.clone()),
                    text: Set(chunk.text.clone()),
                    content_hash: Set(hash),
                    embed_state: Set(EmbedState::Pending),
                    ..Default::default()
                })
                .exec_with_returning(conn)
                .await?;
                replace_knowledge_fts(
                    conn,
                    row.id,
                    &entry.title,
                    &knowledge_fts_body(&chunk.heading, &chunk.text),
                )
                .await?;
            }
        }
    }
    // The FTS title column carries the entry title for every chunk; a rename updates all rows.
    for (id, _, _) in kept.iter() {
        let Some(row) = knowledge_chunks::Entity::find_by_id(*id).one(conn).await? else {
            continue;
        };
        replace_knowledge_fts(
            conn,
            *id,
            &entry.title,
            &knowledge_fts_body(&row.heading, &row.text),
        )
        .await?;
    }
    Ok(())
}

/// The generic hash-diff for book documents of one source row (passages; spec/07 §2).
struct SyncRows<'a> {
    book_id: i32,
    track_id: Option<i32>,
    kind: SearchDocKind,
    ref_id: i32,
    title: &'a str,
    entries: Vec<SyncEntry>,
}

struct SyncEntry {
    text: String,
    meta: String,
}

async fn sync_chunk_rows(conn: &impl ConnectionTrait, rows: SyncRows<'_>) -> Result<(), AppError> {
    let existing = search_docs::Entity::find()
        .filter(search_docs::Column::Kind.eq(rows.kind))
        .filter(search_docs::Column::RefId.eq(rows.ref_id))
        .order_by_asc(search_docs::Column::RefPart)
        .all(conn)
        .await?;
    let mut pool: std::collections::HashMap<String, Vec<search_docs::Model>> =
        std::collections::HashMap::new();
    for row in existing {
        pool.entry(row.content_hash.clone()).or_default().push(row);
    }
    let mut kept: Vec<(i32, bool, usize)> = Vec::new();
    for (index, entry) in rows.entries.iter().enumerate() {
        let hash = doc_hash(&entry.text);
        if let Some(row) = pool.get_mut(&hash).and_then(|rows| rows.pop()) {
            kept.push((row.id, row.ref_part != index as i32, index));
        }
    }
    let stale: Vec<i32> = pool.into_values().flatten().map(|row| row.id).collect();
    // Park kept rows on negative `ref_part` values before writing final positions.
    for (position, (id, changed, _)) in kept.iter().enumerate() {
        if *changed {
            conn.execute_unprepared(&format!(
                "UPDATE search_docs SET ref_part = {} WHERE id = {id}",
                -(position as i64) - 1
            ))
            .await?;
        }
    }
    for id in &stale {
        search_docs::Entity::delete_by_id(*id).exec(conn).await?;
    }
    let kept_by_index: std::collections::HashMap<usize, i32> =
        kept.iter().map(|(id, _, index)| (*index, *id)).collect();
    for (index, entry) in rows.entries.iter().enumerate() {
        match kept_by_index.get(&index) {
            Some(id) => {
                // The text is unchanged (the hash matched); only the reported pids and the
                // position can differ.
                search_docs::ActiveModel {
                    id: Set(*id),
                    ref_part: Set(index as i32),
                    title: Set(rows.title.to_owned()),
                    meta: Set(JsonText(
                        serde_json::from_str(&entry.meta).unwrap_or(serde_json::Value::Null),
                    )),
                    updated_at: Set(chrono::Utc::now()),
                    ..Default::default()
                }
                .update(conn)
                .await?;
                replace_fts(conn, *id, rows.title, &entry.text).await?;
            }
            None => {
                upsert_doc(
                    conn,
                    NewDoc {
                        book_id: rows.book_id,
                        track_id: rows.track_id,
                        kind: rows.kind,
                        ref_id: rows.ref_id,
                        ref_part: index as i32,
                        title: rows.title,
                        body: &entry.text,
                        meta: &entry.meta,
                    },
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// The searchable body of a knowledge chunk: its heading path plus the text, so a section name
/// is findable by keyword as well as through the vector (spec/07 §1.1, §2).
fn knowledge_fts_body(heading: &str, text: &str) -> String {
    if heading.is_empty() {
        text.to_owned()
    } else {
        format!("{heading}\n{text}")
    }
}

async fn replace_knowledge_fts(
    conn: &impl ConnectionTrait,
    id: i32,
    title: &str,
    body: &str,
) -> Result<(), AppError> {
    let title_tokens = fts::segment(title);
    let body_tokens = fts::segment(body);
    if let Some(row) = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "SELECT title, body FROM fts_knowledge WHERE rowid = ?",
            vec![(id as i64).into()],
        ))
        .await?
    {
        let current_title: String = row.try_get("", "title")?;
        let current_body: String = row.try_get("", "body")?;
        if current_title == title_tokens && current_body == body_tokens {
            return Ok(());
        }
    }
    conn.execute_unprepared(&format!("DELETE FROM fts_knowledge WHERE rowid = {id}"))
        .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Sqlite,
        "INSERT INTO fts_knowledge(rowid, title, body) VALUES (?, ?, ?)",
        vec![(id as i64).into(), title_tokens.into(), body_tokens.into()],
    ))
    .await?;
    Ok(())
}

/// Applies every indexable `canon.changed` event of one transaction (spec/07 §2). Called inside
/// the transaction that wrote the change, from `services/canon::write` and the agent runtime.
pub async fn sync_events(
    conn: &impl ConnectionTrait,
    events: &[ServerEvent],
) -> Result<(), AppError> {
    for event in events {
        let ServerEvent::CanonChanged {
            book_id,
            target_type,
            target_id,
            ..
        } = event
        else {
            continue;
        };
        match target_type {
            ChangeTargetType::Entity => sync_entity(conn, *book_id, *target_id).await?,
            ChangeTargetType::MemoryNote => sync_note(conn, *book_id, *target_id).await?,
            ChangeTargetType::Foreshadowing => {
                sync_foreshadowing(conn, *book_id, *target_id).await?
            }
            ChangeTargetType::TimelineEvent => {
                sync_timeline_event(conn, *book_id, *target_id).await?
            }
            ChangeTargetType::ChapterSummary => sync_chapter_summary(conn, *target_id).await?,
            ChangeTargetType::Relation
            | ChangeTargetType::AppearanceSet
            | ChangeTargetType::SettingDoc
            | ChangeTargetType::CharacterDesign => {}
        }
    }
    Ok(())
}

/// Quotes a Rust string as a SQLite string literal.
fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::json::{EntityKind, EntityStatus};
    use std::collections::BTreeMap;

    #[test]
    fn entity_body_follows_the_spec_field_order() {
        let row = entities::Model {
            id: 1,
            book_id: 1,
            track_id: None,
            kind: EntityKind::Character,
            name: "Lin Fan".to_owned(),
            aliases: JsonText(vec!["Fan".to_owned()]),
            summary: "the protagonist".to_owned(),
            description: "long description".to_owned(),
            state: "qi refining".to_owned(),
            attributes: JsonText(BTreeMap::from([("age".to_owned(), "17".to_owned())])),
            importance: 5,
            status: EntityStatus::Active,
            is_protagonist: true,
            first_chapter_id: None,
            last_chapter_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let body = entity_body(&row).expect("body");
        assert_eq!(
            body,
            "the protagonist · Fan · qi refining · age: 17 · long description"
        );
    }
}
