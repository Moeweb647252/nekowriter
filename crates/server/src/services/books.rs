//! Book service: create (with the main track and the three setting documents), read, patch
//! (archive), delete, and the overview (spec/08 §1.3, §5, §7; spec/02 §5 invariants 1–2).

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::BTreeMap;

use crate::api::dto::books::{
    BookDto, BookOverviewDto, BookPatch, BookProgressDto, BookSummaryDto, ChapterPointerDto,
    CreateBookResponse, StatusCountDto, TrackProgressDto,
};
use crate::db::entities::{books, chapters, setting_docs, tracks};
use crate::db::json::{
    AnchorKind, BookSettings, BookStatus, CanonMode, CanonStatus, ChapterStatus, ConceptionState,
    RoleModels, TrackKind, Usage,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{BookOp, ServerEvent};

use super::app_settings;
use super::{conflict, json, now, parse_json};

/// The three setting documents every book has (spec/02 §5 invariant 2).
pub const SETTING_DOC_KINDS: [&str; 3] = ["outline", "world", "protagonist"];

/// `POST /api/books` (spec/08 §5). The book starts `active`; the conception studio arrives in M9,
/// so `start_conception` currently only produces a warning.
pub async fn create(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    input: crate::api::dto::books::BookInput,
) -> Result<CreateBookResponse, AppError> {
    let (response, published) = super::write(db, move |tx| {
        Box::pin(async move { create_inner(tx, input).await })
    })
    .await?;
    events.publish_all(published);
    Ok(response)
}

/// The inner create; runs inside the caller's transaction.
pub async fn create_inner(
    tx: &impl ConnectionTrait,
    input: crate::api::dto::books::BookInput,
) -> Result<(CreateBookResponse, Vec<ServerEvent>), AppError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::validation("a book needs a title"));
    }
    let language = input.writing_language.trim();
    if language.is_empty() {
        return Err(AppError::validation("a writing language is required"));
    }
    validate_language(language)?;

    let settings = app_settings::book_defaults(tx).await?;
    let timestamp = now();
    let model = books::ActiveModel {
        title: Set(title.to_owned()),
        idea: Set(input.idea.trim().to_owned()),
        genre: Set(input.genre.trim().to_owned()),
        writing_language: Set(language.to_owned()),
        // M9 turns `start_conception` into a conceiving book plus a session.
        status: Set(BookStatus::Active.as_str().to_owned()),
        settings: Set(json(&settings)?),
        model_overrides: Set(json(&RoleModels::default())?),
        conception: Set(json(&ConceptionState::default())?),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    let book = books::Entity::insert(model).exec_with_returning(tx).await?;

    // Invariant 1: exactly one main track, created with the book (spec/02 §5).
    let main_track = tracks::ActiveModel {
        book_id: Set(book.id),
        kind: Set(TrackKind::Main.as_str().to_owned()),
        title: Set(String::new()),
        synopsis: Set(String::new()),
        canon_mode: Set(CanonMode::Canon.as_str().to_owned()),
        anchor_kind: Set(AnchorKind::None.as_str().to_owned()),
        anchor_chapter_id: Set(None),
        focus_entity_id: Set(None),
        ord: Set(0),
        created_at: Set(timestamp),
        updated_at: Set(timestamp),
        ..Default::default()
    };
    tracks::Entity::insert(main_track).exec(tx).await?;

    // Invariant 2: exactly three setting documents, empty (spec/02 §5).
    for kind in SETTING_DOC_KINDS {
        let doc = setting_docs::ActiveModel {
            book_id: Set(book.id),
            kind: Set(kind.to_owned()),
            content: Set(String::new()),
            revision: Set(0),
            approved_revision: Set(None),
            updated_at: Set(timestamp),
            ..Default::default()
        };
        setting_docs::Entity::insert(doc).exec(tx).await?;
    }

    let dto = to_dto(&book)?;
    let mut warnings = Vec::new();
    if input.start_conception {
        // M9 implements the studio; the book exists and can be written by hand meanwhile.
        warnings.push("conception.unavailable".to_owned());
    }
    Ok((
        CreateBookResponse {
            book: dto,
            warnings,
        },
        vec![ServerEvent::BookUpdated {
            book_id: book.id,
            op: BookOp::Create,
        }],
    ))
}

/// `GET /api/books?status=` (spec/10 §2.2). Archived books are hidden unless requested.
pub async fn list(
    conn: &impl ConnectionTrait,
    status: Option<BookStatus>,
) -> Result<Vec<BookSummaryDto>, AppError> {
    let mut query = books::Entity::find().order_by_desc(books::Column::UpdatedAt);
    if let Some(status) = status {
        query = query.filter(books::Column::Status.eq(status.as_str()));
    }
    let rows = query.all(conn).await?;
    let mut summaries = Vec::with_capacity(rows.len());
    for row in rows {
        summaries.push(to_summary(conn, row).await?);
    }
    Ok(summaries)
}

/// `GET /api/books/:id`.
pub async fn get(conn: &impl ConnectionTrait, book_id: i32) -> Result<BookDto, AppError> {
    let row = books::Entity::find_by_id(book_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book #{book_id}")))?;
    to_dto(&row)
}

/// `PATCH /api/books/:id` (title, genre, writing language, settings, model overrides, archive).
pub async fn update(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
    patch: BookPatch,
) -> Result<BookDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { update_inner(tx, book_id, patch).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn update_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
    patch: BookPatch,
) -> Result<(BookDto, Vec<ServerEvent>), AppError> {
    let existing = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::not_found(format!("book #{book_id}")))?;
    let current_status = super::enum_of::<BookStatus>("books.status", &existing.status)?;

    let mut active = books::ActiveModel {
        id: Set(book_id),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Some(title) = &patch.title {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::validation("a book needs a title"));
        }
        active.title = Set(title.to_owned());
    }
    if let Some(genre) = &patch.genre {
        active.genre = Set(genre.trim().to_owned());
    }
    if let Some(language) = &patch.writing_language {
        let language = language.trim();
        validate_language(language)?;
        active.writing_language = Set(language.to_owned());
    }
    if let Some(settings) = &patch.settings {
        validate_settings(settings)?;
        active.settings = Set(json(settings)?);
    }
    if let Some(overrides) = &patch.model_overrides {
        active.model_overrides = Set(json(overrides)?);
    }
    if let Some(status) = patch.status {
        // Archiving only hides the book; unarchiving sets it back to active (spec/08 §1.3).
        match status {
            BookStatus::Archived => {
                if current_status != BookStatus::Active {
                    return Err(conflict(
                        ErrorCode::BookInvalidState,
                        "only an active book can be archived",
                        serde_json::json!({ "status": current_status.as_str() }),
                    ));
                }
            }
            BookStatus::Active => {
                if current_status != BookStatus::Archived {
                    return Err(conflict(
                        ErrorCode::BookInvalidState,
                        "only an archived book can be unarchived",
                        serde_json::json!({ "status": current_status.as_str() }),
                    ));
                }
            }
            BookStatus::Conceiving => {
                // `conceiving` is set by the conception studio, never by a patch (spec/08 §5).
                return Err(AppError::validation(
                    "status can only be set to archived or active",
                ));
            }
        }
        active.status = Set(status.as_str().to_owned());
    }
    active.update(tx).await?;

    let row = books::Entity::find_by_id(book_id)
        .one(tx)
        .await?
        .ok_or_else(|| AppError::internal("the book vanished during the update"))?;
    Ok((
        to_dto(&row)?,
        vec![ServerEvent::BookUpdated {
            book_id,
            op: BookOp::Update,
        }],
    ))
}

/// `DELETE /api/books/:id`: cascades to tracks, chapters, setting materials and canon.
pub async fn delete(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    book_id: i32,
) -> Result<(), AppError> {
    let published = super::write(db, move |tx| {
        Box::pin(async move { delete_inner(tx, book_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(())
}

pub async fn delete_inner(
    tx: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<ServerEvent>, AppError> {
    let existing = books::Entity::find_by_id(book_id).one(tx).await?;
    if existing.is_none() {
        return Err(AppError::not_found(format!("book #{book_id}")));
    }
    books::Entity::delete_by_id(book_id).exec(tx).await?;
    Ok(vec![ServerEvent::BookUpdated {
        book_id,
        op: BookOp::Delete,
    }])
}

/// `GET /api/books/:id/overview` (spec/08 §7). M1 fills `progress` and `recent_changes`.
pub async fn overview(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BookOverviewDto, AppError> {
    let book = get(conn, book_id).await?;
    let progress = progress(conn, book_id).await?;
    let recent_changes = super::change_log::recent_for_book(conn, book_id, 10).await?;
    Ok(BookOverviewDto {
        book,
        progress,
        recent_changes,
        usage: Usage::default(),
    })
}

/// Chapter counts and words per track (spec/08 §7 `progress`).
pub async fn progress(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<BookProgressDto, AppError> {
    let track_rows = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .order_by_asc(tracks::Column::Ord)
        .all(conn)
        .await?;
    let chapter_rows = chapters::Entity::find()
        .filter(chapters::Column::BookId.eq(book_id))
        .all(conn)
        .await?;

    let mut per_track: BTreeMap<i32, Vec<&chapters::Model>> = BTreeMap::new();
    for chapter in &chapter_rows {
        per_track.entry(chapter.track_id).or_default().push(chapter);
    }

    let mut tracks_progress = Vec::with_capacity(track_rows.len());
    let mut total_chapters = 0i32;
    let mut official_chapters = 0i32;
    let mut total_words = 0i32;
    let mut official_words = 0i32;
    for track in &track_rows {
        let rows = per_track.get(&track.id).cloned().unwrap_or_default();
        let mut by_status: Vec<StatusCountDto> = Vec::new();
        for status in [
            ChapterStatus::Planned,
            ChapterStatus::Drafting,
            ChapterStatus::InReview,
            ChapterStatus::Revising,
            ChapterStatus::Official,
        ] {
            let count = rows
                .iter()
                .filter(|chapter| chapter.status == status.as_str())
                .count() as i32;
            by_status.push(StatusCountDto { status, count });
            if status == ChapterStatus::Official {
                official_chapters += count;
            }
        }
        let words: i32 = rows.iter().map(|chapter| chapter.word_count).sum();
        let official: i32 = rows
            .iter()
            .filter(|chapter| chapter.status == ChapterStatus::Official.as_str())
            .map(|chapter| chapter.word_count)
            .sum();
        total_chapters += rows.len() as i32;
        total_words += words;
        official_words += official;
        tracks_progress.push(TrackProgressDto {
            track_id: track.id,
            kind: super::enum_of("tracks.kind", &track.kind)?,
            title: track.title.clone(),
            chapters: rows.len() as i32,
            words,
            by_status,
        });
    }
    Ok(BookProgressDto {
        total_chapters,
        official_chapters,
        total_words,
        official_words,
        tracks: tracks_progress,
    })
}

/// Builds one library card.
async fn to_summary(
    conn: &impl ConnectionTrait,
    row: books::Model,
) -> Result<BookSummaryDto, AppError> {
    let chapter_rows = chapters::Entity::find()
        .filter(chapters::Column::BookId.eq(row.id))
        .all(conn)
        .await?;
    let main_track = tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(row.id))
        .filter(tracks::Column::Kind.eq(TrackKind::Main.as_str()))
        .one(conn)
        .await?;
    let chapter_count = chapter_rows.len() as i32;
    let official_chapter_count = chapter_rows
        .iter()
        .filter(|chapter| chapter.status == ChapterStatus::Official.as_str())
        .count() as i32;
    let word_count: i32 = chapter_rows.iter().map(|chapter| chapter.word_count).sum();
    let latest_chapter = main_track.and_then(|track| {
        chapter_rows
            .iter()
            .filter(|chapter| chapter.track_id == track.id)
            .max_by_key(|chapter| chapter.ord)
            .map(|chapter| ChapterPointerDto {
                id: chapter.id,
                number: chapter.ord,
                title: chapter.title.clone(),
                status: ChapterStatus::from_str_opt(&chapter.status)
                    .unwrap_or(ChapterStatus::Planned),
            })
    });
    // Sessions arrive in M3; the count is real from then on.
    let in_review = chapter_rows
        .iter()
        .filter(|chapter| chapter.status == ChapterStatus::InReview.as_str())
        .count() as i32;
    Ok(BookSummaryDto {
        id: row.id,
        title: row.title,
        genre: row.genre,
        writing_language: row.writing_language,
        status: super::enum_of("books.status", &row.status)?,
        chapter_count,
        official_chapter_count,
        word_count,
        latest_chapter,
        attention_count: in_review,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Maps a book row to its DTO.
pub fn to_dto(row: &books::Model) -> Result<BookDto, AppError> {
    Ok(BookDto {
        id: row.id,
        title: row.title.clone(),
        idea: row.idea.clone(),
        genre: row.genre.clone(),
        writing_language: row.writing_language.clone(),
        status: super::enum_of("books.status", &row.status)?,
        settings: parse_json("books.settings", &row.settings)?,
        model_overrides: parse_json("books.model_overrides", &row.model_overrides)?,
        conception: parse_json("books.conception", &row.conception)?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Rejects settings the writer service could not honor.
pub fn validate_settings(settings: &BookSettings) -> Result<(), AppError> {
    if settings.target_words_min <= 0 {
        return Err(AppError::validation("target_words_min must be positive"));
    }
    if settings.target_words_max < settings.target_words_min {
        return Err(AppError::validation(
            "target_words_max must be at least target_words_min",
        ));
    }
    if settings.recent_summary_count < 0 || settings.midterm_window < 0 {
        return Err(AppError::validation("memory windows must not be negative"));
    }
    Ok(())
}

/// Validates a BCP-47 writing language tag: a 2–3 letter primary subtag and optional subtags.
pub fn validate_language(language: &str) -> Result<(), AppError> {
    let valid = !language.is_empty()
        && language.len() <= 35
        && language.split('-').all(|part| {
            !part.is_empty() && part.len() <= 8 && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
        && language
            .split('-')
            .next()
            .is_some_and(|primary| (2..=3).contains(&primary.len()));
    if valid {
        Ok(())
    } else {
        Err(AppError::validation(format!(
            "writing_language must be a BCP-47 tag such as zh-CN or en, got {language:?}"
        )))
    }
}

/// The three setting documents of a book, in the spec's order.
pub async fn setting_docs_of(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Vec<setting_docs::Model>, AppError> {
    let mut rows = setting_docs::Entity::find()
        .filter(setting_docs::Column::BookId.eq(book_id))
        .all(conn)
        .await?;
    rows.sort_by_key(|row| {
        SETTING_DOC_KINDS
            .iter()
            .position(|kind| *kind == row.kind)
            .unwrap_or(usize::MAX)
    });
    Ok(rows)
}

/// Updates a book row's `updated_at` in the caller's transaction.
pub async fn touch(
    tx: &impl ConnectionTrait,
    book_id: i32,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    use sea_orm::ActiveModelTrait;
    let mut active = books::ActiveModel {
        id: Set(book_id),
        ..Default::default()
    };
    active.updated_at = Set(timestamp);
    active.update(tx).await?;
    Ok(())
}

/// The canonical status of a chapter, for services that only carry the string column.
pub fn canon_status_of(raw: &str) -> CanonStatus {
    CanonStatus::from_str_opt(raw).unwrap_or(CanonStatus::None)
}

/// Loads the main track of a book.
pub async fn main_track(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<tracks::Model, AppError> {
    tracks::Entity::find()
        .filter(tracks::Column::BookId.eq(book_id))
        .filter(tracks::Column::Kind.eq(TrackKind::Main.as_str()))
        .one(conn)
        .await?
        .ok_or_else(|| AppError::internal(format!("book #{book_id} has no main track")))
}

/// Selects the highest `ord` of a track's chapters (0 when it has none).
pub async fn last_ord(conn: &impl ConnectionTrait, track_id: i32) -> Result<i32, AppError> {
    let row = chapters::Entity::find()
        .filter(chapters::Column::TrackId.eq(track_id))
        .order_by_desc(chapters::Column::Ord)
        .select_only()
        .column(chapters::Column::Ord)
        .into_tuple::<i32>()
        .one(conn)
        .await?;
    Ok(row.unwrap_or(0))
}
