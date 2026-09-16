//! HTTP 请求与响应体的形状（DTO）。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.2），字段、`serde` 属性与文档一行未改，
//! 只有可见性从私有放宽到 `pub(crate)`——它要跨模块用，而这正是「只移动」允许的那一类编辑。
//! `main.rs` 用一条 `use http::dto::{...}` 把名字接回原位，所以每个 handler 与每个路由都
//! 保持字面不变。

use nekowriter_core::{
    ApprovalMode, FeedbackCitation, ModelCapabilityProfile, PreparationArtifactKind,
    ProjectModelReference, ThinkingLevel, UserQuestionAnswer, VersionId, decimal_i64,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateProjectRequest {
    pub(crate) command_id: Uuid,
    pub(crate) name: String,
    pub(crate) novel_title: String,
    pub(crate) version_label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateVersionRequest {
    pub(crate) command_id: Uuid,
    pub(crate) label: String,
    pub(crate) novel_title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectVersionRequest {
    pub(crate) command_id: Uuid,
    pub(crate) version_id: VersionId,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_project_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyVersionRequest {
    pub(crate) command_id: Uuid,
    pub(crate) label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveNovelBriefRequest {
    pub(crate) command_id: Uuid,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartPreparationRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparationArtifactRequest {
    pub(crate) kind: PreparationArtifactKind,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChapterPlanDraftRequest {
    pub(crate) title: String,
    pub(crate) goal: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmitPreparationArtifactsRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) artifacts: Vec<PreparationArtifactRequest>,
    pub(crate) chapter_plan: Option<Vec<ChapterPlanDraftRequest>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DecidePreparationReviewRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) passed: bool,
    pub(crate) issues: Option<String>,
    #[serde(default)]
    pub(crate) citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DelegateReviewRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    #[serde(default)]
    pub(crate) notes: String,
    #[serde(default)]
    pub(crate) citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelReviewDelegationRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveChapterGuidanceRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartChapterOutlineRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmitChapterOutlineRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppendChapterBodyRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplaceChapterBodyRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) old_str: String,
    pub(crate) new_str: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetApprovalModeRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) mode: ApprovalMode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DecideChapterBodyProposalRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    #[serde(default)]
    pub(crate) feedback: String,
    pub(crate) citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmitRunFeedbackRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) text: String,
    pub(crate) citations: Vec<FeedbackCitation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerQuestionRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    /// 一次询问的整批答案。
    ///
    /// 这里直接收列表，而不是领域类型 `UserQuestionAnswers`：后者的唯一字段也叫
    /// `answers`，把它当作同名字段会形成线上形状 `answers.answers`，与界面按本字段
    /// 发送的 `answers: [...]` 不一致。
    pub(crate) answers: Vec<UserQuestionAnswer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunWorkflowControlRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmitChapterFinalReviewRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdvanceToNextChapterRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfirmNovelCompletionRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddExtraChapterRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_state_revision: i64,
    pub(crate) title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveSystemModelSettingsRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_revision: i64,
    pub(crate) api_base_url: String,
    pub(crate) model_id: String,
    pub(crate) capabilities: nekowriter_core::ModelCapabilityProfile,
    #[serde(with = "decimal_i64")]
    pub(crate) max_context_tokens: i64,
    #[serde(with = "decimal_i64")]
    pub(crate) reserved_output_tokens: i64,
    /// 三态：省略 = 保留既有密钥，`null` = 清回未配置，字符串 = 替换。
    #[serde(default)]
    pub(crate) api_key: nekowriter_store::CredentialPatch,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemSettingsResponse {
    pub(crate) writing_model: Option<PublicSystemModelSettings>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicSystemModelSettings {
    pub(crate) api_base_url: String,
    pub(crate) model_id: String,
    pub(crate) capabilities: nekowriter_core::ModelCapabilityProfile,
    pub(crate) max_context_tokens: String,
    pub(crate) reserved_output_tokens: String,
    pub(crate) revision: String,
    pub(crate) credential_configured: bool,
    pub(crate) created_at_ms: String,
    pub(crate) updated_at_ms: String,
}

/// 连接测试的请求体：明确要测试哪个模型、以及它声明了哪些能力。
///
/// 模型 ID 与能力资料一同给出，因为「是否接受 `reasoning_effort`」是模型的属性，
/// 而不是当前系统默认设置的属性。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestModelRequest {
    pub(crate) model_id: String,
    pub(crate) capabilities: ModelCapabilityProfile,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestModelResponse {
    pub(crate) provider_request_id: Option<String>,
    pub(crate) content_received: bool,
    pub(crate) tool_call_count: usize,
    pub(crate) usage: Option<nekowriter_agent::provider::ModelUsage>,
    /// 本次探针是否真的带上了 `reasoning_effort`。只有它为真时，
    /// 「该模型支持思考」才是被验证过的声明。
    pub(crate) thinking_probed: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventsQuery {
    pub(crate) after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HealthResponse {
    pub(crate) status: &'static str,
    pub(crate) schema_version: u32,
    /// 当前进程实际使用的数据目录，界面用它告诉用户数据与日志在哪里。
    pub(crate) data_dir: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShutdownResponse {
    pub(crate) status: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventCursorResponse {
    pub(crate) event_watermark: String,
}

/// 导出查询参数；格式是必填项，不接受默认值。
#[derive(Debug, Deserialize)]
pub(crate) struct ExportQuery {
    pub(crate) format: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveProjectSettingsRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_revision: i64,
    /// 模型引用必须显式给出（`null` 表示清除覆盖）。缺少该字段会被拒绝，
    /// 而不是当成「未覆盖」静默清空项目设置。
    pub(crate) model: Option<ProjectModelReference>,
    pub(crate) thinking_level: ThinkingLevel,
    #[serde(with = "decimal_i64")]
    pub(crate) auxiliary_tool_budget: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePromptOverrideRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_revision: i64,
    pub(crate) system: String,
    pub(crate) user: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetPromptOverrideRequest {
    pub(crate) command_id: Uuid,
    #[serde(with = "decimal_i64")]
    pub(crate) expected_revision: i64,
}

#[cfg(test)]
mod tests {
    use nekowriter_store::CredentialPatch;
    use serde_json::json;

    use crate::knowledge;

    use super::*;

    #[test]
    fn state_revision_requests_require_decimal_strings() {
        let command_id = Uuid::new_v4();
        let parsed: StartPreparationRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedStateRevision": i64::MAX.to_string(),
        }))
        .unwrap();
        assert_eq!(parsed.command_id, command_id);
        assert_eq!(parsed.expected_state_revision, i64::MAX);
        assert!(
            serde_json::from_value::<StartPreparationRequest>(json!({
                "commandId": Uuid::new_v4(),
                "expectedStateRevision": 1,
            }))
            .is_err()
        );
    }

    /// 审核决定的请求体形状必须与界面 `decideChapterFinalReview` 真正发出的 JSON 一致。
    ///
    /// 「退回修订」写下的意见是正文 Agent 的修订要求：服务端把它排进正文 Run 的自由
    /// 意见批次（store 一侧见 `chapter_final_rejection_hands_its_issues_to_the_body_agent`）。
    /// 这里钉住界面真实发出的报文，避免退回意见因字段名或取值形状不符而根本没到服务端。
    #[test]
    fn review_decision_accepts_the_payload_the_ui_sends() {
        let command_id = Uuid::new_v4();
        let issues = "让沈宁主动留下查清寄信人；第二段的雨声描写与结尾重复。";
        let rejected: DecidePreparationReviewRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedStateRevision": "27",
            "passed": false,
            "issues": issues,
            "citations": [
                {
                    "kind": "document",
                    "documentId": Uuid::new_v4(),
                    "revisionId": Uuid::new_v4(),
                    "startChar": "4",
                    "endChar": "6",
                    "quote": "胶卷",
                    "chapterId": null,
                }
            ],
        }))
        .unwrap();
        assert_eq!(rejected.command_id, command_id);
        assert_eq!(rejected.expected_state_revision, 27);
        assert!(!rejected.passed);
        assert_eq!(rejected.issues.as_deref(), Some(issues));
        assert_eq!(rejected.citations.len(), 1);

        // 通过时界面把 issues 写成空字符串、citations 写成空数组。
        let approved: DecidePreparationReviewRequest = serde_json::from_value(json!({
            "commandId": Uuid::new_v4(),
            "expectedStateRevision": "28",
            "passed": true,
            "issues": "",
            "citations": [],
        }))
        .unwrap();
        assert!(approved.passed);
        assert_eq!(approved.issues.as_deref(), Some(""));
        assert_eq!(approved.citations, vec![]);
    }

    /// 问答提交的请求体形状必须与界面 `answerQuestion` 真正发出的 JSON 一致。
    ///
    /// 实战里发现过反例：接口把领域类型 `UserQuestionAnswers` 当作同名字段，线上
    /// 形状变成 `answers.answers`，而界面按 `answers: [...]` 提交，于是每次提交都是
    /// 422——模型问一次问题就把整章卡在 `waiting_answer`，界面只给出一个永远不会
    /// 成功的提交按钮。
    #[test]
    fn answer_question_request_accepts_the_payload_the_ui_sends() {
        let command_id = Uuid::new_v4();
        let parsed: AnswerQuestionRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedStateRevision": "38",
            "answers": [
                {
                    "questionId": "q1",
                    "selectedOptionIds": ["night"],
                    "customText": "当夜约 2:10，巡道工因暴雨上线检查道岔。",
                },
                {
                    "questionId": "q2",
                    "selectedOptionIds": ["dialogue_only"],
                    "customText": null,
                },
            ],
        }))
        .unwrap();
        assert_eq!(parsed.command_id, command_id);
        assert_eq!(parsed.expected_state_revision, 38);
        assert_eq!(parsed.answers.len(), 2);
        assert_eq!(parsed.answers[0].question_id, "q1");
        assert_eq!(parsed.answers[0].selected_option_ids, ["night"]);
        assert_eq!(parsed.answers[1].custom_text, None);
        // 逐题答案仍然拒绝未知字段：多出来的一层 `answers` 不能静默通过。
        assert!(
            serde_json::from_value::<AnswerQuestionRequest>(json!({
                "commandId": command_id,
                "expectedStateRevision": "38",
                "answers": {
                    "answers": [
                        { "questionId": "q1", "selectedOptionIds": [], "customText": null },
                    ],
                },
            }))
            .is_err()
        );
    }

    /// Embedding 设置与连接测试的请求体形状必须与界面真正发出的 JSON 一致。
    ///
    /// 这条路径此前只有 store 级单测：`/api/questions/{id}/answer` 那个「服务端期望
    /// `answers.answers`、界面发 `answers: [...]`」的缺陷正是同一种漏检。这里用界面
    /// 真实抓到的报文（docs/03 §40）钉住两侧形状，而不只是 Rust 字段。
    #[test]
    fn embedding_requests_accept_the_payload_the_ui_sends() {
        let command_id = Uuid::new_v4();
        let save: knowledge::SaveEmbeddingSettingsRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedRevision": "2",
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": null,
        }))
        .unwrap();
        assert_eq!(save.command_id, command_id);
        assert_eq!(save.expected_revision, 2);
        assert_eq!(save.api_base_url, "http://127.0.0.1:8899/v1");
        assert_eq!(save.model_id, "stub-embed");
        assert_eq!(save.dimension, 8);
        // 保存命令里的 `apiKey` 是三态：`null` = 清除，省略 = 保留，字符串 = 替换。
        assert_eq!(save.api_key, CredentialPatch::Clear);

        let omitted: knowledge::SaveEmbeddingSettingsRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedRevision": "1",
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
        }))
        .unwrap();
        assert_eq!(omitted.api_key, CredentialPatch::Keep);

        let with_key: knowledge::SaveEmbeddingSettingsRequest = serde_json::from_value(json!({
            "commandId": command_id,
            "expectedRevision": "0",
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": "probe-embed-key-001",
        }))
        .unwrap();
        assert_eq!(
            with_key.api_key,
            CredentialPatch::Replace("probe-embed-key-001".to_owned())
        );

        // 连接测试不是命令、不落库，它的 `apiKey` 只有两种含义（给一把临时的 / 用库里
        // 那把），所以刻意不用三态：省略与 `null` 都表示「用库中现有密钥」。
        let probe_with_key: knowledge::TestEmbeddingRequest = serde_json::from_value(json!({
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": "probe-embed-key-001",
        }))
        .unwrap();
        assert_eq!(
            probe_with_key.api_key.as_deref(),
            Some("probe-embed-key-001")
        );

        let test: knowledge::TestEmbeddingRequest = serde_json::from_value(json!({
            "apiBaseUrl": "http://127.0.0.1:8899/v1",
            "modelId": "stub-embed",
            "dimension": "8",
            "apiKey": null,
        }))
        .unwrap();
        assert_eq!(test.dimension, 8);
        assert_eq!(test.api_key, None);

        // 长整数只认十进制字符串；界面把 `dimension` 当字符串发，服务端不能顺手把
        // 数字也收下，否则两侧会在超过 JS 安全整数时悄悄分叉。
        assert!(
            serde_json::from_value::<knowledge::TestEmbeddingRequest>(json!({
                "apiBaseUrl": "http://127.0.0.1:8899/v1",
                "modelId": "stub-embed",
                "dimension": 8,
                "apiKey": null,
            }))
            .is_err()
        );
    }
}
