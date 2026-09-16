//! 写作运行：配额边界、开中断恢复、请求上限停止、显式停止、协议错误与经济性拒绝。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn model_request_dispatch_never_crosses_the_persisted_allowance_boundary() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("请求额度", "达到额度后不能再发出请求", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个结局明确的短篇小说").unwrap(),
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
    let run_model = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    let allowance = run_model.request_allowance_end;
    let mut active_run = run_model.into_active_model();
    active_run.request_count = Set(allowance);
    active_run.update(&store.database).await.unwrap();

    let prepared = store
        .prepare_model_request(NewModelRequest {
            run_id,
            settings_snapshot: json!({ "model": "test-model" }),
            prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
            context_snapshot: json!({ "messages": [] }),
        })
        .await
        .unwrap();
    assert!(matches!(
        store.dispatch_model_request(prepared.id).await,
        Err(StoreError::RequestAllowanceExhausted {
            request_count,
            request_allowance_end,
        }) if request_count == allowance && request_allowance_end == allowance
    ));
    let requests = store.list_model_requests(run_id).await.unwrap();
    assert_eq!(requests, vec![prepared]);
}

#[tokio::test]
async fn startup_recovery_marks_dispatched_runs_interrupted_and_explicit_continue_regrants_allowance()
 {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("中断恢复", "重开后不自动重新计费", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个雨夜车站重逢的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let before_start = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(
            created.version.id,
            Uuid::new_v4(),
            before_start.state_revision,
        )
        .await
        .unwrap();
    let run_id = started.workflow.active_run_id.unwrap();
    let dispatched = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "recovery-test" }),
                prompt_snapshot: json!({ "contentHash": "recovery-test" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);
    drop(store);

    let reopened = Store::open(data_dir.path()).await.unwrap();
    let interrupted = reopened
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    assert_eq!(interrupted.execution, ExecutionStatus::Interrupted);
    let requests = reopened.list_model_requests(run_id).await.unwrap();
    assert_eq!(requests[0].status, ModelRequestStatus::Unknown);
    let interrupted_run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&reopened.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(interrupted_run.status, "interrupted");
    assert_eq!(interrupted_run.request_count, 1);

    let continue_command_id = Uuid::new_v4();
    let resumed = reopened
        .continue_writing_run(run_id, continue_command_id, interrupted.state_revision)
        .await
        .unwrap();
    assert_eq!(resumed.execution, ExecutionStatus::Queued);
    assert_eq!(
        reopened
            .continue_writing_run(run_id, continue_command_id, interrupted.state_revision,)
            .await
            .unwrap(),
        resumed
    );
    let resumed_run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&reopened.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed_run.status, "queued");
    assert_eq!(resumed_run.resume_count, 1);
    assert_eq!(resumed_run.request_allowance_end, 101);
    assert_eq!(resumed_run.consecutive_protocol_errors, 0);

    let next = reopened
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "recovery-test" }),
                prompt_snapshot: json!({ "contentHash": "recovery-test-next" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(next.attempt_no, 2);
    assert_eq!(next.status, ModelRequestStatus::Dispatched);
}

#[tokio::test]
async fn request_limit_stop_requires_explicit_continue_before_another_request() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("额度停止", "达到上限后等待显式继续", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个有明确收束的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let initial = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(created.version.id, Uuid::new_v4(), initial.state_revision)
        .await
        .unwrap();
    let run_id = started.workflow.active_run_id.unwrap();
    let run_model = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    let allowance = run_model.request_allowance_end;
    let mut exhausted_run = run_model.into_active_model();
    exhausted_run.request_count = Set(allowance);
    exhausted_run.update(&store.database).await.unwrap();

    assert!(
        store
            .stop_queued_model_run(
                run_id,
                json!({ "kind": "request_limit", "message": "额度已用完" }),
            )
            .await
            .unwrap()
    );
    let stopped = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    assert_eq!(stopped.execution, ExecutionStatus::Stopped);
    let resumed = store
        .continue_writing_run(run_id, Uuid::new_v4(), stopped.state_revision)
        .await
        .unwrap();
    assert_eq!(resumed.execution, ExecutionStatus::Queued);
    let resumed_run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed_run.request_count, allowance);
    assert_eq!(resumed_run.request_allowance_end, allowance + 100);
    assert_eq!(resumed_run.resume_count, 1);
}

#[tokio::test]
async fn explicit_stop_cancels_the_dispatched_attempt_and_rejects_its_late_callback() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("用户停止", "停止不能让迟到回调继续写作", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个雨夜重逢的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let initial = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(created.version.id, Uuid::new_v4(), initial.state_revision)
        .await
        .unwrap();
    let run_id = started.workflow.active_run_id.unwrap();
    let dispatched = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "stop-test" }),
                prompt_snapshot: json!({ "contentHash": "stop-test" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let in_flight = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    assert_eq!(in_flight.execution, ExecutionStatus::RequestInFlight);

    let stop_command_id = Uuid::new_v4();
    let stopped = store
        .stop_writing_run(run_id, stop_command_id, in_flight.state_revision)
        .await
        .unwrap();
    assert_eq!(stopped.workflow.execution, ExecutionStatus::Stopped);
    assert_eq!(
        stopped.workflow.wait_reason,
        Some(RunWaitReason::UserStopped)
    );
    assert_eq!(
        stopped.workflow.resume_execution,
        Some(ExecutionStatus::RequestInFlight)
    );
    assert_eq!(stopped.cancelled_request_ids, vec![dispatched.id]);
    assert_eq!(
        store
            .stop_writing_run(run_id, stop_command_id, in_flight.state_revision)
            .await
            .unwrap(),
        stopped
    );

    let requests = store.list_model_requests(run_id).await.unwrap();
    assert_eq!(requests[0].status, ModelRequestStatus::Cancelled);
    assert_eq!(requests[0].error.as_ref().unwrap()["kind"], "user_stopped");
    assert!(matches!(
        store
            .resolve_model_request(
                dispatched.id,
                ModelRequestResolution::Failed {
                    provider_request_id: None,
                    error: json!({ "kind": "late_callback" }),
                },
            )
            .await,
        Err(StoreError::InvalidModelRequestState(_))
    ));
    let after_late_callback = store
        .get_preparation_snapshot(created.version.id)
        .await
        .unwrap();
    assert_eq!(after_late_callback.workflow, stopped.workflow);
    assert!(
        after_late_callback
            .artifacts
            .iter()
            .all(|artifact| artifact.revision_id.is_none())
    );

    let resumed = store
        .continue_writing_run(run_id, Uuid::new_v4(), stopped.workflow.state_revision)
        .await
        .unwrap();
    assert_eq!(resumed.execution, ExecutionStatus::Queued);
    let next = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "stop-test" }),
                prompt_snapshot: json!({ "contentHash": "after-stop" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(next.attempt_no, 2);
    assert_eq!(next.status, ModelRequestStatus::Dispatched);
}

#[tokio::test]
async fn protocol_errors_requeue_with_paired_results_then_stop_at_the_limit() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("协议纠正", "连续格式错误必须有限重试", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个结局清晰的短篇小说").unwrap(),
        )
        .await
        .unwrap();
    let initial = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    let started = store
        .start_preparation(created.version.id, Uuid::new_v4(), initial.state_revision)
        .await
        .unwrap();
    let run_id = started.workflow.active_run_id.unwrap();

    for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
        let dispatched = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "protocol-test" }),
                    prompt_snapshot: json!({ "contentHash": format!("protocol-{attempt}") }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let assistant_message = if attempt == 1 {
            NewAgentMessage {
                role: AgentMessageRole::Assistant,
                content: json!({
                    "kind": "assistant_tool_calls",
                    "content": null,
                    "toolCalls": [
                        { "id": "bad-call-1", "name": "append_content", "arguments": {} },
                        { "id": "bad-call-2", "name": "replace_content", "arguments": {} },
                    ],
                }),
                request_id: Some(dispatched.id),
                tool_call_id: None,
            }
        } else {
            NewAgentMessage {
                role: AgentMessageRole::Assistant,
                content: json!({
                    "kind": "text",
                    "role": "assistant",
                    "content": "这不是合法的阶段 JSON",
                }),
                request_id: Some(dispatched.id),
                tool_call_id: None,
            }
        };
        let result = store
            .record_model_protocol_correction(
                ProtocolCorrectionTarget::WritingRun,
                RejectedRoundRecord {
                    provider_request_id: Some(format!("provider-{attempt}")),
                    usage: json!({ "totalTokens": "1" }),
                    assistant_message,
                    error: json!({
                        "kind": "protocol_error",
                        "message": "终态提交不是合法 JSON",
                    }),
                    correction: "请重新返回一个合法的阶段结果。".to_owned(),
                    tool_result_code: "PROTOCOL_ERROR".to_owned(),
                },
            )
            .await
            .unwrap();
        assert!(result.newly_applied);
        assert_eq!(result.consecutive_protocol_errors, attempt);
        assert_eq!(result.requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);
        assert_eq!(
            result.workflow.execution,
            if attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS {
                ExecutionStatus::Queued
            } else {
                ExecutionStatus::Stopped
            }
        );
        // 落库的计数必须等于同一轮事件公布的计数，且每一轮都是这样：被拒回合的
        // 计数与收束状态在同一次写入里落库，任何分支都不得只写一半。
        let persisted = persisted_protocol_error_count(&store, run_id).await;
        assert_eq!(persisted, attempt);
        assert_eq!(
            published_protocol_error_count(&store, dispatched.id).await,
            persisted
        );
        let duplicate = store
                .record_model_protocol_correction(
                    ProtocolCorrectionTarget::WritingRun,
                    RejectedRoundRecord {
                        provider_request_id: Some(format!("provider-{attempt}")),
                        usage: json!({ "totalTokens": "1" }),
                        assistant_message: if attempt == 1 {
                            NewAgentMessage {
                                role: AgentMessageRole::Assistant,
                                content: json!({
                                    "kind": "assistant_tool_calls",
                                    "content": null,
                                    "toolCalls": [
                                        { "id": "bad-call-1", "name": "append_content", "arguments": {} },
                                        { "id": "bad-call-2", "name": "replace_content", "arguments": {} },
                                    ],
                                }),
                                request_id: Some(dispatched.id),
                                tool_call_id: None,
                            }
                        } else {
                            NewAgentMessage {
                                role: AgentMessageRole::Assistant,
                                content: json!({
                                    "kind": "text",
                                    "role": "assistant",
                                    "content": "这不是合法的阶段 JSON",
                                }),
                                request_id: Some(dispatched.id),
                                tool_call_id: None,
                            }
                        },
                        error: json!({
                            "kind": "protocol_error",
                            "message": "终态提交不是合法 JSON",
                        }),
                        correction: "请重新返回一个合法的阶段结果。".to_owned(),
                        tool_result_code: "PROTOCOL_ERROR".to_owned(),
                    },
                )
                .await
                .unwrap();
        assert!(!duplicate.newly_applied);
        assert!(!duplicate.requeue);
    }

    let messages = store.list_agent_messages(run_id).await.unwrap();
    assert_eq!(messages[0].role, AgentMessageRole::Assistant);
    assert_eq!(messages[1].role, AgentMessageRole::Tool);
    assert_eq!(messages[1].tool_call_id.as_deref(), Some("bad-call-1"));
    assert_eq!(messages[2].role, AgentMessageRole::Tool);
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("bad-call-2"));
    assert_eq!(messages[3].role, AgentMessageRole::User);
    assert_eq!(messages[3].content["kind"], "protocol_correction");

    let stopped_run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped_run.status, "stopped");
    assert_eq!(
        stopped_run.consecutive_protocol_errors,
        MAX_CONSECUTIVE_PROTOCOL_ERRORS
    );
    let stopped = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    let resumed = store
        .continue_writing_run(run_id, Uuid::new_v4(), stopped.state_revision)
        .await
        .unwrap();
    assert_eq!(resumed.execution, ExecutionStatus::Queued);
    let resumed_run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed_run.consecutive_protocol_errors, 0);
}

/// 被业务守卫拒绝的阶段提交必须留下配对的工具结果与纠正提示，因而下一轮请求仍然
/// 可以组装；同时它不推进连续协议错误计数。
///
/// 这正是实战卡死的那条路径：只写 assistant 调用、不写配对结果的提交会让
/// 此后每一次请求都在构建阶段被拒（`MissingToolResult`），Run 再也发不出请求。
#[tokio::test]
async fn business_rejected_submission_pairs_the_result_the_next_request_needs() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (_version_id, run_id) = in_flight_preparation_run(&store, "提交被拒").await;
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id,
                settings_snapshot: json!({ "model": "rejected-submission" }),
                prompt_snapshot: json!({ "contentHash": "rejected-submission" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let outcome = store
        .record_phase_submission_rejection(
            ProtocolCorrectionTarget::WritingRun,
            RejectedRoundRecord {
                provider_request_id: Some("rejected-submission-provider".to_owned()),
                usage: json!({ "totalTokens": "1" }),
                assistant_message: crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_preparation",
                    r#"{"artifacts":[],"chapter_plan":[]}"#,
                ),
                error: json!({
                    "kind": "submission_commit",
                    "message": "准备提交缺少必需产物",
                }),
                correction: "上一轮对 `submit_preparation` 的提交被本阶段守卫拒绝。".to_owned(),
                tool_result_code: "PHASE_SUBMISSION_REJECTED".to_owned(),
            },
        )
        .await
        .unwrap();
    assert!(outcome.newly_applied);
    assert!(outcome.requeue);
    assert_eq!(outcome.consecutive_protocol_errors, 0);
    assert_eq!(outcome.workflow.execution, ExecutionStatus::Queued);

    // 被拒的提交绝不留下一半的账本：assistant 调用、配对工具结果、纠正提示。
    let messages = store.list_agent_messages(run_id).await.unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, AgentMessageRole::Assistant);
    assert_eq!(messages[1].role, AgentMessageRole::Tool);
    assert_eq!(messages[1].tool_call_id.as_deref(), Some("call-submit-1"));
    assert_eq!(messages[1].content["kind"], "tool_result");
    let paired = messages[1].content["content"].as_str().unwrap();
    assert!(paired.contains("PHASE_SUBMISSION_REJECTED"), "{paired}");
    assert!(paired.contains("submit_preparation"), "{paired}");
    assert_eq!(messages[2].role, AgentMessageRole::User);
    assert_eq!(messages[2].content["kind"], "protocol_correction");

    // 这一轮不再属于「模型协议不合格」：计数不变，事件也不公布计数与上限原因。
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 0);
    assert_eq!(
        store.list_model_requests(run_id).await.unwrap()[0].status,
        ModelRequestStatus::Succeeded
    );
    let published = store
        .events_after(0)
        .await
        .unwrap()
        .into_iter()
        .rfind(|event| event.event_type == "run.changed")
        .unwrap();
    assert_eq!(published.payload["reason"], "submission_rejected");
    assert!(
        published
            .payload
            .get("consecutive_protocol_errors")
            .is_none()
    );
    assert!(published.payload.get("wait_reason").is_none());
}

/// 被业务守卫拒绝的提交不参与连续协议错误计数，因此超过协议上限的轮数之后 Run 仍然
/// 退给模型重提：非进展回合的工程上限由请求额度承担，不由协议错误计数承担。
#[tokio::test]
async fn business_rejected_submissions_never_stop_at_the_protocol_limit() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (version_id, run_id) = in_flight_preparation_run(&store, "提交持续被拒").await;

    for round in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS + 1 {
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "rejected-submission" }),
                    prompt_snapshot: json!({ "contentHash": format!("rejected-{round}") }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let outcome = store
            .record_phase_submission_rejection(
                ProtocolCorrectionTarget::WritingRun,
                RejectedRoundRecord {
                    provider_request_id: Some(format!("rejected-provider-{round}")),
                    usage: json!({ "totalTokens": "1" }),
                    assistant_message: crate::agent_messages::phase_submission_test_message(
                        request.id,
                        "submit_preparation",
                        r#"{"artifacts":[],"chapter_plan":[]}"#,
                    ),
                    error: json!({
                        "kind": "submission_commit",
                        "message": "准备提交缺少必需产物",
                    }),
                    correction: "上一轮对 `submit_preparation` 的提交被本阶段守卫拒绝。".to_owned(),
                    tool_result_code: "PHASE_SUBMISSION_REJECTED".to_owned(),
                },
            )
            .await
            .unwrap();
        assert!(outcome.requeue, "第 {round} 轮必须立即退回模型");
        assert_eq!(outcome.consecutive_protocol_errors, 0);
        assert_eq!(
            outcome.workflow.execution,
            ExecutionStatus::Queued,
            "第 {round} 轮不得触发协议错误上限"
        );
        assert_eq!(outcome.workflow.wait_reason, None);
    }

    let snapshot = store.get_version_snapshot(version_id).await.unwrap();
    assert_eq!(snapshot.workflow.execution, ExecutionStatus::Queued);
    assert_eq!(snapshot.workflow.wait_reason, None);
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 0);
    // 每一轮都留下了完整配对，整个序列仍然可以组装成下一次请求。
    assert_eq!(
        store.list_agent_messages(run_id).await.unwrap().len(),
        (usize::try_from(MAX_CONSECUTIVE_PROTOCOL_ERRORS).unwrap() + 1) * 3
    );
}

/// 委托审核 lane 的被拒提交与写作 lane 同源：子 Run 的生命周期不变、Review 仍被
/// 这次委托占有、配对结果与纠正提示进账本，计数同样不推进。
#[tokio::test]
async fn business_rejected_review_submission_keeps_the_delegation_running() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (version_id, review_id, delegation_run_id) =
        delegated_background_review(&store, "审核提交被拒").await;
    let request = store
        .capture_and_dispatch_model_request(
            NewModelRequest {
                run_id: delegation_run_id,
                settings_snapshot: json!({ "model": "review-rejected-test" }),
                prompt_snapshot: json!({ "key": "review.preparation.background" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    let before = entity::run::Entity::find_by_id(db_id(delegation_run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    let outcome = store
        .record_phase_submission_rejection(
            ProtocolCorrectionTarget::DelegatedReview { review_id },
            RejectedRoundRecord {
                provider_request_id: Some("review-rejected-provider".to_owned()),
                usage: json!({ "totalTokens": "1" }),
                assistant_message: crate::agent_messages::phase_submission_test_message(
                    request.id,
                    "submit_review",
                    r#"{"passed":true,"confidence":0.82}"#,
                ),
                error: json!({
                    "kind": "submission_commit",
                    "message": "审核提交缺少结论",
                }),
                correction: "上一轮对 `submit_review` 的提交被本阶段守卫拒绝。".to_owned(),
                tool_result_code: "INVALID_REVIEW_RESULT".to_owned(),
            },
        )
        .await
        .unwrap();
    assert!(outcome.newly_applied);
    assert!(outcome.requeue);

    let after = entity::run::Entity::find_by_id(db_id(delegation_run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, before.status);
    assert_ne!(after.status, "failed");
    assert_eq!(after.ended_at, None);
    assert_eq!(after.consecutive_protocol_errors, 0);

    let snapshot = store.get_preparation_snapshot(version_id).await.unwrap();
    let delegated = snapshot
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == PreparationArtifactKind::Background)
        .and_then(|artifact| artifact.review.as_ref())
        .unwrap();
    assert_eq!(delegated.status, ReviewStatus::Delegated);

    let messages = store.list_agent_messages(delegation_run_id).await.unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1].role, AgentMessageRole::Tool);
    let paired = messages[1].content["content"].as_str().unwrap();
    assert!(paired.contains("INVALID_REVIEW_RESULT"), "{paired}");
    assert_eq!(messages[2].content["kind"], "protocol_correction");
}
