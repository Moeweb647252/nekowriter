//! Helpers shared by the store's integration tests.

// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

use nekowriter_core::{
    body::BodyEdit,
    call::{CallRequest, StageSubmission},
    ids::{CommandId, ModelRequestId, ReviewId, VersionId},
    model::{ApiBaseUrl, ModelId, ModelProfile},
    preparation::{ArtifactDraft, ArtifactKind, NovelBrief, PlanItemDraft, PreparationSubmission},
    project::{NovelTitle, ProjectName, WritingLanguage},
    review::{ReviewDecision, ReviewFeedback, ReviewStatus},
    run_message::{ModelToolCall, TranscriptMessage},
    usage::TokenUsage,
    workflow::WorkflowStage,
};
use nekowriter_store::{
    ApiKeyChange, DATABASE_FILE, DispatchInput, DispatchOutcome, NewProject, ProjectView,
    RecordedAnswer, RoundRecord, SealedRequest, Store, SystemSettingsInput, TranscriptEntry,
    WorkspaceView,
};
use tempfile::TempDir;

/// A store in a fresh temporary directory, which is removed when the guard drops.
pub async fn open_store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("a temporary directory can be created");
    let store = Store::open(&dir.path().join(DATABASE_FILE), "test")
        .await
        .expect("a new database opens");
    (dir, store)
}

pub fn new_project(name: &str) -> NewProject {
    NewProject {
        name: ProjectName::parse(name).unwrap(),
        novel_title: NovelTitle::parse("Rain Letters").unwrap(),
        writing_language: WritingLanguage::En,
    }
}

pub async fn create_project(store: &Store, name: &str) -> ProjectView {
    store
        .create_project(CommandId::generate(), new_project(name))
        .await
        .expect("the project is created")
}

/// Saves a model service and a default model, so model work can start.
pub async fn configure_model(store: &Store) {
    let revision = store.system_settings().await.unwrap().revision;
    store
        .save_system_settings(
            CommandId::generate(),
            revision,
            SystemSettingsInput {
                api_base_url: Some(ApiBaseUrl::parse("http://127.0.0.1:9/v1").unwrap()),
                api_key: ApiKeyChange::Keep,
                default_model: Some(ModelProfile {
                    model_id: ModelId::parse("writer").unwrap(),
                    context_window: 128_000,
                    max_output_tokens: 8_000,
                    supports_thinking: false,
                    reports_stream_usage: true,
                }),
            },
        )
        .await
        .expect("the model settings are saved");
}

/// A tool call with empty arguments; the store does not read them.
pub fn call(name: &str) -> ModelToolCall {
    ModelToolCall {
        provider_id: format!("call_{name}"),
        name: name.to_owned(),
        arguments: "{}".to_owned(),
    }
}

/// A round in which the model calls the stage's submission tool `tool`.
pub fn submit_round(tool: &str, submission: StageSubmission) -> RoundRecord {
    RoundRecord::Call {
        call: call(tool),
        request: CallRequest::Submit { submission },
    }
}

/// A round in which the model edits the body.
pub fn edit_round(edit: BodyEdit) -> RoundRecord {
    let tool = match edit {
        BodyEdit::Append { .. } => "append_content",
        BodyEdit::Replace { .. } => "replace_content",
    };
    RoundRecord::Call {
        call: call(tool),
        request: CallRequest::Edit { edit },
    }
}

/// Records the model's answer to the oldest queued run: `round` and the call it carries.
pub async fn record(store: &Store, round: RoundRecord) {
    let call = match &round {
        RoundRecord::Call { call, .. } => call.clone(),
        RoundRecord::ProtocolError { .. } => panic!("record protocol errors directly"),
    };
    let (_, request) = dispatch_next(store).await;
    store
        .record_answer(request, answer(&call), round)
        .await
        .unwrap();
}

/// A version whose preparation is approved, at the first of its two chapters.
pub async fn prepared(store: &Store) -> VersionId {
    let version = started(store).await;
    submit(store, &ArtifactKind::ALL).await;
    for kind in ArtifactKind::ALL {
        decide(store, version, kind, ReviewDecision::Approve).await;
    }
    version
}

pub async fn workspace(store: &Store, version: VersionId) -> WorkspaceView {
    store.workspace(version).await.unwrap()
}

/// A project whose first version has a brief, and the workflow revision after saving it.
pub async fn briefed(store: &Store) -> (VersionId, i64) {
    let project = create_project(store, "Rain").await;
    let version = project.selected_version.id;
    let revision = workspace(store, version).await.revision;
    let ack = store
        .save_brief(
            CommandId::generate(),
            version,
            revision,
            NovelBrief::parse("A ghost story in a lighthouse.").unwrap(),
        )
        .await
        .unwrap();
    (version, ack.revision)
}

/// A project whose first version has a brief and a started preparation run.
pub async fn started(store: &Store) -> VersionId {
    configure_model(store).await;
    let (version, revision) = briefed(store).await;
    store
        .start_preparation(CommandId::generate(), version, revision)
        .await
        .unwrap();
    version
}

/// Dispatches the oldest queued run, rendering pending messages as their debug text.
pub async fn dispatch_next(store: &Store) -> (DispatchInput, ModelRequestId) {
    let run = store
        .next_queued_run()
        .await
        .unwrap()
        .expect("a run is queued");
    let input = store.dispatch_input(run).await.unwrap();
    let rendered = input
        .messages
        .iter()
        .filter_map(|entry| match entry {
            TranscriptEntry::Pending { id, message } => Some((
                *id,
                TranscriptMessage::User {
                    content: format!("{message:?}"),
                },
            )),
            TranscriptEntry::Rendered(_) => None,
        })
        .collect();
    let outcome = store
        .dispatch(
            &input,
            SealedRequest {
                rendered,
                body: "{}".to_owned(),
                estimated_input_tokens: 10,
            },
        )
        .await
        .unwrap();
    let DispatchOutcome::Dispatched { request_id } = outcome else {
        panic!("the queued run is dispatched");
    };
    (input, request_id)
}

pub fn answer(call: &ModelToolCall) -> RecordedAnswer {
    RecordedAnswer {
        content: None,
        tool_calls: vec![call.clone()],
        finish_reason: Some("tool_calls".to_owned()),
        usage: TokenUsage::default(),
        raw_usage: None,
        provider_request_id: None,
    }
}

pub fn preparation_submission(kinds: &[ArtifactKind]) -> PreparationSubmission {
    PreparationSubmission {
        artifacts: kinds
            .iter()
            .map(|kind| ArtifactDraft {
                kind: *kind,
                content: format!("{kind:?} text"),
            })
            .collect(),
        chapter_plan: kinds.contains(&ArtifactKind::Outline).then(|| {
            vec![
                PlanItemDraft {
                    title: "Arrival".to_owned(),
                    goal: "The keeper finds the letter.".to_owned(),
                },
                PlanItemDraft {
                    title: "Storm".to_owned(),
                    goal: "The lamp fails.".to_owned(),
                },
            ]
        }),
    }
}

/// Lets the model submit `kinds` for the next queued run.
pub async fn submit(store: &Store, kinds: &[ArtifactKind]) {
    let (_, request) = dispatch_next(store).await;
    store
        .record_answer(
            request,
            answer(&call("submit_preparation")),
            submit_round(
                "submit_preparation",
                StageSubmission::Preparation {
                    submission: preparation_submission(kinds),
                },
            ),
        )
        .await
        .unwrap();
}

pub fn review_of(view: &WorkspaceView, kind: ArtifactKind) -> (ReviewId, ReviewStatus) {
    let review = view
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == kind)
        .and_then(|artifact| artifact.review.as_ref())
        .expect("the artifact has a review");
    (review.id, review.status)
}

pub async fn decide(
    store: &Store,
    version: VersionId,
    kind: ArtifactKind,
    decision: ReviewDecision,
) {
    let view = workspace(store, version).await;
    let (review, _) = review_of(&view, kind);
    store
        .decide_review(CommandId::generate(), review, view.revision, decision)
        .await
        .unwrap();
}

/// Decides the review with `review_id`, quoting the workspace's current revision.
pub async fn decide_review(
    store: &Store,
    version: VersionId,
    review_id: ReviewId,
    decision: ReviewDecision,
) {
    let revision = workspace(store, version).await.revision;
    store
        .decide_review(CommandId::generate(), review_id, revision, decision)
        .await
        .unwrap();
}

pub fn reject(feedback: &str) -> ReviewDecision {
    ReviewDecision::Reject {
        feedback: ReviewFeedback::parse(feedback).unwrap(),
    }
}

pub fn stage(view: &WorkspaceView) -> &WorkflowStage {
    &view.version.stage
}
