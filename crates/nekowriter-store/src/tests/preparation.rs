//! 准备阶段：稿件提交、修订轮只接受被拒的稿、终止失败要配上从未跑过的工具调用。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn preparation_model_submission_is_committed_with_its_response_and_reviews() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("自动准备", "模型完成后必须原子进入审核", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部关于旧车站与重逢的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let workflow = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(created.version.id, Uuid::new_v4(), workflow.state_revision)
        .await
        .unwrap();
    let run_id = started.workflow.active_run_id.unwrap();
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "test-model" }),
                prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                context_snapshot: json!({ "messages": [] }),
            },
            vec![NewAgentMessage {
                role: AgentMessageRole::User,
                content: json!({
                    "kind": "text",
                    "role": "user",
                    "content": "请生成四项准备产物。"
                }),
                request_id: None,
                tool_call_id: None,
            }],
        )
        .await
        .unwrap();
    let artifacts = PreparationArtifactKind::ALL
        .into_iter()
        .map(|kind| {
            PreparationArtifactInput::new(kind, format!("{} 的可审核内容", kind.label())).unwrap()
        })
        .collect::<Vec<_>>();
    let chapter_plan = vec![ChapterPlanDraft::new("第一章：重逢", "在旧车站相遇").unwrap()];

    let submitted = store
        .commit_preparation_model_submission(
            request.id,
            Some("provider-request-1".to_owned()),
            json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
            crate::agent_messages::phase_submission_test_message(
                request.id,
                "submit_preparation",
                r#"{"artifacts":[],"chapter_plan":[]}"#,
            ),
            artifacts,
            Some(chapter_plan),
        )
        .await
        .unwrap();
    assert_eq!(
        submitted.workflow.stage,
        nekowriter_core::WorkflowStage::PreparingReviewing
    );
    assert_eq!(
        submitted.workflow.execution,
        nekowriter_core::ExecutionStatus::WaitingReview
    );
    assert!(
        submitted
            .artifacts
            .iter()
            .all(|artifact| artifact.revision_id.is_some() && artifact.review.is_some())
    );
    // 阶段提交在账本里是配对的两条：提交工具的调用与它被接受的结果。
    assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
    assert_eq!(
        store.list_model_requests(run_id).await.unwrap()[0].status,
        ModelRequestStatus::Succeeded
    );
    let run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "waiting");
    assert_eq!(run.request_count, 1);

    let replayed = store
        .commit_preparation_model_submission(
            request.id,
            Some("provider-request-1".to_owned()),
            json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
            crate::agent_messages::phase_submission_test_message(
                request.id,
                "submit_preparation",
                r#"{"artifacts":[]}"#,
            ),
            Vec::new(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(replayed.workflow, submitted.workflow);
    assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
}

/// 准备修订轮：模型一轮只提交「需要修订」的那些产物，而且只有粗纲也在待修订
/// 之列时才允许附带 `chapter_plan`。系统消息里的合同与解析器按同一份清单生成，
/// 所以「照合同提交」必须被 Store 接受，提交其它项必须被明确拒绝且不留痕迹。
#[tokio::test]
async fn preparation_revision_round_accepts_only_the_rejected_artifacts() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("准备修订", "只有被拒绝的产物需要重写", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let version_id = created.version.id;
    store
        .save_novel_brief(
            version_id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部关于旧车站与重逢的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let workflow = store
        .get_version_snapshot(version_id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(version_id, Uuid::new_v4(), workflow.state_revision)
        .await
        .unwrap();
    let run_id = started.workflow.active_run_id.unwrap();
    let first_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "test-model" }),
                prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let artifacts = PreparationArtifactKind::ALL
        .into_iter()
        .map(|kind| {
            PreparationArtifactInput::new(kind, format!("{} 的首版内容", kind.label())).unwrap()
        })
        .collect::<Vec<_>>();
    let mut snapshot = store
        .commit_preparation_model_submission(
            first_request.id,
            Some("provider-request-1".to_owned()),
            json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
            crate::agent_messages::phase_submission_test_message(
                first_request.id,
                "submit_preparation",
                r#"{"artifacts":[],"chapter_plan":[]}"#,
            ),
            artifacts,
            Some(vec![
                ChapterPlanDraft::new("第一章：重逢", "旧车站的相遇").unwrap(),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);

    // 三项通过、资料被拒绝：工作流进入准备修订，同一个 Run 重新排队。
    for kind in [
        PreparationArtifactKind::Background,
        PreparationArtifactKind::Characters,
        PreparationArtifactKind::Outline,
    ] {
        let review_id = snapshot
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == kind)
            .and_then(|artifact| artifact.review.as_ref())
            .map(|review| review.id)
            .unwrap();
        snapshot = store
            .decide_preparation_review(
                review_id,
                Uuid::new_v4(),
                snapshot.workflow.state_revision,
                HumanReviewDecision::new(true, "").unwrap(),
                Vec::new(),
            )
            .await
            .unwrap();
    }
    let research_review_id = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
        .and_then(|artifact| artifact.review.as_ref())
        .map(|review| review.id)
        .unwrap();
    snapshot = store
        .decide_preparation_review(
            research_review_id,
            Uuid::new_v4(),
            snapshot.workflow.state_revision,
            HumanReviewDecision::new(false, "资料需要补充 1998 年港城晚报的原件出处。").unwrap(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingRevising);
    assert_eq!(snapshot.workflow.active_run_id, Some(run_id));

    let revision_request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "test-model" }),
                prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    // 负向：修订轮不能覆盖不需要修订的产物，失败不留痕。
    assert!(matches!(
        store
            .commit_preparation_model_submission(
                revision_request.id,
                Some("provider-request-2".to_owned()),
                json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
                crate::agent_messages::phase_submission_test_message(
                    revision_request.id,
                    "submit_preparation",
                    "{\"artifacts\":[{\"kind\":\"background\",\"content\":\"越权重写\"}]}",
                ),
                vec![
                    PreparationArtifactInput::new(
                        PreparationArtifactKind::Background,
                        "越权重写的背景",
                    )
                    .unwrap()
                ],
                None,
            )
            .await,
        Err(StoreError::InvalidPreparationArtifacts(_))
    ));

    // 正向：只提交被拒绝的资料项，不带 chapter_plan，照合同提交即可通过。
    snapshot = store
        .commit_preparation_model_submission(
            revision_request.id,
            Some("provider-request-2".to_owned()),
            json!({ "inputTokens": "21", "outputTokens": "8", "totalTokens": "29" }),
            crate::agent_messages::phase_submission_test_message(
                revision_request.id,
                "submit_preparation",
                "{\"artifacts\":[{\"kind\":\"research\",\"content\":\"补充后的资料\"}]}",
            ),
            vec![
                PreparationArtifactInput::new(
                    PreparationArtifactKind::Research,
                    "资料：1998 年港城晚报原件第 3 版。",
                )
                .unwrap(),
            ],
            None,
        )
        .await
        .unwrap();
    assert_eq!(snapshot.workflow.stage, WorkflowStage::PreparingReviewing);
    let research = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Research)
        .unwrap();
    assert_eq!(
        research.content.as_deref(),
        Some("资料：1998 年港城晚报原件第 3 版。")
    );
    assert_eq!(
        research.review.as_ref().map(|review| review.status.clone()),
        Some(ReviewStatus::Open)
    );
}

/// 请求进入终态时也必须补写配对结果：终态之后不会再有工具结果到达，只留下
/// assistant 调用同样会让这个 Run 再也组装不出下一次请求。
#[tokio::test]
async fn terminal_failure_pairs_tool_calls_that_never_ran() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (_version_id, run_id) = in_flight_preparation_run(&store, "终态失败配对").await;
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "terminal-failure" }),
                prompt_snapshot: json!({ "contentHash": "terminal-failure" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let resolved = store
        .resolve_model_request_with_assistant_message(
            request.id,
            ModelRequestResolution::Failed {
                provider_request_id: Some("terminal-failure-provider".to_owned()),
                error: json!({ "kind": "provider_error", "message": "流在响应完成前中断" }),
            },
            crate::agent_messages::phase_submission_test_message(
                request.id,
                "submit_preparation",
                r#"{"artifacts":[],"chapter_plan":[]}"#,
            ),
        )
        .await
        .unwrap();
    assert_eq!(resolved.status, ModelRequestStatus::Failed);

    // 终态失败不推进协议计数；配对结果把「这次调用没有执行」的真实结局交给模型。
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 0);
    let messages = store.list_agent_messages(run_id).await.unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, AgentMessageRole::Assistant);
    assert_eq!(messages[1].role, AgentMessageRole::Tool);
    assert_eq!(messages[1].tool_call_id.as_deref(), Some("call-submit-1"));
    let paired = messages[1].content["content"].as_str().unwrap();
    assert!(paired.contains("TOOL_NOT_EXECUTED"), "{paired}");
    assert!(paired.contains("流在响应完成前中断"), "{paired}");
}
