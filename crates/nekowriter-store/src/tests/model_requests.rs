//! 模型请求记账：检查点不可变、只在派发时计费、捕获与派发同一次提交。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn model_request_checkpoints_are_immutable_and_charge_only_on_dispatch() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("请求检查点", "模型调用不能重复计费", None::<&str>).unwrap(),
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
    let input = NewModelRequest {
        run_id,
        settings_snapshot: json!({ "model": "test-model" }),
        prompt_snapshot: json!({ "contentHash": "prompt-hash" }),
        context_snapshot: json!({ "messages": [] }),
    };

    let prepared = store.prepare_model_request(input.clone()).await.unwrap();
    assert_eq!(prepared.attempt_no, 1);
    assert_eq!(prepared.status, ModelRequestStatus::Prepared);
    assert!(matches!(
        store.prepare_model_request(input.clone()).await,
        Err(StoreError::ResourceBusy(_))
    ));

    let dispatched = store.dispatch_model_request(prepared.id).await.unwrap();
    assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);
    assert_eq!(
        store.dispatch_model_request(prepared.id).await.unwrap(),
        dispatched
    );
    let in_flight = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    assert_eq!(
        in_flight.execution,
        nekowriter_core::ExecutionStatus::RequestInFlight
    );
    let run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.request_count, 1);
    assert_eq!(run.status, "running");

    let resolution = ModelRequestResolution::Succeeded {
        provider_request_id: Some("provider-request-1".to_owned()),
        usage: json!({ "inputTokens": "12", "outputTokens": "4", "totalTokens": "16" }),
    };
    let succeeded = store
        .resolve_model_request_with_assistant_message(
            prepared.id,
            resolution.clone(),
            NewAgentMessage {
                role: AgentMessageRole::Assistant,
                content: json!({
                    "kind": "text",
                    "role": "assistant",
                    "content": "{\"artifacts\":[...]}"
                }),
                request_id: Some(prepared.id),
                tool_call_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(succeeded.status, ModelRequestStatus::Succeeded);
    assert_eq!(
        store
            .resolve_model_request(prepared.id, resolution)
            .await
            .unwrap(),
        succeeded
    );
    assert_eq!(store.list_agent_messages(run_id).await.unwrap().len(), 1);
    let at_boundary = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    assert_eq!(
        at_boundary.execution,
        nekowriter_core::ExecutionStatus::ToolBoundary
    );

    let second = store.prepare_model_request(input).await.unwrap();
    assert_eq!(second.attempt_no, 2);
    let cancelled = store
        .cancel_prepared_model_request(second.id)
        .await
        .unwrap();
    assert_eq!(cancelled.status, ModelRequestStatus::Cancelled);
    let run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.request_count, 1);
}

#[tokio::test]
async fn captured_model_request_commits_its_context_and_dispatch_checkpoint_together() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("原子请求", "请求上下文必须与发送检查点一致", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个在雨夜相遇又告别的短篇小说").unwrap(),
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
                context_snapshot: json!({ "messages": ["system", "user"] }),
            },
            vec![
                NewAgentMessage {
                    role: AgentMessageRole::System,
                    content: json!({
                        "kind": "text",
                        "role": "system",
                        "content": "你是严谨的小说写作助手。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                },
                NewAgentMessage {
                    role: AgentMessageRole::User,
                    content: json!({
                        "kind": "text",
                        "role": "user",
                        "content": "开始生成准备产物。"
                    }),
                    request_id: None,
                    tool_call_id: None,
                },
            ],
        )
        .await
        .unwrap();

    assert_eq!(request.status, ModelRequestStatus::Dispatched);
    assert_eq!(request.attempt_no, 1);
    assert_eq!(
        store
            .list_agent_messages(run_id)
            .await
            .unwrap()
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let snapshot = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap();
    assert_eq!(
        snapshot.workflow.execution,
        nekowriter_core::ExecutionStatus::RequestInFlight
    );
    let run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "running");
    assert_eq!(run.request_count, 1);

    let events = store.events_after(0).await.unwrap();
    assert!(events.iter().any(|event| event.event_type == "run.changed"));
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "workflow.changed")
    );
}

#[tokio::test]
async fn queued_model_setup_failure_becomes_a_persisted_failed_run_without_a_fake_request() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("预发失败", "请求封存前失败也必须结束排队", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("写一个不会永久转圈的短篇小说").unwrap(),
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

    assert!(
        store
            .fail_queued_model_run(
                run_id,
                json!({
                    "kind": "pre_dispatch",
                    "message": "上下文超过模型窗口",
                }),
            )
            .await
            .unwrap()
    );
    assert!(
        !store
            .fail_queued_model_run(
                run_id,
                json!({
                    "kind": "pre_dispatch",
                    "message": "重复回调不应改写失败记录",
                }),
            )
            .await
            .unwrap()
    );
    let snapshot = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap();
    assert_eq!(
        snapshot.workflow.execution,
        nekowriter_core::ExecutionStatus::Failed
    );
    let run = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "failed");
    assert!(run.checkpoint_json.contains("scheduler_error"));
    assert_eq!(
        store.list_model_requests(run_id).await.unwrap(),
        [] as [model_requests::PersistedModelRequest; 0]
    );
}
