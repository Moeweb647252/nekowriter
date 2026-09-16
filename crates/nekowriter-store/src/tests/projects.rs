//! 项目与稿本：持久化、进度批读、删除与恢复、旧稿本仍可读。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn projects_persist_and_command_retries_do_not_duplicate_them() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let command_id = Uuid::new_v4();
    let project = NewProject::new("雨夜计划", "雨夜的来客", None::<&str>).unwrap();

    let created = store
        .create_project(command_id, project.clone())
        .await
        .unwrap();
    let replayed = store.create_project(command_id, project).await.unwrap();
    assert_eq!(created, replayed);
    assert_eq!(store.list_projects().await.unwrap().len(), 1);
    let project_events = store
        .events_after(0)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "project.changed")
        .collect::<Vec<_>>();
    assert_eq!(project_events.len(), 1, "重放命令不能重复写入项目事件");
    assert_eq!(project_events[0].payload["action"], "created");
    assert_eq!(
        project_events[0].payload["project_id"],
        created.project.id.to_string()
    );

    drop(store);
    let reopened = Store::open(data_dir.path()).await.unwrap();
    assert_eq!(
        reopened.get_project(created.project.id).await.unwrap(),
        created
    );
}

#[tokio::test]
async fn project_list_progress_is_batched_from_current_chapters_and_revisions() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("进度摘要", "雨夜的来客", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let now = utc_now_millis();
    let main_approved_id = ChapterId::new_v4();
    let main_planned_id = ChapterId::new_v4();
    let extra_id = ChapterId::new_v4();
    for (id, kind, status, order_index) in [
        (main_approved_id, "main", "approved", 1),
        (main_planned_id, "main", "planned", 2),
        (extra_id, "extra", "in_progress", 1),
    ] {
        chapter::ActiveModel {
            id: Set(db_id(id)),
            version_id: Set(db_id(created.version.id)),
            kind: Set(kind.to_owned()),
            plan_item_id: Set(None),
            order_index: Set(order_index),
            title: Set(format!("章节 {order_index}")),
            status: Set(status.to_owned()),
            approved_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(&store.database)
        .await
        .unwrap();
    }

    for (chapter_id, content) in [(main_approved_id, "雨落码头"), (extra_id, "尾声仍在继续")]
    {
        let document_id = DocumentId::new_v4();
        let revision_id = RevisionId::new_v4();
        document::ActiveModel {
            id: Set(db_id(document_id)),
            version_id: Set(db_id(created.version.id)),
            chapter_id: Set(Some(chapter_id.to_string())),
            role: Set(CHAPTER_BODY_ROLE.to_owned()),
            current_revision_id: Set(None),
            revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(&store.database)
        .await
        .unwrap();
        document_revision::ActiveModel {
            id: Set(db_id(revision_id)),
            document_id: Set(db_id(document_id)),
            revision_no: Set(1),
            content: Set(content.to_owned()),
            structured_payload: Set(None),
            word_count: Set(count_as_i64(non_whitespace_char_count(content))),
            created_by: Set("test".to_owned()),
            source_revision_id: Set(None),
            created_at: Set(now),
        }
        .insert(&store.database)
        .await
        .unwrap();
        document::ActiveModel {
            id: Set(db_id(document_id)),
            current_revision_id: Set(Some(revision_id.to_string())),
            revision: Set(1),
            updated_at: Set(now),
            ..Default::default()
        }
        .update(&store.database)
        .await
        .unwrap();
    }

    let item = store
        .list_projects()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        item.progress,
        ProjectProgress {
            main_chapter_total: 2,
            main_chapter_approved: 1,
            extra_chapter_total: 1,
            current_word_count: 10,
        }
    );
}

#[tokio::test]
async fn deleting_a_project_hides_its_versions_and_restoring_it_brings_them_back() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("保留", "留下的作品", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let doomed = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("待删", "要删的作品", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(store.list_projects().await.unwrap().len(), 2);

    // 项目下先单独删掉一个稿本，用来验证恢复项目不会把它一并复活。
    let second = store
        .create_version(
            doomed.project.id,
            Uuid::new_v4(),
            NewNovelVersion::new("第二稿", "要删的作品").unwrap(),
        )
        .await
        .unwrap();
    let removed = doomed.version.id;
    store
        .delete_version(removed, Uuid::new_v4(), doomed.version.revision)
        .await
        .unwrap();

    let command_id = Uuid::new_v4();
    let current = store.get_project(doomed.project.id).await.unwrap();
    let bin = store
        .delete_project(doomed.project.id, command_id, current.project.revision)
        .await
        .unwrap();
    // 同一条命令重放返回原结果，不会重复删除。
    assert_eq!(
        bin,
        store
            .delete_project(doomed.project.id, command_id, current.project.revision)
            .await
            .unwrap()
    );
    assert_eq!(store.list_projects().await.unwrap().len(), 1);
    assert!(matches!(
        store.get_project(doomed.project.id).await.unwrap_err(),
        StoreError::ProjectNotFound
    ));
    assert!(matches!(
        store
            .get_version_snapshot(second.version.id)
            .await
            .unwrap_err(),
        StoreError::VersionNotFound
    ));

    let project_item = bin
        .items
        .iter()
        .find(|item| item.id == doomed.project.id)
        .expect("项目应该出现在回收站里");
    assert_eq!(project_item.kind, RecycleItemKind::Project);
    assert!(project_item.restorable);
    assert!(!project_item.purgeable);
    assert_eq!(project_item.parent, None);
    assert_eq!(project_item.detail.as_deref(), Some("2 份稿本 · 0 章"));

    // 祖先不可见时，被删的稿本不能单独恢复。
    let version_item = bin
        .items
        .iter()
        .find(|item| item.id == removed)
        .expect("被删的稿本应该还在回收站里");
    assert!(!version_item.restorable);
    assert!(version_item.restore_blocked_reason.is_some());
    assert!(matches!(
        store
            .restore_recycle_item(RecycleItemKind::Version, removed, Uuid::new_v4())
            .await
            .unwrap_err(),
        StoreError::RecycleAncestorDeleted(_)
    ));

    // 项目与正文一样构成审计链，不提供物理删除。
    assert!(matches!(
        store
            .purge_recycle_item(RecycleItemKind::Project, doomed.project.id, Uuid::new_v4())
            .await
            .unwrap_err(),
        StoreError::RecycleNotPurgeable(_)
    ));

    // 恢复项目：稿本原样回来，但此前单独删掉的稿本仍留在回收站里。
    let restored = store
        .restore_recycle_item(RecycleItemKind::Project, doomed.project.id, Uuid::new_v4())
        .await
        .unwrap();
    assert_eq!(store.list_projects().await.unwrap().len(), 2);
    assert!(restored.items.iter().any(|item| item.id == removed));
    assert_eq!(
        store
            .get_version_snapshot(second.version.id)
            .await
            .unwrap()
            .version,
        second.version
    );

    // 恢复不存在的项目：明确报错，而不是假装成功。
    assert!(matches!(
        store
            .restore_recycle_item(RecycleItemKind::Project, doomed.project.id, Uuid::new_v4())
            .await
            .unwrap_err(),
        StoreError::RecycleItemNotFound
    ));
}

#[tokio::test]
async fn old_versions_remain_readable_after_the_selected_version_changes() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let first = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("多稿本快照", "旧稿仍可查看", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let second = store
        .create_version(
            first.project.id,
            Uuid::new_v4(),
            NewNovelVersion::new("第二稿", "旧稿仍可查看").unwrap(),
        )
        .await
        .unwrap();

    let old_snapshot = store.get_version_snapshot(first.version.id).await.unwrap();
    assert_eq!(old_snapshot.version.id, first.version.id);
    assert_eq!(old_snapshot.project.selected_version_id, second.version.id);
    assert_eq!(old_snapshot.workflow.version_id, first.version.id);

    let select_command_id = Uuid::new_v4();
    let selected = store
        .select_version(
            first.project.id,
            first.version.id,
            select_command_id,
            second.project.revision,
        )
        .await
        .unwrap();
    let replayed = store
        .select_version(
            first.project.id,
            first.version.id,
            select_command_id,
            second.project.revision,
        )
        .await
        .unwrap();
    assert_eq!(replayed, selected);
    assert_eq!(selected.version.id, first.version.id);
    assert_eq!(selected.project.selected_version_id, first.version.id);
}
