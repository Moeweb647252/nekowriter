//! Agent 工具调用与消息：重放、提问参数、消息的不可变有序记录。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn free_agent_tool_call_requeues_and_replays_its_stored_result() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("知识检索工具", "自由工具结果落库", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一部关于灯塔与雾港的短篇小说").unwrap(),
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
    let tool_result = json!({
        "hits": [{
            "chunkId": "chunk-1",
            "entryId": "entry-1",
            "entryRevision": "1",
            "spaceId": "space-1",
            "chunkIndex": "0",
            "title": "雾港城",
            "text": "灯塔用蓝鲸油点亮。",
            "score": 0.5,
        }],
        "coverage": {
            "readyEntries": "1",
            "pendingEntries": "0",
            "failedEntries": "0",
            "notConfiguredEntries": "0",
        },
    });
    let call = FreeAgentToolCall {
        provider_call_id: "call-1".to_owned(),
        name: nekowriter_core::TOOL_QUERY_KNOWLEDGE.to_owned(),
        arguments: json!({ "query": "灯塔", "topK": 5 }),
        result: tool_result.clone(),
        failed: false,
        error_code: None,
    };

    let committed = store
        .execute_free_agent_tool_call(
            request.id,
            Some("provider-request-1".to_owned()),
            json!({ "totalTokens": "5" }),
            call.clone(),
        )
        .await
        .unwrap();
    assert_eq!(committed.result, tool_result);
    assert!(committed.newly_resolved);
    assert!(committed.requeue);
    assert_eq!(
        store
            .free_agent_tool_result(request.id, "call-1")
            .await
            .unwrap(),
        Some(tool_result.clone())
    );
    // 一轮工具结果落库后：assistant 工具调用、tool 结果、Run 重新排队、工作流回到生成中。
    assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);
    let queued = store
        .get_preparation_snapshot(created.version.id)
        .await
        .unwrap();
    assert_eq!(queued.workflow.execution, ExecutionStatus::Queued);

    // 同一 provider_call_id 的重放只返回已提交结果，不再追加消息或工具调用记录。
    let replayed = store
        .execute_free_agent_tool_call(
            request.id,
            Some("provider-request-1".to_owned()),
            json!({ "totalTokens": "5" }),
            call.clone(),
        )
        .await
        .unwrap();
    assert_eq!(replayed.result, tool_result);
    assert!(!replayed.newly_resolved);
    assert!(!replayed.requeue);
    assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 3);

    // 同一调用 ID 换了参数属于协议错误，不能静默改写已落库的调用。
    let conflict = store
        .execute_free_agent_tool_call(
            request.id,
            Some("provider-request-1".to_owned()),
            json!({ "totalTokens": "5" }),
            FreeAgentToolCall {
                arguments: json!({ "query": "另一个问题", "topK": 5 }),
                ..call
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(conflict, StoreError::InvalidModelRequestState(_)));
}

#[tokio::test]
async fn invalid_agent_question_tool_calls_stop_at_the_protocol_limit() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("问题工具上限", "无效问题不能无限消耗请求", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个有明确结局的短篇小说").unwrap(),
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

    for attempt in 1..=MAX_CONSECUTIVE_PROTOCOL_ERRORS {
        let request = store
                .capture_and_dispatch_model_request(
                    NewModelRequest {
                        run_id,
                        settings_snapshot: json!({ "model": "invalid-question-test" }),
                        prompt_snapshot: json!({ "contentHash": format!("invalid-question-{attempt}") }),
                        context_snapshot: json!({ "messages": [] }),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
        let result = store
            .execute_agent_question_tool_call(
                request.id,
                Some(format!("invalid-question-provider-{attempt}")),
                json!({ "totalTokens": "1" }),
                AgentQuestionToolCall {
                    provider_call_id: format!("invalid-question-call-{attempt}"),
                    // `questions` 为空是完整 JSON，但不符合 ask_user 的领域合同。
                    arguments: json!({ "questions": [] }),
                },
            )
            .await
            .unwrap();
        let AppliedAgentQuestionToolCall::Resolved {
            result,
            newly_resolved,
            requeue,
            ..
        } = result
        else {
            panic!("无效 ask_user 应生成可回送的工具错误");
        };
        assert!(newly_resolved);
        assert_eq!(result["status"], "error");
        assert_eq!(result["code"], "TOOL_ARGUMENTS_INVALID");
        assert_eq!(requeue, attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS);

        let snapshot = store
            .get_preparation_snapshot(created.version.id)
            .await
            .unwrap();
        assert_eq!(
            snapshot.workflow.execution,
            if attempt < MAX_CONSECUTIVE_PROTOCOL_ERRORS {
                ExecutionStatus::Queued
            } else {
                ExecutionStatus::Stopped
            }
        );
    }

    let run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "stopped");
    assert_eq!(
        run.consecutive_protocol_errors,
        MAX_CONSECUTIVE_PROTOCOL_ERRORS
    );
    let last_run_event = store
        .events_after(0)
        .await
        .unwrap()
        .into_iter()
        .rfind(|event| event.event_type == "run.changed")
        .unwrap();
    assert_eq!(last_run_event.payload["reason"], "tool_error");
    assert_eq!(
        last_run_event.payload["wait_reason"],
        "protocol_error_limit"
    );
    assert_eq!(
        last_run_event.payload["consecutive_protocol_errors"],
        MAX_CONSECUTIVE_PROTOCOL_ERRORS.to_string()
    );
}

#[tokio::test]
async fn agent_messages_are_immutable_ordered_run_scoped_records() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("消息账本", "Agent 历史不能只保留在内存中", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一段需要保留上下文的故事").unwrap(),
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
    let request_id = Uuid::new_v4();

    let appended = store
        .append_agent_messages(
            run_id,
            vec![
                NewAgentMessage {
                    role: AgentMessageRole::System,
                    content: json!({
                        "kind": "text",
                        "role": "system",
                        "content": "你是小说写作助手。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                },
                NewAgentMessage {
                    role: AgentMessageRole::User,
                    content: json!({
                        "kind": "text",
                        "role": "user",
                        "content": "请开始准备。"
                    }),
                    request_id: Some(request_id),
                    tool_call_id: None,
                },
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        appended
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(appended[1].request_id, Some(request_id));
    assert_eq!(store.list_agent_messages(run_id).await.unwrap(), appended);

    assert!(matches!(
        store
            .append_agent_messages(
                run_id,
                vec![NewAgentMessage {
                    role: AgentMessageRole::Tool,
                    content: json!({ "kind": "tool_result", "content": "结果" }),
                    request_id: Some(request_id),
                    tool_call_id: None,
                }],
            )
            .await,
        Err(StoreError::InvalidAgentMessage(_))
    ));
    assert_eq!(store.list_agent_messages(run_id).await.unwrap(), appended);
}

/// `ask_user` 参数无效时模型看到的完整形状（码、消息、`tool_call_id`、字段集）。
///
/// `question_tool_error_result` 的形参从 `String` 改成 `&str` 后，对外可见的就是这四个
/// 字段；两个触发条件（违反 `ask_user` 领域合同、连问题模式都解析不了）都经公开入口
/// `execute_agent_question_tool_call` 走一遍。
#[tokio::test]
async fn ask_user_argument_failures_keep_the_model_visible_shape() {
    /// 派发一次请求并执行一次参数无效的 `ask_user`，回送模型的结果。
    async fn invalid_ask_user(
        store: &Store,
        run_id: RunId,
        attempt: usize,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        let request = store
            .capture_and_dispatch_model_request(
                NewModelRequest {
                    run_id,
                    settings_snapshot: json!({ "model": "ask-user-shape-test" }),
                    prompt_snapshot: json!({ "contentHash": format!("ask-user-shape-{attempt}") }),
                    context_snapshot: json!({ "messages": [] }),
                },
                Vec::new(),
            )
            .await
            .unwrap();
        let applied = store
            .execute_agent_question_tool_call(
                request.id,
                Some(format!("ask-user-shape-provider-{attempt}")),
                json!({}),
                AgentQuestionToolCall {
                    provider_call_id: format!("ask-user-shape-call-{attempt}"),
                    arguments,
                },
            )
            .await
            .unwrap();
        let AppliedAgentQuestionToolCall::Resolved {
            provider_call_id,
            result,
            newly_resolved,
            requeue,
        } = applied
        else {
            panic!("无效参数应形成可回送模型的错误结果，而不是等待用户");
        };
        assert_eq!(provider_call_id, format!("ask-user-shape-call-{attempt}"));
        assert!(newly_resolved);
        assert!(requeue);
        result
    }

    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let (version_id, run_id) = in_flight_preparation_run(&store, "问题工具形状").await;

    // 触发一：完整 JSON 但违反领域合同（零个问题）。
    assert_eq!(
        invalid_ask_user(&store, run_id, 1, json!({ "questions": [] })).await,
        json!({
            "status": "error",
            "code": "TOOL_ARGUMENTS_INVALID",
            "message": "问题列表不能为空",
            "tool_call_id": "ask-user-shape-call-1",
        })
    );

    // 触发二：连问题模式的形状都不对（`questions` 不是数组）。消息里的 serde 细节
    // （具体哪个字段类型不对）不是本用例要钉的东西，其余三个字段连键集一起断言，
    // 因此多写一个键也会失败。
    let mut unparsable =
        invalid_ask_user(&store, run_id, 2, json!({ "questions": "收件人是谁" })).await;
    let message = unparsable["message"].as_str().unwrap().to_owned();
    assert!(
        message.starts_with("ask_user 参数不符合问题模式："),
        "解析失败的消息应点明是 ask_user 参数问题：{message}"
    );
    unparsable["message"] = json!("<serde 细节>");
    assert_eq!(
        unparsable,
        json!({
            "status": "error",
            "code": "TOOL_ARGUMENTS_INVALID",
            "message": "<serde 细节>",
            "tool_call_id": "ask-user-shape-call-2",
        })
    );

    // 两次无效参数相当于两次工具协议错误（不到上限 3），Run 仍可继续。
    assert_eq!(persisted_protocol_error_count(&store, run_id).await, 2);
    assert_eq!(
        store
            .get_preparation_snapshot(version_id)
            .await
            .unwrap()
            .workflow
            .execution,
        ExecutionStatus::Queued
    );
}
