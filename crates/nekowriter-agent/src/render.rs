//! Rendering stored data into model text.
//!
//! Tool results, review outcomes and corrections are stored as data and rendered here when
//! they are sealed into a request (`nekowriter_core::run_message`). A tool result is a JSON
//! object with a `status` and a model-facing `message`.

use nekowriter_core::{
    body::{BodyEdit, EditProblem},
    chapter::{ChapterInput, ChapterKind},
    citation::{CitationSource, VerifiedCitation},
    document::DocumentRole,
    field::FieldProblem,
    knowledge::SearchUnavailable,
    preparation::{ArtifactKind, ChapterPlan},
    project::WritingLanguage,
    question::QuestionProblem,
    review::ReviewKind,
    run_message::{
        ArgumentsProblem, Correction, FeedbackItem, JsonType, KnowledgeHit, ProtocolProblem,
        ToolOutcome, TranscriptMessage, UnsealedMessage,
    },
    stage::{FoldInput, FoldStage, ReviewSubject},
    submission::SubmissionProblem,
};
use serde_json::json;

use crate::{model_text::model_text, stage::ASK_USER};

/// The writing language, named in the model's language.
#[must_use]
pub fn language_name(language: WritingLanguage) -> String {
    let languages = &model_text().languages;
    match language {
        WritingLanguage::ZhCn => languages.zh_cn.render(&[]),
        WritingLanguage::En => languages.en.render(&[]),
    }
}

#[must_use]
pub fn artifact_name(kind: ArtifactKind) -> String {
    let artifacts = &model_text().artifacts;
    match kind {
        ArtifactKind::Background => artifacts.background.render(&[]),
        ArtifactKind::Research => artifacts.research.render(&[]),
        ArtifactKind::Characters => artifacts.characters.render(&[]),
        ArtifactKind::Outline => artifacts.outline.render(&[]),
    }
}

/// Artifact names joined into one phrase.
#[must_use]
pub fn artifact_list(kinds: impl IntoIterator<Item = ArtifactKind>) -> String {
    let names: Vec<_> = kinds.into_iter().map(artifact_name).collect();
    model_list(&names)
}

/// Names joined into one phrase.
#[must_use]
pub fn model_list(names: &[String]) -> String {
    names.join(&model_text().list.separator.render(&[]))
}

/// A chapter's name, such as its kind, number and title.
#[must_use]
pub fn chapter_name(chapter: &ChapterInput) -> String {
    let kinds = &model_text().chapter_kinds;
    let kind = match chapter.kind {
        ChapterKind::Main => kinds.main.render(&[]),
        ChapterKind::Extra => kinds.extra.render(&[]),
    };
    model_text().chapters.name.render(&[
        ("kind", &kind),
        ("number", &chapter.number.to_string()),
        ("title", &chapter.title),
    ])
}

/// The chapter's current body, as the body run's stage input ends with it.
#[must_use]
pub fn current_body(body: &str) -> String {
    let text = &model_text().chapters;
    if body.trim().is_empty() {
        text.empty_body.render(&[])
    } else {
        text.current_body.render(&[("body", body)])
    }
}

/// The folds of the completed stages as one text, in story order (§11); `None` before any
/// stage is completed.
#[must_use]
pub fn folds(folds: &[FoldInput]) -> Option<String> {
    if folds.is_empty() {
        return None;
    }
    let text = &model_text().folds;
    let separator = text.separator.render(&[]);
    let mut parts = vec![text.intro.render(&[])];
    for fold in folds {
        match &fold.stage {
            FoldStage::Preparation => {
                parts.push(text.preparation.render(&[]));
                for document in &fold.documents {
                    if let Some(kind) = artifact_of(document.role) {
                        parts.push(text.document.render(&[("name", &artifact_name(kind))]));
                    }
                    parts.push(document.content.clone());
                    if let Some(plan) = &document.chapter_plan {
                        parts.push(text.chapter_plan.render(&[]));
                        parts.push(plan_text(plan));
                    }
                }
            }
            FoldStage::ChapterOutline { chapter } => {
                parts.push(
                    text.chapter_outline
                        .render(&[("chapter", &chapter_name(chapter))]),
                );
                parts.extend(
                    fold.documents
                        .iter()
                        .map(|document| document.content.clone()),
                );
            }
            FoldStage::ChapterBody { chapter } => {
                parts.push(
                    text.chapter_body
                        .render(&[("chapter", &chapter_name(chapter))]),
                );
                parts.extend(
                    fold.documents
                        .iter()
                        .map(|document| document.content.clone()),
                );
            }
        }
    }
    Some(parts.join(&separator))
}

fn artifact_of(role: DocumentRole) -> Option<ArtifactKind> {
    match role {
        DocumentRole::Background => Some(ArtifactKind::Background),
        DocumentRole::Research => Some(ArtifactKind::Research),
        DocumentRole::Characters => Some(ArtifactKind::Characters),
        DocumentRole::Outline => Some(ArtifactKind::Outline),
        DocumentRole::NovelBrief
        | DocumentRole::ChapterGuidance
        | DocumentRole::ChapterOutline
        | DocumentRole::ChapterBody => None,
    }
}

/// The chapter plan as numbered lines.
fn plan_text(plan: &ChapterPlan) -> String {
    let item = &model_text().folds.plan_item;
    plan.0
        .iter()
        .enumerate()
        .map(|(index, chapter)| {
            item.render(&[
                ("number", &(index + 1).to_string()),
                ("title", &chapter.title),
                ("goal", &chapter.goal),
            ])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A rejection's feedback with its citations, or the note that a review agent named no
/// issues (§10).
fn rejection_feedback(feedback: Option<&str>, citations: &[VerifiedCitation]) -> String {
    match (feedback, citations.is_empty()) {
        (None, true) => model_text().review.no_issues.render(&[]),
        (feedback, _) => with_citations(feedback.unwrap_or_default(), citations),
    }
}

/// Feedback text followed by the passages it quotes, each with where it comes from.
#[must_use]
pub fn with_citations(text: &str, citations: &[VerifiedCitation]) -> String {
    if citations.is_empty() {
        return text.to_owned();
    }
    let labels = &model_text().citations;
    let mut lines = Vec::with_capacity(citations.len() + 2);
    if !text.trim().is_empty() {
        lines.push(text.to_owned());
    }
    lines.push(labels.heading.render(&[]));
    for cited in citations {
        lines.push(labels.item.render(&[
            ("quote", &cited.citation.quote),
            ("source", &citation_source(&cited.source)),
        ]));
    }
    lines.join("\n")
}

fn citation_source(source: &CitationSource) -> String {
    let labels = &model_text().citations;
    let CitationSource::Document { role, chapter } = source else {
        return labels.proposal.render(&[]);
    };
    let chapter = chapter.as_ref().map_or_else(String::new, |chapter| {
        let kinds = &model_text().chapter_kinds;
        let kind = match chapter.kind {
            ChapterKind::Main => kinds.main.render(&[]),
            ChapterKind::Extra => kinds.extra.render(&[]),
        };
        model_text().chapters.name.render(&[
            ("kind", &kind),
            ("number", &chapter.number.to_string()),
            ("title", &chapter.title),
        ])
    });
    match role {
        DocumentRole::NovelBrief => labels.brief.render(&[]),
        DocumentRole::ChapterGuidance => labels.guidance.render(&[("chapter", &chapter)]),
        DocumentRole::ChapterOutline => labels.outline.render(&[("chapter", &chapter)]),
        DocumentRole::ChapterBody => labels.body.render(&[("chapter", &chapter)]),
        DocumentRole::Background
        | DocumentRole::Research
        | DocumentRole::Characters
        | DocumentRole::Outline => artifact_of(*role).map(artifact_name).unwrap_or_default(),
    }
}

/// What a review agent reviews, after its prompt: the subject and the user's notes.
#[must_use]
pub fn review_material(subject: &ReviewSubject, notes: Option<&FeedbackItem>) -> String {
    let text = &model_text().review_material;
    let mut parts = match subject {
        ReviewSubject::Artifact {
            artifact,
            brief,
            content,
            chapter_plan,
        } => {
            let mut parts = vec![
                text.brief.render(&[("brief", brief)]),
                text.artifact.render(&[
                    ("artifact", &artifact_name(*artifact)),
                    ("content", content),
                ]),
            ];
            if let Some(plan) = chapter_plan {
                parts.push(model_text().folds.chapter_plan.render(&[]));
                parts.push(plan_text(plan));
            }
            parts
        }
        ReviewSubject::ChapterOutline {
            chapter,
            guidance,
            outline,
        } => vec![
            text.guidance.render(&[("guidance", &non_empty(guidance))]),
            text.outline
                .render(&[("chapter", &chapter_name(chapter)), ("outline", outline)]),
        ],
        ReviewSubject::Chapter {
            chapter,
            body,
            summary,
        } => vec![
            text.chapter
                .render(&[("chapter", &chapter_name(chapter)), ("body", body)]),
            text.summary.render(&[("summary", &non_empty(summary))]),
        ],
        ReviewSubject::Edit {
            chapter: _,
            body,
            edit,
        } => vec![
            text.body.render(&[("body", &non_empty(body))]),
            match edit {
                BodyEdit::Append { content } => text.append.render(&[("content", content)]),
                BodyEdit::Replace { old_str, new_str } => text
                    .replace
                    .render(&[("old", old_str), ("new", &non_empty(new_str))]),
            },
        ],
    };
    if let Some(notes) = notes {
        let notes = with_citations(&notes.text, &notes.citations);
        parts.push(text.notes.render(&[("notes", &notes)]));
    }
    parts.join(&model_text().folds.separator.render(&[]))
}

/// The text, or the model's word for nothing.
fn non_empty(text: &str) -> String {
    if text.trim().is_empty() {
        model_text().chapters.none.render(&[])
    } else {
        text.to_owned()
    }
}

/// Renders a stored message for the request that first carries it.
#[must_use]
pub fn unsealed(message: &UnsealedMessage, submission_tool: &str) -> TranscriptMessage {
    match message {
        UnsealedMessage::ToolResult {
            provider_call_id,
            outcome,
            feedback,
        } => {
            let feedback: Vec<String> = feedback
                .iter()
                .map(|item| with_citations(&item.text, &item.citations))
                .collect();
            TranscriptMessage::Tool {
                provider_call_id: provider_call_id.clone(),
                content: tool_result(outcome, &feedback),
            }
        }
        UnsealedMessage::PreparationReviewOutcome { rejected, stale } => {
            let review = &model_text().review;
            let mut items: Vec<String> = rejected
                .iter()
                .map(|item| {
                    review.rejected.render(&[
                        ("artifact", &artifact_name(item.artifact)),
                        (
                            "feedback",
                            &rejection_feedback(item.feedback.as_deref(), &item.citations),
                        ),
                    ])
                })
                .collect();
            items.extend(
                stale
                    .iter()
                    .map(|kind| review.stale.render(&[("artifact", &artifact_name(*kind))])),
            );
            let items = items.join(&review.item_separator.render(&[]));
            TranscriptMessage::User {
                content: review.preparation_outcome.render(&[("items", &items)]),
            }
        }
        UnsealedMessage::ReviewRejected {
            kind,
            feedback,
            citations,
        } => {
            let feedback = rejection_feedback(feedback.as_deref(), citations);
            let review = &model_text().review;
            let text = match kind {
                ReviewKind::ChapterFinal => &review.chapter_rejected,
                // Preparation reviews are delivered per round; only chapter outlines and
                // chapters are rejected one at a time.
                ReviewKind::ChapterOutline
                | ReviewKind::ChapterEdit
                | ReviewKind::PreparationBackground
                | ReviewKind::PreparationResearch
                | ReviewKind::PreparationCharacters
                | ReviewKind::PreparationOutline => &review.outline_rejected,
            };
            TranscriptMessage::User {
                content: text.render(&[("feedback", &feedback)]),
            }
        }
        UnsealedMessage::Correction { correction } => {
            let corrections = &model_text().corrections;
            let content = match correction {
                Correction::NoToolCall => corrections
                    .no_tool_call
                    .render(&[("tool", submission_tool)]),
                Correction::ProtocolError => corrections.protocol_error.render(&[]),
                Correction::SubmissionRejected => corrections.submission_rejected.render(&[]),
            };
            TranscriptMessage::User { content }
        }
    }
}

/// A tool result with the user's feedback that travels with it, as a separate `feedback`
/// array that is never mixed into the message (§8.5).
#[must_use]
pub fn tool_result(outcome: &ToolOutcome, feedback: &[String]) -> String {
    let mut result: serde_json::Value =
        serde_json::from_str(&tool_outcome(outcome)).expect("tool outcomes are JSON objects");
    if !feedback.is_empty() {
        result["feedback"] = json!(feedback);
    }
    result.to_string()
}

/// A tool result: `{"status": ..., "message": ...}`, and the body's word count after an
/// edit.
#[must_use]
pub fn tool_outcome(outcome: &ToolOutcome) -> String {
    let results = &model_text().results;
    let (status, message) = match outcome {
        ToolOutcome::EditApplied { word_count, .. } => {
            let message = results
                .edit_applied
                .render(&[("word_count", &word_count.to_string())]);
            return json!({ "status": "applied", "message": message, "word_count": word_count })
                .to_string();
        }
        ToolOutcome::EditNoChange { word_count } => {
            let message = results
                .edit_no_change
                .render(&[("word_count", &word_count.to_string())]);
            return json!({ "status": "unchanged", "message": message, "word_count": word_count })
                .to_string();
        }
        ToolOutcome::EditRejected { problem } => (
            "rejected",
            results
                .edit_rejected
                .render(&[("problem", &edit_problem(problem))]),
        ),
        ToolOutcome::EditDeclined => ("rejected", results.edit_declined.render(&[])),
        ToolOutcome::Answered { answers } => {
            let answers: Vec<_> = answers
                .iter()
                .map(|answer| {
                    json!({
                        "question": answer.question,
                        "selected": answer.selected,
                        "text": answer.custom_text,
                    })
                })
                .collect();
            return json!({
                "status": "answered",
                "message": results.answered.render(&[]),
                "answers": answers,
            })
            .to_string();
        }
        ToolOutcome::QuestionRejected { problem } => (
            "rejected",
            results
                .question_rejected
                .render(&[("problem", &question_problem(problem))]),
        ),
        ToolOutcome::SubmissionAccepted => ("accepted", results.submission_accepted.render(&[])),
        ToolOutcome::SubmissionRejected { problem } => (
            "rejected",
            results
                .submission_rejected
                .render(&[("problem", &submission_problem(problem))]),
        ),
        ToolOutcome::ProtocolError { problem } => (
            "error",
            results
                .protocol_error
                .render(&[("problem", &protocol_problem(problem))]),
        ),
        ToolOutcome::KnowledgeFound {
            hits,
            remaining_budget,
        } => return knowledge_found(hits, *remaining_budget),
        ToolOutcome::KnowledgeFailed { failure: _ } => (
            "error",
            results.knowledge_failed.render(&[(
                "problem",
                &model_text().knowledge.embedding_failed.render(&[]),
            )]),
        ),
        ToolOutcome::KnowledgeUnavailable { reason } => (
            "rejected",
            results
                .knowledge_unavailable
                .render(&[("reason", &search_unavailable(reason))]),
        ),
        ToolOutcome::KnowledgeQueryInvalid { problem } => (
            "rejected",
            results
                .knowledge_query_invalid
                .render(&[("problem", &query_problem(problem))]),
        ),
        ToolOutcome::KnowledgeSpacesGone { names } => (
            "rejected",
            results
                .knowledge_spaces_gone
                .render(&[("spaces", &model_list(names))]),
        ),
        ToolOutcome::QuotaExhausted { budget } => (
            "rejected",
            results
                .quota_exhausted
                .render(&[("budget", &budget.to_string()), ("tool", ASK_USER)]),
        ),
    };
    json!({ "status": status, "message": message }).to_string()
}

/// The passages a knowledge query found, with the budget left in its scope.
fn knowledge_found(hits: &[KnowledgeHit], remaining: u32) -> String {
    let results = &model_text().results;
    let remaining = remaining.to_string();
    if hits.is_empty() {
        let message = results.knowledge_empty.render(&[("remaining", &remaining)]);
        return json!({ "status": "found", "message": message, "hits": [] }).to_string();
    }
    let hits: Vec<_> = hits
        .iter()
        .map(|hit| {
            json!({
                "title": hit.title,
                "space": hit.space,
                "text": hit.text,
                "relevance": f64::from(hit.relevance_permille) / 1000.0,
            })
        })
        .collect();
    let message = results.knowledge_found.render(&[("remaining", &remaining)]);
    json!({ "status": "found", "message": message, "hits": hits }).to_string()
}

fn search_unavailable(reason: &SearchUnavailable) -> String {
    let text = &model_text().knowledge;
    match reason {
        SearchUnavailable::NotConfigured => text.not_configured.render(&[]),
        SearchUnavailable::Indexing { .. } => text.indexing.render(&[]),
    }
}

fn query_problem(problem: &FieldProblem) -> String {
    let text = &model_text().knowledge;
    match problem {
        FieldProblem::TooLong { max_chars } => text
            .query_too_long
            .render(&[("max_chars", &max_chars.to_string())]),
        FieldProblem::ControlCharacter => text.query_control_character.render(&[]),
        FieldProblem::Empty => text.query_empty.render(&[]),
        FieldProblem::InvalidUrl | FieldProblem::OutOfRange { .. } => {
            text.query_invalid.render(&[])
        }
    }
}

#[must_use]
pub fn question_problem(problem: &QuestionProblem) -> String {
    let text = &model_text().question_problems;
    match problem {
        QuestionProblem::NoQuestions => text.no_questions.render(&[]),
        QuestionProblem::TooManyQuestions { max } => {
            text.too_many_questions.render(&[("max", &max.to_string())])
        }
        QuestionProblem::DuplicateQuestion { id } => text.duplicate_question.render(&[("id", id)]),
        QuestionProblem::EmptyTitle { id } => text.empty_title.render(&[("id", id)]),
        QuestionProblem::NoOptions { id } => text.no_options.render(&[("id", id)]),
        QuestionProblem::TooManyOptions { id, max } => text
            .too_many_options
            .render(&[("id", id), ("max", &max.to_string())]),
        QuestionProblem::DuplicateOption { id, option } => text
            .duplicate_option
            .render(&[("id", id), ("option", option)]),
        QuestionProblem::EmptyOption { id, option } => {
            text.empty_option.render(&[("id", id), ("option", option)])
        }
    }
}

#[must_use]
pub fn edit_problem(problem: &EditProblem) -> String {
    let text = &model_text().edit_problems;
    match problem {
        EditProblem::EmptyContent => text.empty_content.render(&[]),
        EditProblem::EmptyOldStr => text.empty_old_str.render(&[]),
        EditProblem::NoMatch => text.no_match.render(&[]),
        EditProblem::AmbiguousMatch { matches } => text
            .ambiguous_match
            .render(&[("matches", &matches.to_string())]),
        EditProblem::BodyTooLong { max_chars } => text
            .body_too_long
            .render(&[("max_chars", &max_chars.to_string())]),
        EditProblem::StaleBase => text.stale_base.render(&[]),
    }
}

#[must_use]
pub fn submission_problem(problem: &SubmissionProblem) -> String {
    let text = &model_text().submission_problems;
    let number = |number: &u32| number.to_string();
    match problem {
        SubmissionProblem::MissingArtifacts { kinds } => text
            .missing_artifacts
            .render(&[("artifacts", &artifact_list(kinds.iter().copied()))]),
        SubmissionProblem::UnexpectedArtifacts { kinds } => text
            .unexpected_artifacts
            .render(&[("artifacts", &artifact_list(kinds.iter().copied()))]),
        SubmissionProblem::DuplicateArtifact { artifact } => text
            .duplicate_artifact
            .render(&[("artifact", &artifact_name(*artifact))]),
        SubmissionProblem::EmptyArtifact { artifact } => text
            .empty_artifact
            .render(&[("artifact", &artifact_name(*artifact))]),
        SubmissionProblem::ArtifactTooLong {
            artifact,
            max_chars,
        } => text.artifact_too_long.render(&[
            ("artifact", &artifact_name(*artifact)),
            ("max_chars", &number(max_chars)),
        ]),
        SubmissionProblem::ChapterPlanMissing => text.chapter_plan_missing.render(&[]),
        SubmissionProblem::ChapterPlanNotExpected => text.chapter_plan_not_expected.render(&[]),
        SubmissionProblem::ChapterPlanEmpty => text.chapter_plan_empty.render(&[]),
        SubmissionProblem::ChapterPlanTooLong { max_items } => text
            .chapter_plan_too_long
            .render(&[("max_items", &number(max_items))]),
        SubmissionProblem::ChapterTitleEmpty { number: chapter } => text
            .chapter_title_empty
            .render(&[("number", &number(chapter))]),
        SubmissionProblem::ChapterTitleNotSingleLine { number: chapter } => text
            .chapter_title_not_single_line
            .render(&[("number", &number(chapter))]),
        SubmissionProblem::ChapterTitleTooLong {
            number: chapter,
            max_chars,
        } => text.chapter_title_too_long.render(&[
            ("number", &number(chapter)),
            ("max_chars", &number(max_chars)),
        ]),
        SubmissionProblem::ChapterGoalEmpty { number: chapter } => text
            .chapter_goal_empty
            .render(&[("number", &number(chapter))]),
        SubmissionProblem::ChapterGoalTooLong {
            number: chapter,
            max_chars,
        } => text.chapter_goal_too_long.render(&[
            ("number", &number(chapter)),
            ("max_chars", &number(max_chars)),
        ]),
        SubmissionProblem::OutlineEmpty => text.outline_empty.render(&[]),
        SubmissionProblem::OutlineTooLong { max_chars } => text
            .outline_too_long
            .render(&[("max_chars", &number(max_chars))]),
        SubmissionProblem::SummaryEmpty => text.summary_empty.render(&[]),
        SubmissionProblem::SummaryTooLong { max_chars } => text
            .summary_too_long
            .render(&[("max_chars", &number(max_chars))]),
        SubmissionProblem::BodyEmpty => text.body_empty.render(&[]),
        SubmissionProblem::WrongChapter => text.wrong_chapter.render(&[]),
        SubmissionProblem::FeedbackPending => text.feedback_pending.render(&[]),
        SubmissionProblem::ConfidenceOutOfRange => text.confidence_out_of_range.render(&[]),
    }
}

#[must_use]
pub fn protocol_problem(problem: &ProtocolProblem) -> String {
    let text = &model_text().protocol_problems;
    match problem {
        ProtocolProblem::UnknownTool { name } => text.unknown_tool.render(&[("name", name)]),
        ProtocolProblem::ParallelToolCalls => text.parallel_tool_calls.render(&[]),
        ProtocolProblem::InvalidArguments { problem } => text
            .invalid_arguments
            .render(&[("problem", &arguments_problem(problem))]),
    }
}

#[must_use]
pub fn arguments_problem(problem: &ArgumentsProblem) -> String {
    let text = &model_text().arguments_problems;
    let location = |path: &str| {
        if path.is_empty() {
            text.root.render(&[])
        } else {
            text.field.render(&[("path", path)])
        }
    };
    match problem {
        ArgumentsProblem::NotJson => text.not_json.render(&[]),
        ArgumentsProblem::MissingField { path, field } => text
            .missing_field
            .render(&[("location", &location(path)), ("field", field)]),
        ArgumentsProblem::WrongType { path, expected } => text.wrong_type.render(&[
            ("location", &location(path)),
            ("expected", &json_type(*expected)),
        ]),
        ArgumentsProblem::NotAllowed {
            path,
            value,
            allowed,
        } => text.not_allowed.render(&[
            ("location", &location(path)),
            ("value", value),
            ("allowed", &allowed.join(", ")),
        ]),
    }
}

fn json_type(json_type: JsonType) -> String {
    let types = &model_text().json_types;
    match json_type {
        JsonType::Object => types.object.render(&[]),
        JsonType::Array => types.array.render(&[]),
        JsonType::String => types.string.render(&[]),
        JsonType::Integer => types.integer.render(&[]),
        JsonType::Number => types.number.render(&[]),
        JsonType::Boolean => types.boolean.render(&[]),
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::run_message::RejectedArtifact;

    use super::*;

    #[test]
    fn knowledge_results_carry_the_hits_and_the_budget_left() {
        let found: serde_json::Value =
            serde_json::from_str(&tool_outcome(&ToolOutcome::KnowledgeFound {
                hits: vec![KnowledgeHit {
                    title: "Lamp oil".to_owned(),
                    space: "Lighthouses".to_owned(),
                    text: "Carried up the stairs.".to_owned(),
                    relevance_permille: 875,
                }],
                remaining_budget: 4,
            }))
            .unwrap();
        assert_eq!(found["status"], "found");
        assert_eq!(found["hits"][0]["relevance"], 0.875);
        assert!(found["message"].as_str().unwrap().contains('4'));

        let exhausted = tool_outcome(&ToolOutcome::QuotaExhausted { budget: 10 });
        assert!(
            exhausted.contains("10") && exhausted.contains(ASK_USER),
            "{exhausted}"
        );
    }

    // Together these tests render every message kind, so every placeholder in the resource
    // file is checked against the values the code passes.

    #[test]
    fn every_submission_problem_renders() {
        let artifacts = vec![ArtifactKind::Background, ArtifactKind::Outline];
        let problems = [
            SubmissionProblem::MissingArtifacts {
                kinds: artifacts.clone(),
            },
            SubmissionProblem::UnexpectedArtifacts { kinds: artifacts },
            SubmissionProblem::DuplicateArtifact {
                artifact: ArtifactKind::Research,
            },
            SubmissionProblem::EmptyArtifact {
                artifact: ArtifactKind::Research,
            },
            SubmissionProblem::ArtifactTooLong {
                artifact: ArtifactKind::Characters,
                max_chars: 10,
            },
            SubmissionProblem::ChapterPlanMissing,
            SubmissionProblem::ChapterPlanNotExpected,
            SubmissionProblem::ChapterPlanEmpty,
            SubmissionProblem::ChapterPlanTooLong { max_items: 5 },
            SubmissionProblem::ChapterTitleEmpty { number: 2 },
            SubmissionProblem::ChapterTitleNotSingleLine { number: 2 },
            SubmissionProblem::ChapterTitleTooLong {
                number: 2,
                max_chars: 3,
            },
            SubmissionProblem::ChapterGoalEmpty { number: 2 },
            SubmissionProblem::ChapterGoalTooLong {
                number: 2,
                max_chars: 3,
            },
            SubmissionProblem::OutlineEmpty,
            SubmissionProblem::OutlineTooLong { max_chars: 3 },
            SubmissionProblem::SummaryEmpty,
            SubmissionProblem::SummaryTooLong { max_chars: 3 },
            SubmissionProblem::BodyEmpty,
            SubmissionProblem::WrongChapter,
            SubmissionProblem::FeedbackPending,
            SubmissionProblem::ConfidenceOutOfRange,
        ];
        for problem in problems {
            let rendered = tool_outcome(&ToolOutcome::SubmissionRejected { problem });
            assert!(rendered.contains("\"rejected\""), "{rendered}");
        }
        let accepted = tool_outcome(&ToolOutcome::SubmissionAccepted);
        assert!(accepted.contains("\"accepted\""), "{accepted}");
    }

    #[test]
    fn every_protocol_problem_renders() {
        let arguments = [
            ArgumentsProblem::NotJson,
            ArgumentsProblem::MissingField {
                path: String::new(),
                field: "artifacts".to_owned(),
            },
            ArgumentsProblem::WrongType {
                path: "artifacts".to_owned(),
                expected: JsonType::Array,
            },
            ArgumentsProblem::NotAllowed {
                path: "artifacts[0].kind".to_owned(),
                value: "x".to_owned(),
                allowed: vec!["outline".to_owned()],
            },
        ];
        let problems = arguments
            .into_iter()
            .map(|problem| ProtocolProblem::InvalidArguments { problem })
            .chain([
                ProtocolProblem::UnknownTool {
                    name: "x".to_owned(),
                },
                ProtocolProblem::ParallelToolCalls,
            ]);
        for problem in problems {
            let rendered = tool_outcome(&ToolOutcome::ProtocolError { problem });
            assert!(rendered.contains("\"error\""), "{rendered}");
        }
        for json_type in [
            JsonType::Integer,
            JsonType::Number,
            JsonType::Boolean,
            JsonType::Object,
            JsonType::String,
        ] {
            assert_ne!(super::json_type(json_type), "");
        }
    }

    #[test]
    fn corrections_review_outcomes_and_fixed_texts_render() {
        for correction in [
            Correction::NoToolCall,
            Correction::ProtocolError,
            Correction::SubmissionRejected,
        ] {
            let message = unsealed(
                &UnsealedMessage::Correction { correction },
                "submit_preparation",
            );
            assert!(matches!(message, TranscriptMessage::User { .. }));
        }
        let outcome = unsealed(
            &UnsealedMessage::PreparationReviewOutcome {
                rejected: vec![RejectedArtifact {
                    artifact: ArtifactKind::Outline,
                    feedback: Some("More conflict.".to_owned()),
                    citations: Vec::new(),
                }],
                stale: vec![ArtifactKind::Characters],
            },
            "submit_preparation",
        );
        let TranscriptMessage::User { content } = outcome else {
            panic!("a review outcome is a user message");
        };
        assert!(content.contains("More conflict."));
        assert_ne!(language_name(WritingLanguage::En), "");
        let test = &model_text().connection_test;
        assert_ne!(test.system.render(&[]), "");
        assert_ne!(test.user.render(&[]), "");
    }

    #[test]
    fn a_no_tool_call_correction_names_the_submission_tool() {
        let TranscriptMessage::User { content } = unsealed(
            &UnsealedMessage::Correction {
                correction: Correction::NoToolCall,
            },
            "submit_preparation",
        ) else {
            panic!("a correction is a user message");
        };
        assert!(content.contains("submit_preparation"));
    }

    #[test]
    fn every_edit_outcome_renders_with_its_status() {
        let applied = tool_outcome(&ToolOutcome::EditApplied {
            base_revision: None,
            revision: nekowriter_core::ids::RevisionId::generate(),
            word_count: 12,
        });
        assert!(
            applied.contains("\"applied\"") && applied.contains("12"),
            "{applied}"
        );
        let declined = tool_outcome(&ToolOutcome::EditDeclined);
        assert!(declined.contains("\"rejected\""), "{declined}");
        let unchanged = tool_outcome(&ToolOutcome::EditNoChange { word_count: 12 });
        assert!(unchanged.contains("\"unchanged\""), "{unchanged}");
        for problem in [
            EditProblem::EmptyContent,
            EditProblem::EmptyOldStr,
            EditProblem::NoMatch,
            EditProblem::AmbiguousMatch { matches: 3 },
            EditProblem::BodyTooLong { max_chars: 9 },
            EditProblem::StaleBase,
        ] {
            let rendered = tool_outcome(&ToolOutcome::EditRejected { problem });
            assert!(rendered.contains("\"rejected\""), "{rendered}");
        }
    }

    #[test]
    fn rejected_outlines_and_chapters_carry_the_feedback() {
        for kind in [ReviewKind::ChapterOutline, ReviewKind::ChapterFinal] {
            let TranscriptMessage::User { content } = unsealed(
                &UnsealedMessage::ReviewRejected {
                    kind,
                    feedback: Some("More rain.".to_owned()),
                    citations: Vec::new(),
                },
                "submit_chapter",
            ) else {
                panic!("a review outcome is a user message");
            };
            assert!(content.contains("More rain."));
        }
    }

    fn chapter(kind: ChapterKind) -> ChapterInput {
        ChapterInput {
            id: nekowriter_core::ids::ChapterId::generate(),
            kind,
            number: 2,
            title: "Storm".to_owned(),
            goal: String::new(),
        }
    }

    #[test]
    fn folds_render_every_stage_once_in_order() {
        use nekowriter_core::{ids::PlanItemId, preparation::ChapterPlanItem, stage::FoldDocument};

        assert_eq!(folds(&[]), None);
        let document = |role, content: &str, chapter_plan| FoldDocument {
            role,
            content: content.to_owned(),
            chapter_plan,
        };
        let plan = ChapterPlan(vec![ChapterPlanItem {
            id: PlanItemId::generate(),
            title: "Arrival".to_owned(),
            goal: "The letter.".to_owned(),
        }]);
        let text = folds(&[
            FoldInput {
                stage: FoldStage::Preparation,
                documents: vec![
                    document(DocumentRole::Background, "BACKGROUND", None),
                    document(DocumentRole::Outline, "OUTLINE", Some(plan)),
                ],
            },
            FoldInput {
                stage: FoldStage::ChapterBody {
                    chapter: chapter(ChapterKind::Main),
                },
                documents: vec![document(DocumentRole::ChapterBody, "BODY", None)],
            },
        ])
        .unwrap();
        let background = text.find("BACKGROUND").unwrap();
        let outline = text.find("OUTLINE").unwrap();
        let plan = text.find("The letter.").unwrap();
        let body = text.find("BODY").unwrap();
        assert!(
            background < outline && outline < plan && plan < body,
            "{text}"
        );
        assert!(text.contains("Storm"));
    }

    #[test]
    fn chapter_names_and_bodies_render() {
        assert!(chapter_name(&chapter(ChapterKind::Extra)).contains("Storm"));
        assert_ne!(
            chapter_name(&chapter(ChapterKind::Main)),
            chapter_name(&chapter(ChapterKind::Extra))
        );
        assert!(current_body("The keeper woke.").contains("The keeper woke."));
        assert_ne!(current_body(" "), "");
    }

    #[test]
    fn feedback_travels_in_its_own_array() {
        let plain = tool_result(&ToolOutcome::EditNoChange { word_count: 1 }, &[]);
        assert!(!plain.contains("feedback"), "{plain}");
        let with_feedback = tool_result(
            &ToolOutcome::EditNoChange { word_count: 1 },
            &["More rain.".to_owned()],
        );
        let value: serde_json::Value = serde_json::from_str(&with_feedback).unwrap();
        assert_eq!(value["feedback"], json!(["More rain."]));
        assert_eq!(value["status"], "unchanged");
    }

    #[test]
    fn answers_and_question_problems_render() {
        use nekowriter_core::question::AnsweredQuestion;

        let answered = tool_outcome(&ToolOutcome::Answered {
            answers: vec![AnsweredQuestion {
                question: "Weather?".to_owned(),
                selected: vec!["Snow".to_owned()],
                custom_text: Some("and fog".to_owned()),
            }],
        });
        let value: serde_json::Value = serde_json::from_str(&answered).unwrap();
        assert_eq!(value["status"], "answered");
        assert_eq!(value["answers"][0]["selected"], json!(["Snow"]));
        assert_eq!(value["answers"][0]["text"], "and fog");
        let id = || "q1".to_owned();
        for problem in [
            QuestionProblem::NoQuestions,
            QuestionProblem::TooManyQuestions { max: 10 },
            QuestionProblem::DuplicateQuestion { id: id() },
            QuestionProblem::EmptyTitle { id: id() },
            QuestionProblem::NoOptions { id: id() },
            QuestionProblem::TooManyOptions { id: id(), max: 20 },
            QuestionProblem::DuplicateOption {
                id: id(),
                option: "a".to_owned(),
            },
            QuestionProblem::EmptyOption {
                id: id(),
                option: "a".to_owned(),
            },
        ] {
            let rendered = tool_outcome(&ToolOutcome::QuestionRejected { problem });
            assert!(rendered.contains("\"rejected\""), "{rendered}");
        }
    }

    #[test]
    fn citations_follow_their_feedback_with_their_source() {
        use nekowriter_core::{
            chapter::ChapterLabel,
            citation::{Citation, CitationTarget},
            ids::RevisionId,
        };

        let cited = VerifiedCitation {
            citation: Citation {
                target: CitationTarget::Revision {
                    revision_id: RevisionId::generate(),
                },
                start: 0,
                end: 4,
                quote: "dark".to_owned(),
            },
            source: CitationSource::Document {
                role: DocumentRole::ChapterBody,
                chapter: Some(ChapterLabel {
                    kind: ChapterKind::Main,
                    number: 2,
                    title: "Storm".to_owned(),
                }),
            },
        };
        let text = with_citations("Brighter.", std::slice::from_ref(&cited));
        let brighter = text.find("Brighter.").unwrap();
        let quote = text.find("dark").unwrap();
        assert!(brighter < quote && text.contains("Storm"), "{text}");
        assert_eq!(with_citations("Plain.", &[]), "Plain.");
        assert!(rejection_feedback(None, &[cited]).contains("dark"));
        assert_eq!(
            rejection_feedback(None, &[]),
            model_text().review.no_issues.render(&[])
        );
    }
}
