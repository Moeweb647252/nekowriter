//! 实体解码、注册表，以及文档槽位 / 章节顺序 / 每项目一条活跃正文 Run 三条约束。
//!
//! 从 `lib.rs` 的 `mod tests` 拆出（`docs/08` 的 P2.3），只移动：函数体逐字不变。

use super::*;

#[tokio::test]
async fn seaorm_entities_decode_nullable_text_foreign_keys() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("实体映射", "SQLite 空外键", None::<&str>).unwrap(),
        )
        .await
        .unwrap();

    let version = novel_version::Entity::find_by_id(db_id(created.version.id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(version.source_version_id, None);

    let documents = document::Entity::find()
        .filter(
            document::Entity::COLUMN
                .version_id
                .eq(db_id(created.version.id)),
        )
        .all(&store.database)
        .await
        .unwrap();
    assert_eq!(documents.len(), 5);
    assert!(documents.iter().all(|document| {
        document.chapter_id.is_none() && document.current_revision_id.is_none()
    }));
    let related_version = documents[0]
        .find_related(novel_version::Entity)
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(related_version.id, db_id(created.version.id));

    let workflow = workflow_instance::Entity::find_by_id(db_id(created.version.id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(workflow.active_run_id, None);
}

#[tokio::test]
async fn entity_registry_initializes_every_store_table() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();

    let metadata = entity::application_metadata::Entity::find_by_id("schema_version".to_owned())
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(metadata.value, SCHEMA_VERSION.to_string());

    entity::project::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::novel_version::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::chapter::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::document::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::document_revision::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::revision_dependency::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::workflow_instance::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::run::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::agent_message::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::model_request::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::tool_call::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::system_model_setting::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::review::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::command_receipt::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
    entity::event::Entity::find()
        .one(&store.database)
        .await
        .unwrap();
}

#[tokio::test]
async fn entity_schema_enforces_active_top_level_document_slots() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("实体索引", "顶层文档槽", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let document_slot = document::Entity::find()
        .filter(
            document::Entity::COLUMN
                .version_id
                .eq(db_id(created.version.id)),
        )
        .filter(document::Entity::COLUMN.role.eq(NOVEL_BRIEF_ROLE))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();

    let mut active_duplicate = document_slot.clone().into_active_model();
    active_duplicate.id = Set(db_id(DocumentId::new_v4()));
    assert!(active_duplicate.insert(&store.database).await.is_err());

    let mut archived_duplicate = document_slot.into_active_model();
    archived_duplicate.id = Set(db_id(DocumentId::new_v4()));
    archived_duplicate.deleted_at = Set(Some(utc_now_millis()));
    archived_duplicate.insert(&store.database).await.unwrap();
}

#[tokio::test]
async fn entity_schema_enforces_active_chapter_order() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let created = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("章节索引", "章节顺序", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let now = utc_now_millis();
    let chapter_id = Uuid::new_v4();
    let first = entity::chapter::ActiveModel {
        id: Set(db_id(chapter_id)),
        version_id: Set(db_id(created.version.id)),
        kind: Set("main".to_owned()),
        plan_item_id: Set(Some(Uuid::new_v4().to_string())),
        order_index: Set(1),
        title: Set("第一章".to_owned()),
        status: Set("planned".to_owned()),
        approved_revision_id: Set(None),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    };
    first.insert(&store.database).await.unwrap();

    let duplicate = entity::chapter::ActiveModel {
        id: Set(db_id(Uuid::new_v4())),
        version_id: Set(db_id(created.version.id)),
        kind: Set("main".to_owned()),
        plan_item_id: Set(Some(Uuid::new_v4().to_string())),
        order_index: Set(1),
        title: Set("重复顺序".to_owned()),
        status: Set("planned".to_owned()),
        approved_revision_id: Set(None),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    }
    .insert(&store.database)
    .await;
    assert!(duplicate.is_err());

    let guidance = document::ActiveModel {
        id: Set(db_id(Uuid::new_v4())),
        version_id: Set(db_id(created.version.id)),
        chapter_id: Set(Some(chapter_id.to_string())),
        role: Set("chapter_guidance".to_owned()),
        current_revision_id: Set(None),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    };
    guidance.insert(&store.database).await.unwrap();
    let duplicate_guidance = document::ActiveModel {
        id: Set(db_id(Uuid::new_v4())),
        version_id: Set(db_id(created.version.id)),
        chapter_id: Set(Some(chapter_id.to_string())),
        role: Set("chapter_guidance".to_owned()),
        current_revision_id: Set(None),
        revision: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
        deleted_at: Set(None),
    }
    .insert(&store.database)
    .await;
    assert!(duplicate_guidance.is_err());
}

#[tokio::test]
async fn entity_schema_allows_only_one_active_top_level_writing_run_per_project() {
    let data_dir = TempDir::new().unwrap();
    let store = Store::open(data_dir.path()).await.unwrap();
    let first = store
        .create_project(
            Uuid::new_v4(),
            NewProject::new("Run 互斥", "跨稿本不能并行写作", None::<&str>).unwrap(),
        )
        .await
        .unwrap();
    let second = store
        .create_version(
            first.project.id,
            Uuid::new_v4(),
            NewNovelVersion::new("第二稿", "跨稿本不能并行写作").unwrap(),
        )
        .await
        .unwrap();
    let now = utc_now_millis();
    let first_run_id = Uuid::new_v4();
    entity::run::ActiveModel {
        id: Set(db_id(first_run_id)),
        project_id: Set(db_id(first.project.id)),
        version_id: Set(db_id(first.version.id)),
        kind: Set("preparation".to_owned()),
        parent_run_id: Set(None),
        stage: Set("preparing".to_owned()),
        status: Set("queued".to_owned()),
        resume_count: Set(0),
        request_count: Set(0),
        request_allowance_end: Set(100),
        consecutive_protocol_errors: Set(0),
        checkpoint_json: Set("{}".to_owned()),
        quota_scope_id: Set(None),
        started_at: Set(now),
        ended_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&store.database)
    .await
    .unwrap();

    let second_run = entity::run::ActiveModel {
        id: Set(db_id(Uuid::new_v4())),
        project_id: Set(db_id(first.project.id)),
        version_id: Set(db_id(second.version.id)),
        kind: Set("chapter_outline".to_owned()),
        parent_run_id: Set(None),
        stage: Set("writing.outline".to_owned()),
        status: Set("queued".to_owned()),
        resume_count: Set(0),
        request_count: Set(0),
        request_allowance_end: Set(100),
        consecutive_protocol_errors: Set(0),
        checkpoint_json: Set("{}".to_owned()),
        quota_scope_id: Set(None),
        started_at: Set(now),
        ended_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    assert!(second_run.clone().insert(&store.database).await.is_err());

    let first_run = entity::run::Entity::find_by_id(db_id(first_run_id))
        .one(&store.database)
        .await
        .unwrap()
        .unwrap();
    let mut completed_run = first_run.into_active_model();
    completed_run.status = Set("succeeded".to_owned());
    completed_run.ended_at = Set(Some(now));
    completed_run.update(&store.database).await.unwrap();
    second_run.insert(&store.database).await.unwrap();
}
