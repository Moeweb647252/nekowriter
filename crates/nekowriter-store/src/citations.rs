//! Verifying citations against the text they quote (`docs/product-spec.md` §18.4).

use nekowriter_core::{
    body::{self, EditResult},
    call::CallRequest,
    citation::{
        ChapterLabel, Citation, CitationProblem, CitationSource, CitationTarget, MAX_CITATIONS,
        VerifiedCitation, check_quote,
    },
    document::DocumentRole,
    ids::{ChapterId, VersionId},
};
use sea_orm::{ConnectionTrait, EntityTrait};

use crate::{
    StoreError,
    chapters::{chapter_text, find_chapter},
    digest::sha256_hex,
    entity::{
        document, revision, run,
        tool_call::{self, ToolCallStatus},
    },
};

/// Checks each citation against its text in `version` and says where it comes from.
pub(crate) async fn verify_citations<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
    citations: &[Citation],
) -> Result<Vec<VerifiedCitation>, StoreError> {
    if citations.len() > MAX_CITATIONS {
        return Err(StoreError::CitationInvalid(
            CitationProblem::LimitExceeded {
                max: u32::try_from(MAX_CITATIONS).unwrap_or(u32::MAX),
            },
        ));
    }
    let mut verified = Vec::with_capacity(citations.len());
    for citation in citations {
        let (text, source) = cited_text(db, version_id, &citation.target).await?;
        check_quote(&text, citation).map_err(StoreError::CitationInvalid)?;
        verified.push(VerifiedCitation {
            citation: citation.clone(),
            source,
        });
    }
    Ok(verified)
}

/// The text a citation target names, if it belongs to `version`.
async fn cited_text<C: ConnectionTrait>(
    db: &C,
    version_id: VersionId,
    target: &CitationTarget,
) -> Result<(String, CitationSource), StoreError> {
    let gone = || StoreError::CitationInvalid(CitationProblem::TargetGone);
    match target {
        CitationTarget::Revision { revision_id } => {
            let revision = revision::Entity::find_by_id(*revision_id)
                .one(db)
                .await?
                .ok_or_else(gone)?;
            let document = document::Entity::find_by_id(revision.document_id)
                .one(db)
                .await?
                .ok_or_else(gone)?;
            if document.version_id != version_id {
                return Err(gone());
            }
            let chapter = match document.chapter_id {
                Some(id) => {
                    let chapter = find_chapter(db, id).await?;
                    Some(ChapterLabel {
                        kind: chapter.kind,
                        number: chapter.number,
                        title: chapter.title,
                    })
                }
                None => None,
            };
            let source = CitationSource::Document {
                role: document.role,
                chapter,
            };
            Ok((revision.content, source))
        }
        CitationTarget::Proposal {
            call_id,
            candidate_hash,
        } => {
            let call = tool_call::Entity::find_by_id(*call_id)
                .one(db)
                .await?
                .filter(|call| call.status == ToolCallStatus::AwaitingReview)
                .ok_or_else(gone)?;
            let chapter_id = run::Entity::find_by_id(call.run_id)
                .one(db)
                .await?
                .filter(|run| run.version_id == version_id)
                .and_then(|run| run.chapter_id)
                .ok_or_else(gone)?;
            let candidate = proposal_candidate(db, chapter_id, &call).await?;
            if &sha256_hex([candidate.as_bytes()]) != candidate_hash {
                return Err(gone());
            }
            Ok((candidate, CitationSource::Proposal))
        }
    }
}

/// The body a pending proposal of the chapter's body would produce. The body cannot change
/// while the proposal waits, so the proposal always changes it.
pub(crate) async fn proposal_candidate<C: ConnectionTrait>(
    db: &C,
    chapter_id: ChapterId,
    call: &tool_call::Model,
) -> Result<String, StoreError> {
    let corrupt = || StoreError::Corrupt(format!("proposal {} does not change the body", call.id));
    let Some(CallRequest::Edit { edit }) = &call.parsed else {
        return Err(corrupt());
    };
    let base = chapter_text(db, chapter_id, DocumentRole::ChapterBody).await?;
    match body::apply(&base, edit) {
        Ok(EditResult::Changed(candidate)) => Ok(candidate),
        Ok(EditResult::NoChange) | Err(_) => Err(corrupt()),
    }
}
