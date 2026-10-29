//! Setting proposals (spec/06 §5, spec/10 §2.3): the archivist proposes, the author decides.
//!
//! A proposal stores the edits exactly. Accept re-checks every `find` against the **current** text
//! in one transaction: when a `find` no longer matches (or the target is gone, or a new
//! character's name was taken meanwhile) the proposal becomes `conflict` and nothing is applied
//! (D-013, D-044). Reject dismisses a `pending` or `conflict` proposal.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;

use crate::api::dto::proposals::{ProposalDto, ProposalPreviewDto};
use crate::api::dto::settings_materials::{CharacterInput, CharacterPatch};
use crate::db::entities::{character_designs, setting_docs, setting_proposals};
use crate::db::json::{
    ChangeActor, ChangeOp, ChangeTargetType, FindReplace, ProposalEdits, ProposalStatus,
    ProposalTargetType, SettingDocKind, Snapshot,
};
use crate::error::{AppError, ErrorCode};
use crate::events::{ServerEvent, SettingsKind};

use super::now;

/// A proposal to create, before serialization (the tool's transaction calls this).
pub struct NewProposal<'a> {
    pub book_id: i32,
    pub target_type: ProposalTargetType,
    pub target_kind: Option<SettingDocKind>,
    pub target_id: Option<i32>,
    pub edits: &'a ProposalEdits,
    pub rationale: &'a str,
    pub chapter_id: Option<i32>,
    pub session_id: Option<i32>,
}

/// Inserts a `pending` proposal inside the caller's transaction (spec/06 §5).
pub async fn create_inner(
    tx: &impl ConnectionTrait,
    proposal: NewProposal<'_>,
) -> Result<setting_proposals::Model, AppError> {
    let edits = crate::services::json_text(proposal.edits.clone());
    Ok(setting_proposals::ActiveModel {
        book_id: Set(proposal.book_id),
        target_type: Set(proposal.target_type),
        target_kind: Set(proposal.target_kind),
        target_id: Set(proposal.target_id),
        edits: Set(edits),
        rationale: Set(proposal.rationale.trim().to_owned()),
        status: Set(ProposalStatus::Pending),
        chapter_id: Set(proposal.chapter_id),
        session_id: Set(proposal.session_id),
        created_at: Set(now()),
        ..Default::default()
    }
    .insert(tx)
    .await?)
}

/// `GET /api/books/:id/proposals` (spec/10 §2.3).
pub async fn list(
    conn: &impl ConnectionTrait,
    book_id: i32,
    status: Option<ProposalStatus>,
) -> Result<Vec<ProposalDto>, AppError> {
    super::books::get(conn, book_id).await?;
    let mut statement = setting_proposals::Entity::find()
        .filter(setting_proposals::COLUMN.book_id.eq(book_id))
        .order_by_desc(setting_proposals::COLUMN.id);
    if let Some(status) = status {
        statement = statement.filter(setting_proposals::COLUMN.status.eq(status.as_str()));
    }
    let mut items = Vec::new();
    for row in statement.all(conn).await? {
        items.push(to_dto(conn, &row).await?);
    }
    Ok(items)
}

/// `POST /api/proposals/:id/accept` (spec/06 §5, spec/10 §2.3).
pub async fn accept(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    proposal_id: i32,
) -> Result<ProposalDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { accept_inner(tx, proposal_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn accept_inner(
    tx: &sea_orm::DatabaseTransaction,
    proposal_id: i32,
) -> Result<(ProposalDto, Vec<ServerEvent>), AppError> {
    let proposal = load(tx, proposal_id).await?;
    if proposal.status != ProposalStatus::Pending {
        return Err(invalid_state(&proposal, "accept"));
    }
    let book_id = proposal.book_id;
    let mut events = Vec::new();
    let mut applied = true;

    match target_type_of(&proposal) {
        ProposalTargetType::SettingDoc => {
            let Some(kind) = proposal.target_kind else {
                return Err(AppError::internal(
                    "a setting proposal has no document kind",
                ));
            };
            let doc = setting_docs::Entity::find()
                .filter(setting_docs::COLUMN.book_id.eq(book_id))
                .filter(setting_docs::COLUMN.kind.eq(kind.as_str()))
                .one(tx)
                .await?
                .ok_or_else(|| AppError::internal("the proposed setting document is gone"))?;
            let edits = edits_of(&proposal)?;
            match apply_edits(&doc.content, &edits) {
                Ok(content) => {
                    let before = Snapshot::SettingDoc {
                        content: doc.content.clone(),
                    };
                    let after = Snapshot::SettingDoc {
                        content: content.clone(),
                    };
                    let revision = doc.revision + 1;
                    setting_docs::ActiveModel {
                        id: Set(doc.id),
                        content: Set(content),
                        revision: Set(revision),
                        updated_at: Set(now()),
                        ..Default::default()
                    }
                    .update(tx)
                    .await?;
                    super::change_log::record(
                        tx,
                        super::change_log::NewChange {
                            book_id,
                            track_id: None,
                            chapter_id: proposal.chapter_id,
                            session_id: None,
                            actor: ChangeActor::User,
                            target_type: ChangeTargetType::SettingDoc,
                            target_id: doc.id,
                            op: ChangeOp::Update,
                            before: Some(&before),
                            after: Some(&after),
                            note: &proposal.rationale,
                            reverts_id: None,
                        },
                    )
                    .await?;
                    super::books::touch(tx, book_id, now()).await?;
                    events.push(ServerEvent::SettingsChanged {
                        book_id,
                        kind: SettingsKind::SettingDoc,
                        doc_kind: Some(kind),
                        revision: Some(revision),
                    });
                }
                Err(_) => applied = false,
            }
        }
        ProposalTargetType::CharacterDesign => {
            let Some(design_id) = proposal.target_id else {
                return Err(AppError::internal("a character proposal has no design"));
            };
            let design = character_designs::Entity::find_by_id(design_id)
                .one(tx)
                .await?;
            let Some(design) = design else {
                // The design was deleted or renamed away: nothing can be applied (spec/06 §5).
                return mark_conflict(tx, proposal, events).await;
            };
            let edits = edits_of(&proposal)?;
            match apply_edits(&design.profile, &edits) {
                Ok(profile) => {
                    let (_, mut changed) = super::characters::update_inner(
                        tx,
                        design_id,
                        CharacterPatch {
                            profile: Some(profile),
                            note: proposal.rationale.clone(),
                            ..CharacterPatch::default()
                        },
                    )
                    .await?;
                    events.append(&mut changed);
                }
                Err(_) => applied = false,
            }
        }
        ProposalTargetType::NewCharacter => {
            let ProposalEdits::CreateCharacter { character } = edits_of(&proposal)? else {
                return Err(AppError::internal(
                    "a new-character proposal has no character",
                ));
            };
            match super::characters::create_inner(
                tx,
                book_id,
                CharacterInput {
                    name: character.name.clone(),
                    role_label: character.role_label.clone(),
                    summary: character.summary.clone(),
                    profile: character.profile.clone(),
                    note: proposal.rationale.clone(),
                },
            )
            .await
            {
                Ok((_, mut changed)) => events.append(&mut changed),
                Err(error) if error.code == ErrorCode::ConflictExists => applied = false,
                Err(error) => return Err(error),
            }
        }
    }

    if !applied {
        return mark_conflict(tx, proposal, events).await;
    }
    setting_proposals::ActiveModel {
        id: Set(proposal.id),
        status: Set(ProposalStatus::Accepted),
        decided_at: Set(Some(now())),
        ..Default::default()
    }
    .update(tx)
    .await?;
    events.push(ServerEvent::ProposalChanged {
        book_id,
        proposal_id: proposal.id,
        status: ProposalStatus::Accepted,
    });
    let row = load(tx, proposal.id).await?;
    Ok((to_dto(tx, &row).await?, events))
}

/// Marks the proposal `conflict`, applies nothing, and returns it (spec/06 §5, D-044).
async fn mark_conflict(
    tx: &sea_orm::DatabaseTransaction,
    proposal: setting_proposals::Model,
    mut events: Vec<ServerEvent>,
) -> Result<(ProposalDto, Vec<ServerEvent>), AppError> {
    setting_proposals::ActiveModel {
        id: Set(proposal.id),
        status: Set(ProposalStatus::Conflict),
        decided_at: Set(Some(now())),
        ..Default::default()
    }
    .update(tx)
    .await?;
    events.push(ServerEvent::ProposalChanged {
        book_id: proposal.book_id,
        proposal_id: proposal.id,
        status: ProposalStatus::Conflict,
    });
    let row = load(tx, proposal.id).await?;
    Ok((to_dto(tx, &row).await?, events))
}

/// `POST /api/proposals/:id/reject` (spec/06 §5): allowed from `pending` or `conflict`.
pub async fn reject(
    db: &crate::db::Db,
    events: &crate::events::EventBus,
    proposal_id: i32,
) -> Result<ProposalDto, AppError> {
    let (dto, published) = super::write(db, move |tx| {
        Box::pin(async move { reject_inner(tx, proposal_id).await })
    })
    .await?;
    events.publish_all(published);
    Ok(dto)
}

pub async fn reject_inner(
    tx: &sea_orm::DatabaseTransaction,
    proposal_id: i32,
) -> Result<(ProposalDto, Vec<ServerEvent>), AppError> {
    let proposal = load(tx, proposal_id).await?;
    let status = proposal.status;
    if !matches!(status, ProposalStatus::Pending | ProposalStatus::Conflict) {
        return Err(invalid_state(&proposal, "reject"));
    }
    setting_proposals::ActiveModel {
        id: Set(proposal.id),
        status: Set(ProposalStatus::Rejected),
        decided_at: Set(Some(now())),
        ..Default::default()
    }
    .update(tx)
    .await?;
    let row = load(tx, proposal.id).await?;
    Ok((
        to_dto(tx, &row).await?,
        vec![ServerEvent::ProposalChanged {
            book_id: proposal.book_id,
            proposal_id: proposal.id,
            status: ProposalStatus::Rejected,
        }],
    ))
}

/// Applies a list of find/replace edits to `text`; every `find` must occur exactly once in the
/// text as it stands when that edit runs (spec/06 §5).
pub fn apply_edits(text: &str, edits: &ProposalEdits) -> Result<String, (String, usize)> {
    match edits {
        ProposalEdits::Edits { edits } => apply_find_replace(text, edits),
        ProposalEdits::Append { text: append } => Ok(append_text(text, append)),
        // A new character has no target text to edit.
        ProposalEdits::CreateCharacter { .. } => Ok(text.to_owned()),
    }
}

/// Applies find/replace pairs in order, failing with `(find, count)` of the first mismatch.
pub fn apply_find_replace(text: &str, edits: &[FindReplace]) -> Result<String, (String, usize)> {
    let mut current = text.to_owned();
    for edit in edits {
        let count = current.matches(&edit.find).count();
        if count != 1 || edit.find.is_empty() {
            return Err((edit.find.clone(), count));
        }
        current = current.replacen(&edit.find, &edit.replace, 1);
    }
    Ok(current)
}

/// Appends a passage to a document, separated by a blank line (spec/06 §5 "append").
pub fn append_text(text: &str, append: &str) -> String {
    if text.trim().is_empty() {
        append.to_owned()
    } else {
        format!("{text}\n\n{append}")
    }
}

/// Maps a row to its DTO, computing the preview diff against the current target (spec/10 §2.3).
pub async fn to_dto(
    conn: &impl ConnectionTrait,
    row: &setting_proposals::Model,
) -> Result<ProposalDto, AppError> {
    let edits = edits_of(row)?;
    let (before, after) = match target_type_of(row) {
        ProposalTargetType::SettingDoc => {
            let kind = row.target_kind;
            match kind {
                Some(kind) => {
                    let doc = setting_docs::Entity::find()
                        .filter(setting_docs::COLUMN.book_id.eq(row.book_id))
                        .filter(setting_docs::COLUMN.kind.eq(kind.as_str()))
                        .one(conn)
                        .await?;
                    match doc {
                        Some(doc) => {
                            let after = apply_edits(&doc.content, &edits)
                                .unwrap_or_else(|_| doc.content.clone());
                            (doc.content, after)
                        }
                        None => (String::new(), String::new()),
                    }
                }
                None => (String::new(), String::new()),
            }
        }
        ProposalTargetType::CharacterDesign => {
            let design = match row.target_id {
                Some(design_id) => {
                    character_designs::Entity::find_by_id(design_id)
                        .one(conn)
                        .await?
                }
                None => None,
            };
            match design {
                Some(design) => {
                    let after = apply_edits(&design.profile, &edits)
                        .unwrap_or_else(|_| design.profile.clone());
                    (design.profile, after)
                }
                None => (String::new(), String::new()),
            }
        }
        ProposalTargetType::NewCharacter => match &edits {
            ProposalEdits::CreateCharacter { character } => (
                String::new(),
                format!("{}\n\n{}", character.summary, character.profile),
            ),
            _ => (String::new(), String::new()),
        },
    };
    Ok(ProposalDto {
        id: row.id,
        book_id: row.book_id,
        target_type: target_type_of(row),
        target_kind: row.target_kind,
        target_id: row.target_id,
        edits,
        rationale: row.rationale.clone(),
        status: row.status,
        chapter_id: row.chapter_id,
        session_id: row.session_id,
        created_at: row.created_at,
        decided_at: row.decided_at,
        preview: ProposalPreviewDto { before, after },
    })
}

/// Loads one proposal or reports `not_found`.
pub async fn load(
    conn: &impl ConnectionTrait,
    proposal_id: i32,
) -> Result<setting_proposals::Model, AppError> {
    setting_proposals::Entity::find_by_id(proposal_id)
        .one(conn)
        .await?
        .ok_or_else(|| AppError::not_found(format!("proposal {proposal_id}")))
}

/// The stored edits, decoded.
pub fn edits_of(row: &setting_proposals::Model) -> Result<ProposalEdits, AppError> {
    Ok(row.edits.clone().into_inner())
}

fn target_type_of(row: &setting_proposals::Model) -> ProposalTargetType {
    row.target_type
}

fn invalid_state(row: &setting_proposals::Model, operation: &str) -> AppError {
    AppError::new(
        ErrorCode::ProposalInvalidState,
        format!("the proposal cannot be {operation}ed in its state"),
    )
    .with_details(json!({
        "status": row.status,
        "operation": operation,
    }))
}
