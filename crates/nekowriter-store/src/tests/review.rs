//! 审核：拒收轮次计数、引用校验、委派审核只生效一次、细纲/终审工作流。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

/// 委托审核的每一轮（含达上限的那一轮）落库的计数都必须等于这一轮事件公布的
/// 计数。
///
/// 修复前达上限的那一轮只写失败状态、不写计数，子 Run 行里停在 2，而它刚写的
/// 两条事件都说 3——落库状态与自己发出的事件互相矛盾。
#[tokio::test]
async fn rejected_review_round_persists_the_counter_it_publishes() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (version_id, review_id, delegation_run_id) =
        delegated_background_review(&store, "审核协议纠正").await;

    for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id: delegation_run_id,
                    settings_snapshot: json!({ "model": "review-protocol-test" }),
                    prompt_snapshot: json!({ "key": "review.preparation.background" }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let outcome = store
                .record_model_protocol_correction(
                    ProtocolCorrectionTarget::DelegatedReview { review_id },
                    RejectedRoundRecord {
                        provider_request_id: Some(format!("review-provider-{attempt}")),
                        usage: json!({ "totalTokens": "1" }),
                        assistant_message: crate::agent_messages::phase_submission_test_message(
                            request.id,
                            "submit_review",
                            r#"{"passed":true,"confidence":0.82,"issues":["【建议】章节清单还不完整"]}"#,
                        ),
                        error: json!({
                            "kind": "protocol_error",
                            "message": "审核提交与本阶段合同不符",
                        }),
                        correction: "请重新调用 submit_review。".to_owned(),
                        tool_result_code: "PHASE_SUBMISSION_INVALID".to_owned(),
                    },
                )
                .await
                .unwrap();
        assert!(outcome.newly_applied);
        assert_eq!(outcome.consecutive_protocol_errors, attempt);
        assert_eq!(outcome.requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);

        let persisted = persisted_protocol_error_count(&store, delegation_run_id).await;
        assert_eq!(persisted, attempt);
        assert_eq!(
            published_protocol_error_count(&store, request.id).await,
            persisted,
            "第 {attempt} 轮：落库计数与同一轮事件公布的计数必须一致"
        );
    }

    // 达上限那轮之后：子 Run 失败但计数仍然落库，Review 重新开放。
    let child_run = entity::run::Entity::find_by_id(db_id(delegation_run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child_run.status, "failed");
    assert_eq!(
        child_run.consecutive_protocol_errors,
        MAX_CONSECUTIVE_PROTOCOL_ERRORS
    );
    let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
    let reopened = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
        .and_then(|artifact| artifact.review.as_ref())
        .unwrap();
    assert_eq!(reopened.status, ReviewStatus::Open);
}

#[tokio::test]
async fn review_decision_persists_validated_citations_and_rejects_bad_ones() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let project = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("决策引用", "旧码头", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = project.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部关于旧码头与匿名报纸的悬疑小说。").unwrap(),
        )
        .await
        .unwrap();
    let mut snapshot = store
        .start_preparation(version_id, Uuid::new_v4(), 0)
        .await
        .unwrap();
    let preparation_run_id = snapshot.workflow.active_run_id.unwrap();
    store
        .append_agent_messages(
            preparation_run_id,
            vec![NewAgentMessage {
                role: AgentMessageRole::Assistant,
                content: json!({
                    "kind": "text",
                    "role": "assistant",
                    "content": "我会先形成四项准备产物。"
                }),
                request_id: None,
                tool_call_id: None,
            }],
        )
        .await
        .unwrap();
    snapshot = store
        .submit_preparation_artifacts(
            version_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            vec![
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Outline,
                    "# 粗纲\n\n第一章：旧码头的报纸",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Characters,
                    "林舟：追查旧报纸的记者。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "资料：1998 年港城晚报。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Background,
                    "海港城连续七日暴雨，旧码头即将拆除。",
                )
                .unwrap(),
            ],
            Some(vec![
                nekowriter_core::ChapterPlanDraft::new(
                    "旧码头的报纸",
                    "林舟在暴雨中收到匿名报纸并决定追查。",
                )
                .unwrap(),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);

    let background = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
        .unwrap();
    let review_id = background.review.as_ref().unwrap().id;
    let document_id = background.document_id;
    let revision_id = background.revision_id.unwrap();
    let content = background.content.clone().unwrap();
    assert_eq!(content, "海港城连续七日暴雨，旧码头即将拆除。");

    // 负向：quote 与修订原文不一致时必须拒绝，且审核保持未决。
    assert!(matches!(
        store
            .decide_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                vec![nekowriter_core::FeedbackCitation::Document {
                    document_id,
                    revision_id,
                    range: nekowriter_core::TextCitationRange {
                        start_char: 0,
                        end_char: 2,
                        quote: "暴雨".to_owned(),
                    },
                    chapter_id: None,
                }],
            )
            .await,
        Err(StoreError::InvalidCitation(_))
    ));
    let after_rejection = store.get_preparation_snapshot(version_id).await.unwrap();
    assert_eq!(
        after_rejection
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.status.clone()),
        Some(ReviewStatus::Open)
    );

    // 负向：空文字且没有引用仍不能形成一条可执行的退回意见。
    assert!(matches!(
        store
            .decide_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(false, "").unwrap(),
                vec![],
            )
            .await,
        Err(StoreError::InvalidReviewDecision(_))
    ));

    // 正向：退回可以只附一条经验证的引用，不必再拼一段重复文字。
    snapshot = store
        .decide_review(
            review_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            HumanReviewDecision::new(false, "").unwrap(),
            vec![nekowriter_core::FeedbackCitation::Document {
                document_id,
                revision_id,
                range: nekowriter_core::TextCitationRange {
                    start_char: 7,
                    end_char: 9,
                    quote: "暴雨".to_owned(),
                },
                chapter_id: None,
            }],
        )
        .await
        .unwrap();
    assert_eq!(
        snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.status.clone()),
        Some(ReviewStatus::Rejected)
    );

    let review_model = entity::review::Entity::find_by_id(db_id(review_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    let user_input: serde_json::Value =
        serde_json::from_str(review_model.user_input_json.as_deref().unwrap()).unwrap();
    assert_eq!(user_input["source"], "user");
    assert_eq!(user_input["citations"][0]["kind"], "document");
    assert_eq!(user_input["citations"][0]["quote"], "暴雨");
    assert_eq!(
        user_input["citations"][0]["revisionId"],
        revision_id.to_string()
    );
}

#[tokio::test]
async fn delegated_review_applies_once_and_cancellation_rejects_late_results() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let project = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("审核委托", "港城旧报纸", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = project.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部围绕旧码头和匿名报纸的悬疑小说。 ").unwrap(),
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
                    "港城的旧码头将在雨季后拆除。",
                )
                .unwrap(),
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "旧报纸来自 1998 年的港城晚报，来源尚待核验。",
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
    let background_review_id = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
        .and_then(|artifact| artifact.review.as_ref())
        .map(|review| review.id)
        .unwrap();
    let delegation = store
        .delegate_review(
            background_review_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            "检查设定是否足以支撑第一章。".to_owned(),
            Vec::new(),
        )
        .await
        .unwrap();
    let task = store
        .delegated_review_task(background_review_id, delegation.run_id)
        .await
        .unwrap();
    assert!(matches!(
        task.kind,
        DelegatedReviewKind::Preparation(PreparationArtifactKind::Background)
    ));
    assert_eq!(task.user_notes, "检查设定是否足以支撑第一章。");
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: delegation.run_id,
                settings_snapshot: json!({ "model": "review-test" }),
                prompt_snapshot: json!({ "key": "review.preparation.background" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    snapshot = store
        .commit_delegated_review_result(
            background_review_id,
            delegation.run_id,
            request.id,
            Some("review-provider-1".to_owned()),
            json!({}),
            crate::agent_messages::phase_submission_test_message(
                request.id,
                "submit_review",
                r#"{"passed":true,"confidence":0.91,"issues":[]}"#,
            ),
            true,
            0.91,
            Vec::new(),
        )
        .await
        .unwrap();
    let background_review = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
        .and_then(|artifact| artifact.review.as_ref())
        .unwrap();
    assert_eq!(background_review.status, ReviewStatus::Approved);
    assert_eq!(background_review.passed, Some(true));
    let replayed = store
        .commit_delegated_review_result(
            background_review_id,
            delegation.run_id,
            request.id,
            Some("review-provider-1".to_owned()),
            json!({}),
            crate::agent_messages::phase_submission_test_message(
                request.id,
                "submit_review",
                r#"{"passed":true,"confidence":0.91,"issues":[]}"#,
            ),
            true,
            0.91,
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(replayed, snapshot);

    let research_review_id = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
        .and_then(|artifact| artifact.review.as_ref())
        .map(|review| review.id)
        .unwrap();
    let delegation = store
        .delegate_review(
            research_review_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            String::new(),
            Vec::new(),
        )
        .await
        .unwrap();
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: delegation.run_id,
                settings_snapshot: json!({ "model": "review-test" }),
                prompt_snapshot: json!({ "key": "review.preparation.research" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let cancellation = store
        .cancel_review_delegation(
            research_review_id,
            Uuid::new_v4(),
            delegation.snapshot.workflow.state_revision,
        )
        .await
        .unwrap();
    let research_review = cancellation
        .snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
        .and_then(|artifact| artifact.review.as_ref())
        .unwrap();
    assert_eq!(research_review.status, ReviewStatus::Open);
    assert!(matches!(
        store
            .commit_delegated_review_result(
                research_review_id,
                delegation.run_id,
                request.id,
                None,
                json!({}),
                crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.8,"issues":[]}"#,
                ),
                true,
                0.8,
                Vec::new(),
            )
            .await,
        Err(StoreError::ReviewAlreadyResolved)
    ));

    store
        .delegate_review(
            research_review_id,
            Uuid::new_v4(),
            cancellation.snapshot.workflow.state_revision,
            "重启恢复不应保留悬挂的审核请求。".to_owned(),
            Vec::new(),
        )
        .await
        .unwrap();
    drop(store);
    let reopened = Store::open(data_dir.path()).await.unwrap();
    let recovered = reopened.get_preparation_snapshot(version_id).await.unwrap();
    assert_eq!(
        recovered
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
            .and_then(|artifact| artifact.review.as_ref())
            .unwrap()
            .status,
        ReviewStatus::Open
    );
}

#[tokio::test]
async fn delegated_outline_edit_and_final_reviews_apply_their_workflows() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let project = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("审核流程", "雨夜车站", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = project.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部围绕雨夜车站和失踪信件展开的悬疑小说。").unwrap(),
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
                    "用户提供了一封没有署名的旧信，出处尚待核验。",
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
    let outline_content = "1. 沈宁辨认旧信上的车站印章。\n2. 她在候车室发现姐姐留下的胶卷。\n3. 末班车鸣笛，迫使她决定是否追上寄信人。";
    let outline = store
        .submit_chapter_outline(
            chapter_id,
            Uuid::new_v4(),
            outline_started.state_revision,
            nekowriter_core::NewChapterOutline::new(outline_content).unwrap(),
        )
        .await
        .unwrap();
    let outline_review_id = outline.outline.review.as_ref().unwrap().id;
    let outline_delegation = store
        .delegate_review(
            outline_review_id,
            Uuid::new_v4(),
            outline.workflow.state_revision,
            "检查细纲是否保留悬念。".to_owned(),
            Vec::new(),
        )
        .await
        .unwrap();
    let outline_task = store
        .delegated_review_task(outline_review_id, outline_delegation.run_id)
        .await
        .unwrap();
    assert_eq!(outline_task.kind, DelegatedReviewKind::ChapterOutline);
    assert_eq!(outline_task.target_content, outline_content);
    assert!(outline_task.chapter_context.contains("末班车"));
    let outline_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: outline_delegation.run_id,
                settings_snapshot: json!({ "model": "review-test" }),
                prompt_snapshot: json!({ "key": "review.chapter.outline" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let after_outline = store
        .commit_delegated_review_result(
            outline_review_id,
            outline_delegation.run_id,
            outline_request.id,
            Some("review-provider-outline".to_owned()),
            json!({}),
            crate::agent_messages::phase_submission_test_message(
                outline_request.id,
                "submit_review",
                r#"{"passed":true,"confidence":0.82,"issues":[]}"#,
            ),
            true,
            0.82,
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(after_outline.workflow.stage, WorkflowStage::WritingDrafting);
    assert_eq!(after_outline.workflow.execution, ExecutionStatus::Queued);
    let body_run_id = after_outline.workflow.active_run_id.unwrap();
    let approved_outline = store.get_chapter_outline(chapter_id).await.unwrap();
    assert_eq!(
        approved_outline.outline.review.as_ref().unwrap().status,
        ReviewStatus::Approved
    );
    assert_eq!(
        approved_outline.context_fold.as_ref().unwrap().plain_text,
        outline_content
    );

    let approve_required = store
        .set_approval_mode(
            version_id,
            Uuid::new_v4(),
            after_outline.workflow.state_revision,
            nekowriter_core::ApprovalMode::ApproveRequired,
        )
        .await
        .unwrap();
    let body_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: body_run_id,
                settings_snapshot: json!({ "model": "writing-test" }),
                prompt_snapshot: json!({ "key": "chapter.write" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let proposal_content = "沈宁攥紧胶卷，踏上刚刚进站的末班车。";
    let applied = store
        .execute_chapter_body_agent_tool_call(
            body_request.id,
            chapter_id,
            Some("writing-provider-proposal".to_owned()),
            json!({}),
            ChapterBodyAgentToolCall {
                provider_call_id: "proposal-call".to_owned(),
                name: "append_content".to_owned(),
                arguments: json!({ "content": proposal_content }),
            },
        )
        .await
        .unwrap();
    assert!(!applied.newly_resolved);
    let pending = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(pending.workflow.stage, WorkflowStage::WritingDrafting);
    assert_eq!(pending.workflow.execution, ExecutionStatus::WaitingReview);
    assert_eq!(
        pending.workflow.active_run_id,
        approve_required.active_run_id
    );
    let proposal = pending.pending_proposal.as_ref().unwrap();
    let edit_review = pending.pending_proposal_review.as_ref().unwrap();
    assert_eq!(edit_review.status, ReviewStatus::Open);
    // 委托时固化的引用必须逐字复核：错误的候选哈希不能进入子 Run。
    assert!(matches!(
        store
            .delegate_review(
                edit_review.id,
                Uuid::new_v4(),
                pending.workflow.state_revision,
                String::new(),
                vec![nekowriter_core::FeedbackCitation::Candidate {
                    tool_call_id: proposal.tool_call_id,
                    candidate_hash: "0".repeat(64),
                    candidate_range: nekowriter_core::TextCitationRange {
                        start_char: 4,
                        end_char: 6,
                        quote: "胶卷".to_owned(),
                    },
                }],
            )
            .await,
        Err(StoreError::InvalidCitation(_))
    ));
    let edit_delegation = store
        .delegate_review(
            edit_review.id,
            Uuid::new_v4(),
            pending.workflow.state_revision,
            "检查这次追加是否保留人物动机。".to_owned(),
            vec![nekowriter_core::FeedbackCitation::Candidate {
                tool_call_id: proposal.tool_call_id,
                candidate_hash: proposal.candidate_hash.clone(),
                candidate_range: nekowriter_core::TextCitationRange {
                    start_char: 4,
                    end_char: 6,
                    quote: "胶卷".to_owned(),
                },
            }],
        )
        .await
        .unwrap();
    let edit_task = store
        .delegated_review_task(edit_review.id, edit_delegation.run_id)
        .await
        .unwrap();
    assert_eq!(edit_task.kind, DelegatedReviewKind::ChapterEdit);
    assert_eq!(edit_task.base_content, "");
    assert_eq!(edit_task.target_content, proposal.candidate_content);
    assert_eq!(
        edit_task.citations,
        vec![nekowriter_core::FeedbackCitation::Candidate {
            tool_call_id: proposal.tool_call_id,
            candidate_hash: proposal.candidate_hash.clone(),
            candidate_range: nekowriter_core::TextCitationRange {
                start_char: 4,
                end_char: 6,
                quote: "胶卷".to_owned(),
            },
        }]
    );
    let edit_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: edit_delegation.run_id,
                settings_snapshot: json!({ "model": "review-test" }),
                prompt_snapshot: json!({ "key": "review.chapter.edit" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let after_edit = store
        .commit_delegated_review_result(
            edit_review.id,
            edit_delegation.run_id,
            edit_request.id,
            Some("review-provider-edit".to_owned()),
            json!({}),
            crate::agent_messages::phase_submission_test_message(
                edit_request.id,
                "submit_review",
                r#"{"passed":true,"confidence":0.77,"issues":[]}"#,
            ),
            true,
            0.77,
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(after_edit.workflow.stage, WorkflowStage::WritingDrafting);
    assert_eq!(after_edit.workflow.execution, ExecutionStatus::Queued);
    let edited_body = store.get_chapter_body(chapter_id).await.unwrap();
    assert_eq!(edited_body.body.content.as_deref(), Some(proposal_content));
    assert!(edited_body.pending_proposal.is_none());
    assert!(edited_body.pending_proposal_review.is_none());
    let body_revision_id = edited_body.body.revision_id.unwrap();
    let revision_view = store
        .get_document_revision(edited_body.body.document_id, body_revision_id)
        .await
        .unwrap();
    assert_eq!(revision_view.content, proposal_content);
    assert_eq!(revision_view.role, CHAPTER_BODY_ROLE);
    assert_eq!(revision_view.chapter_id, Some(chapter_id));
    assert!(revision_view.is_current);
    assert!(matches!(
        store
            .get_document_revision(edited_body.body.document_id, Uuid::new_v4())
            .await,
        Err(StoreError::DocumentRevisionNotFound)
    ));

    let final_review = store
        .submit_chapter_final_review(
            chapter_id,
            Uuid::new_v4(),
            after_edit.workflow.state_revision,
        )
        .await
        .unwrap();
    let final_review_id = final_review.body.review.as_ref().unwrap().id;
    let final_delegation = store
        .delegate_review(
            final_review_id,
            Uuid::new_v4(),
            final_review.workflow.state_revision,
            "检查结尾是否完成本章目标。".to_owned(),
            Vec::new(),
        )
        .await
        .unwrap();
    let final_task = store
        .delegated_review_task(final_review_id, final_delegation.run_id)
        .await
        .unwrap();
    assert_eq!(final_task.kind, DelegatedReviewKind::ChapterFinal);
    assert_eq!(final_task.target_content, proposal_content);
    assert_eq!(final_task.chapter_outline, outline_content);
    let final_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: final_delegation.run_id,
                settings_snapshot: json!({ "model": "review-test" }),
                prompt_snapshot: json!({ "key": "review.chapter.final" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let done = store
        .commit_delegated_review_result(
            final_review_id,
            final_delegation.run_id,
            final_request.id,
            Some("review-provider-final".to_owned()),
            json!({}),
            crate::agent_messages::phase_submission_test_message(
                final_request.id,
                "submit_review",
                r#"{"passed":true,"confidence":0.94,"issues":[]}"#,
            ),
            true,
            0.94,
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(done.workflow.stage, WorkflowStage::WritingChapterDone);
    assert_eq!(done.workflow.execution, ExecutionStatus::Idle);
    assert_eq!(
        store.list_chapters(version_id).await.unwrap()[0].status,
        nekowriter_core::ChapterStatus::Approved
    );
}
