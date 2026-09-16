//! 事件与生命周期：水印、事件顺序、关停中断与回收站。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn version_snapshot_watermark_excludes_expired_events() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("事件水位", "快照只看可恢复事件", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let stale = entity::event::ActiveModel {
        version_id: Set(Some(created.version.id.to_string())),
        event_type: Set("workflow.changed".to_owned()),
        entity_id: Set(db_id(Uuid::new_v4())),
        revision: Set("1".to_owned()),
        payload_json: Set("{}".to_owned()),
        occurred_at: Set(utc_now_millis() - event_log::EVENT_RETENTION_MILLIS - 1),
        ..Default::default()
    }
    .insert(&store.database)
    .await
    .unwrap();
    let retained = entity::event::ActiveModel {
        version_id: Set(Some(created.version.id.to_string())),
        event_type: Set("workflow.changed".to_owned()),
        entity_id: Set(db_id(Uuid::new_v4())),
        revision: Set("2".to_owned()),
        payload_json: Set("{}".to_owned()),
        occurred_at: Set(utc_now_millis()),
        ..Default::default()
    }
    .insert(&store.database)
    .await
    .unwrap();

    let snapshot = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap();
    assert_eq!(snapshot.event_watermark, retained.seq.to_string());
    assert_ne!(snapshot.event_watermark, stale.seq.to_string());
    let events = store.events_after(0).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .any(|event| event.seq == retained.seq.to_string())
    );
    assert!(
        events
            .iter()
            .all(|event| event.seq != stale.seq.to_string())
    );
}

#[tokio::test]
async fn persisted_events_are_ordered_and_wake_subscribers() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("事件同步", "持久事件", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let mut notifications = store.subscribe_events();

    store
        .save_novel_brief(
            created.version.id,
            Uuid::new_v4(),
            NewNovelBrief::new("通过 SSE 同步的小说目标。").unwrap(),
        )
        .await
        .unwrap();

    assert!(notifications.try_recv().is_ok());
    let events = store.events_after(0).await.unwrap();
    assert!(!events.is_empty());
    assert!(
        events.windows(2).all(|pair| {
            pair[0].seq.parse::<i64>().unwrap() < pair[1].seq.parse::<i64>().unwrap()
        })
    );
    let document_event = events
        .iter()
        .find(|event| event.event_type == "document.committed")
        .unwrap();
    assert_eq!(document_event.version_id, Some(created.version.id));
    assert_eq!(document_event.payload["role"], NOVEL_BRIEF_ROLE);
    assert!(
        store
            .events_after(document_event.seq.parse().unwrap())
            .await
            .unwrap()
            .is_empty()
    );
    let bounds = store.event_stream_bounds().await.unwrap();
    assert_eq!(bounds.minimum_cursor, 0);
    assert_eq!(bounds.watermark, document_event.seq.parse::<i64>().unwrap());
}

#[tokio::test]
async fn process_shutdown_interrupts_in_flight_runs_and_close_keeps_data() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("退出收尾", "退出服务不丢已提交内容", None::<&str>).unwrap(),
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
                settings_snapshot: json!({ "model": "shutdown-test" }),
                prompt_snapshot: json!({ "contentHash": "shutdown-test" }),
                context_snapshot: json!({ "messages": [] }),
            },
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(dispatched.status, ModelRequestStatus::Dispatched);

    let outcome = store
        .interrupt_active_runs(InterruptionSource::ProcessShutdown)
        .await
        .unwrap();
    assert_eq!(outcome.writing_runs, 1);
    assert_eq!(outcome.review_delegations, 0);

    let workflow = store
        .get_version_snapshot(created.version.id)
        .await
        .unwrap()
        .workflow;
    assert_eq!(workflow.execution, ExecutionStatus::Interrupted);
    let requests = store.list_model_requests(run_id).await.unwrap();
    assert_eq!(requests[0].status, ModelRequestStatus::Unknown);
    let run_model = entity::run::Entity::find_by_id(db_id(run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run_model.status, "interrupted");
    assert!(run_model.ended_at.is_some());
    let request_model = entity::model_request::Entity::find_by_id(db_id(dispatched.id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    let error_json = request_model.error_json.unwrap();
    assert!(error_json.contains("process_shutdown"), "{error_json}");
    assert!(!error_json.contains("startup_recovery"), "{error_json}");
    // 两种中断来源在事件里可区分，界面才能解释这次中断来自哪个动作。
    let events = store.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.payload["reason"] == "process_shutdown")
            .count(),
        2
    );
    assert!(
        events
            .iter()
            .all(|event| event.payload["reason"] != "startup_recovery")
    );

    // 已经没有自动执行的 Run，重复收尾不再计入。
    let repeated = store
        .interrupt_active_runs(InterruptionSource::ProcessShutdown)
        .await
        .unwrap();
    assert_eq!(repeated, InterruptionOutcome::default());

    store.close().await.unwrap();
    // 关闭之后连接池真的不再接受查询。
    assert!(store.list_projects().await.is_err());

    // WAL 已落回主库文件：重新打开能读回全部记录，且不需要再次恢复。
    let reopened = Store::open(data_dir.path()).await.unwrap();
    assert_eq!(reopened.list_projects().await.unwrap().len(), 1);
    assert_eq!(
        reopened.list_model_requests(run_id).await.unwrap()[0].status,
        ModelRequestStatus::Unknown
    );
}

/// 回收站覆盖提示词模板与知识对象：彻底删除之后不可恢复，父对象未恢复时
/// 子对象拒绝恢复，而不是安静地半恢复。
#[tokio::test]
async fn recycle_bin_covers_templates_and_knowledge_objects() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();

    let template = store
        .create_prompt_template(
            Uuid::new_v4(),
            NewPromptTemplate {
                title: "雨景描写".to_owned(),
                body: "写一段雨夜街景。".to_owned(),
                variables: Vec::new(),
            },
        )
        .await
        .unwrap();
    let space = store
        .create_knowledge_space(
            Uuid::new_v4(),
            KnowledgeSpaceInput {
                title: "人物设定".to_owned(),
            },
        )
        .await
        .unwrap();
    let entry = store
        .create_knowledge_entry(
            Uuid::new_v4(),
            NewKnowledgeEntry {
                space_id: space.id,
                title: "林舟".to_owned(),
                keywords: vec!["主角".to_owned()],
                content: "林舟是港城报社的记者。".to_owned(),
                source: None,
            },
        )
        .await
        .unwrap();

    store
        .delete_prompt_template(template.id, Uuid::new_v4(), template.revision)
        .await
        .unwrap();
    store
        .delete_knowledge_entry(entry.id, Uuid::new_v4(), entry.revision)
        .await
        .unwrap();
    // 条目已软删除，所以 Space 现在可以删除：这正是「父对象在回收站里」的场景。
    let space_revision = store.list_knowledge_spaces().await.unwrap()[0].revision;
    store
        .delete_knowledge_space(space.id, Uuid::new_v4(), space_revision)
        .await
        .unwrap();

    let bin = store.get_recycle_bin().await.unwrap();
    assert_eq!(bin.items.len(), 3);
    let bin_entry = bin
        .items
        .iter()
        .find(|item| item.id == entry.id)
        .expect("知识条目应该在回收站里");
    assert_eq!(bin_entry.kind, RecycleItemKind::KnowledgeEntry);
    assert!(!bin_entry.restorable);
    assert!(bin_entry.purgeable);
    assert!(
        bin_entry
            .restore_blocked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Space"))
    );
    let bin_space = bin
        .items
        .iter()
        .find(|item| item.id == space.id)
        .expect("Space 应该在回收站里");
    assert_eq!(bin_space.kind, RecycleItemKind::KnowledgeSpace);
    assert!(bin_space.restorable);
    let bin_template = bin
        .items
        .iter()
        .find(|item| item.id == template.id)
        .expect("模板应该在回收站里");
    assert_eq!(bin_template.kind, RecycleItemKind::PromptTemplate);
    assert!(bin_template.purgeable);

    let ancestor_error = store
        .restore_recycle_item(RecycleItemKind::KnowledgeEntry, entry.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(
        ancestor_error,
        StoreError::RecycleAncestorDeleted(_)
    ));

    // 先恢复 Space，条目才能恢复；恢复后内容与修订都对得上。
    store
        .restore_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
        .await
        .unwrap();
    let restored = store
        .restore_recycle_item(RecycleItemKind::KnowledgeEntry, entry.id, Uuid::new_v4())
        .await
        .unwrap();
    assert!(restored.items.iter().all(|item| item.id != entry.id));
    let restored_entry = store.get_knowledge_entry(entry.id).await.unwrap();
    assert_eq!(restored_entry.content, "林舟是港城报社的记者。");
    assert!(restored_entry.revision > entry.revision);

    // 彻底删除模板：行真的没了，之后再想恢复必须报「不在回收站里」。
    let purged = store
        .purge_recycle_item(RecycleItemKind::PromptTemplate, template.id, Uuid::new_v4())
        .await
        .unwrap();
    assert!(purged.items.iter().all(|item| item.id != template.id));
    let restore_error = store
        .restore_recycle_item(RecycleItemKind::PromptTemplate, template.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(restore_error, StoreError::RecycleItemNotFound));

    // 重复恢复同一条记录同样报错。
    let repeated = store
        .restore_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(repeated, StoreError::RecycleItemNotFound));

    // 已经恢复的对象不在回收站里，再想彻底删除同样报错，而不是假装成功。
    let not_in_bin = store
        .purge_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(matches!(not_in_bin, StoreError::RecycleItemNotFound));
    assert_eq!(store.list_knowledge_spaces().await.unwrap().len(), 1);
    assert_eq!(
        store
            .list_knowledge_entries(Some(space.id), None)
            .await
            .unwrap()
            .len(),
        1
    );

    // 条目先删、Space 后删，然后两者一起彻底删除：回收站真的空下来。
    store
        .delete_knowledge_entry(entry.id, Uuid::new_v4(), restored_entry.revision)
        .await
        .unwrap();
    let space_revision = store.list_knowledge_spaces().await.unwrap()[0].revision;
    store
        .delete_knowledge_space(space.id, Uuid::new_v4(), space_revision)
        .await
        .unwrap();
    store
        .purge_recycle_item(RecycleItemKind::KnowledgeEntry, entry.id, Uuid::new_v4())
        .await
        .unwrap();
    store
        .purge_recycle_item(RecycleItemKind::KnowledgeSpace, space.id, Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(
        store.list_knowledge_spaces().await.unwrap(),
        [] as [nekowriter_core::KnowledgeSpace; 0]
    );
    assert!(store.get_knowledge_entry(entry.id).await.is_err());
    assert_eq!(
        store.get_recycle_bin().await.unwrap().items,
        [] as [nekowriter_core::recycle::RecycleItem; 0]
    );
}
