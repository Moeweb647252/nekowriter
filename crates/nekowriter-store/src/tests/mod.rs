//! `store` 的测试模块：导入表与共享装置在这里，按域拆到兄弟子模块。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3）：53 个定义项按域进 14 个子模块，
//! 共享 harness 与需要真实数据库的公共装置留在本文件，子模块用 `use super::*` 取用。
//!
//! `lance` 而不是 `lancedb`：子模块名会遮蔽同名的外部 crate，而用例体里就有
//! `lancedb::connect(...)`。

use arrow_array::record_batch;
use futures_util::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use nekowriter_core::{
    AppendChapterContent, ApprovalMode, ChapterId, ChapterPlanDraft, DocumentId, ExecutionStatus,
    HumanReviewDecision, MAX_CONSECUTIVE_PROTOCOL_ERRORS, ModelCapabilityProfile,
    NewChapterGuidance, NewChapterOutline, NewCopiedNovelVersion, NewExtraChapter, NewNovelBrief,
    NewNovelVersion, NewProject, NewPromptTemplate, PreparationArtifactInput,
    PreparationArtifactKind, ProjectProgress, ReplaceChapterContent, ReviewId, ReviewStatus,
    RevisionId, RunId, RunWaitReason, WorkflowStage, count_as_i64, non_whitespace_char_count,
};
use sea_orm::{EntityTrait, IntoActiveModel, ModelTrait, QueryFilter, Set};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

use crate::entity::{chapter, document, document_revision, novel_version, workflow_instance};

use super::*;

mod agent_messages;
mod chapters;
mod commands;
mod events;
mod fold;
mod lance;
mod model_requests;
mod preparation;
mod projects;
mod review;
mod schema;
mod settings;
mod shape;
mod writing;

// 把每个文件按 4 空格缩进的定义行切成函数体，注释行先去掉，避免注释里的名字被数进
// 形状。
fn functions(source: &str) -> Vec<(String, String)> {
    let mut functions: Vec<(String, String)> = Vec::new();
    for line in source.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let Some(rest) = line.strip_prefix("    ") else {
            if let Some((_, body)) = functions.last_mut() {
                body.push_str(line);
                body.push('\n');
            }
            continue;
        };
        let rest = rest.strip_prefix("pub ").unwrap_or(rest);
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let is_definition = rest.starts_with("fn ") && !line.starts_with("        ");
        if is_definition {
            let name = rest["fn ".len()..].split('(').next().unwrap_or_default();
            functions.push((name.to_owned(), String::new()));
        }
        if let Some((_, body)) = functions.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    functions
}

/// 子 Run 行里落库的连续协议错误计数。
async fn persisted_protocol_error_count(store: &Store, run_id: RunId) -> i64 {
    entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap()
        .consecutive_protocol_errors
}

/// 这一轮被拒时事件公布出来的连续协议错误计数。
///
/// 这个数字只通过事件到达客户端（快照与 Dashboard 的 DTO 都不含它），所以它和
/// 落库值必须来自同一次写入、永远相等：否则行里的数字会与它刚发出的事件矛盾。
/// 两条 lane 的纠正事件都带 `request_id`，用它把范围锁在这一轮。
async fn published_protocol_error_count(store: &Store, request_id: Uuid) -> i64 {
    store
        .events_after(0)
        .await
        .unwrap()
        .into_iter()
        .rfind(|event| {
            event.payload["protocol_correction"] == true
                && event.payload["request_id"].as_str() == Some(&request_id.to_string())
        })
        .expect("被拒的一轮必须留下纠正事件")
        .payload["consecutive_protocol_errors"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// 已启动准备阶段、等待模型首轮响应的写作 Run。
async fn in_flight_preparation_run(store: &Store, title: &str) -> (VersionId, RunId) {
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new(title, "被业务守卫拒绝的提交必须可继续", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = created.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个结局清晰的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let initial = store
        .get_version_snapshot(version_id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(version_id, Uuid::new_v4(), initial.state_revision)
        .await
        .unwrap();
    (version_id, started.workflow.active_run_id.unwrap())
}

/// 已进入准备阶段并已把背景产物委派给审核子 Run 的稿本。
///
/// 审核 lane 的两个用例（协议纠正、被业务守卫拒绝）需要完全相同的起点；把它写成
/// 一处，两条 lane 的差别就只留在各自真正要断言的收束行为上。
async fn delegated_background_review(store: &Store, title: &str) -> (VersionId, ReviewId, RunId) {
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new(title, "被拒的审核提交必须可继续", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = created.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部围绕旧码头和匿名报纸的悬疑小说。").unwrap(),
        )
        .await
        .unwrap();
    let started = store
        .start_preparation(version_id, Uuid::new_v4(), 0)
        .await
        .unwrap();
    let snapshot = store
        .submit_preparation_artifacts(
            version_id,
            Uuid::new_v4(),
            started.workflow.state_revision,
            vec![
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Background,
                    "港城的旧码头将在雨季后拆除。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "旧报纸来自 1998 年的港城晚报。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Characters,
                    "林舟是追查父亲旧案的记者。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Outline,
                    "林舟在码头收到匿名报纸，循着署名进入仓库并决定公开线索。",
                )
                .unwrap(),
            ],
            Some(vec![
                ChapterPlanDraft::new("雨夜来客", "林舟在码头收到匿名报纸。").unwrap(),
            ]),
        )
        .await
        .unwrap();
    let review_id = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
        .and_then(|artifact| artifact.review.as_ref())
        .map(|review| review.id)
        .unwrap();
    let delegation = store
        .delegate_review(
            review_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            "检查设定是否足以支撑第一章。".to_owned(),
            Vec::new(),
        )
        .await
        .unwrap();
    (version_id, review_id, delegation.run_id)
}

/// 已进入正文写作阶段、章节 Run 可以继续派发请求的稿本。
///
/// 正文工具调用的失败分支、以及失败之后“Run 仍可继续写”的断言都从同一处起点出发；
/// 把准备、四项审核、章节细纲与细纲审核写成一处，用例之间的差别就只留在各自真正要
/// 断言的正文编辑上。
async fn drafting_chapter_body_run(store: &Store, title: &str) -> (VersionId, ChapterId, RunId) {
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new(title, "雨夜车站", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = created.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部围绕雨夜车站与失踪信件展开的悬疑短篇。").unwrap(),
        )
        .await
        .unwrap();
    let started = store
        .start_preparation(version_id, Uuid::new_v4(), 0)
        .await
        .unwrap();
    let mut snapshot = store
        .submit_preparation_artifacts(
            version_id,
            Uuid::new_v4(),
            started.workflow.state_revision,
            vec![
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Background,
                    "雨夜的旧车站即将拆除，末班车仍在运行。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "写信人只留下了一枚车站印章，出处尚待核验。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Characters,
                    "沈宁是追查失踪姐姐下落的摄影记者。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Outline,
                    "沈宁在末班车上收到旧信，决定进入废弃候车室寻找寄信人。",
                )
                .unwrap(),
            ],
            Some(vec![
                ChapterPlanDraft::new("末班车", "沈宁在雨夜车站收到一封旧信。").unwrap(),
            ]),
        )
        .await
        .unwrap();
    for kind in PreparationArtifactKind::ALL {
        let review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == kind)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        snapshot = store
            .decide_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
    }
    let chapter_id = snapshot.workflow.active_chapter_id.unwrap();
    store
        .save_chapter_guidance(
            chapter_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            NewChapterGuidance::new("让沈宁先辨认旧信上的车站印章。").unwrap(),
        )
        .await
        .unwrap();
    let ready = store.get_preparation_snapshot(version_id).await.unwrap();
    let outline_started = store
        .start_chapter_outline(chapter_id, Uuid::new_v4(), ready.workflow.state_revision)
        .await
        .unwrap();
    let outline = store
        .submit_chapter_outline(
            chapter_id,
            Uuid::new_v4(),
            outline_started.state_revision,
            NewChapterOutline::new(
                "1. 沈宁辨认旧信上的车站印章。\n2. 末班车鸣笛，迫使她决定是否追上寄信人。",
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let drafting = store
        .decide_review(
            outline.outline.review.as_ref().unwrap().id,
            Uuid::new_v4(),
            outline.workflow.state_revision,
            HumanReviewDecision::new(true, "").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(drafting.workflow.stage, WorkflowStage::WritingDrafting);
    let run_id = drafting.workflow.active_run_id.unwrap();
    (version_id, chapter_id, run_id)
}

/// 在正文 Run 上派发一次模型请求并执行一次正文工具调用。
///
/// 正文阶段的工具调用必须属于一条已经派发的请求（store 用它记账），所以用例里
/// “造一次调用”就是“派发 + 执行”这两步；写在一处，各条断言不必各拄一遍。
async fn dispatched_body_tool_call(
    store: &Store,
    chapter_id: ChapterId,
    run_id: RunId,
    attempt: usize,
    name: &str,
    arguments: serde_json::Value,
) -> AppliedChapterBodyToolCall {
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "body-tool-error-test" }),
                prompt_snapshot: json!({ "contentHash": format!("body-tool-{attempt}") }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    store
        .execute_chapter_body_agent_tool_call(
            request.id,
            chapter_id,
            Some(format!("body-tool-provider-{attempt}")),
            json!({}),
            ChapterBodyAgentToolCall {
                provider_call_id: format!("body-tool-call-{attempt}"),
                name: name.to_owned(),
                arguments,
            },
        )
        .await
        .unwrap()
}
