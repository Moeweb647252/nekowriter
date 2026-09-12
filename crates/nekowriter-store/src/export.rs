//! 稿本正文导出的读取。
//!
//! 导出内容只来自 `documents` / `document_revisions` 的已提交修订链，没有任何
//! 另存副本：导出就是把 `manuscript` 模块读出的那份快照渲染出来。读取在同一个
//! 事务里完成，因此生成期间即使有其他写入提交，导出的每个章节也来自同一个时刻，
//! 不会把不同时刻的章节版本拼在一起。

use nekowriter_core::{VersionExport, VersionId, utc_now_millis};
use sea_orm::TransactionTrait;

use super::{Store, StoreError, manuscript::load_manuscript};

impl Store {
    /// 读取一份可直接渲染的导出快照。
    ///
    /// # Errors
    ///
    /// 稿本不存在或已删除时返回 [`StoreError::VersionNotFound`]；数据库读取失败或
    /// 记录损坏时返回相应错误。
    pub async fn build_version_export(
        &self,
        version_id: VersionId,
    ) -> Result<VersionExport, StoreError> {
        let transaction = self.database.begin().await?;
        let manuscript = load_manuscript(&transaction, version_id).await?;
        // 只读事务：读完即回滚，不留下任何写入痕迹。
        transaction.rollback().await?;
        Ok(manuscript.export(utc_now_millis()))
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{ExportFormat, NewProject};
    use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::{
        CHAPTER_BODY_ROLE, db_id,
        entity::{chapter, document, document_revision},
    };

    async fn open() -> (TempDir, Store) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        (data_dir, store)
    }

    async fn insert_chapter(
        store: &Store,
        version_id: Uuid,
        kind: &str,
        order_index: i64,
        title: &str,
        status: &str,
        approved_revision_id: Option<Uuid>,
    ) -> Uuid {
        let chapter_id = Uuid::new_v4();
        let now = utc_now_millis();
        chapter::ActiveModel {
            id: Set(db_id(chapter_id)),
            version_id: Set(db_id(version_id)),
            kind: Set(kind.to_owned()),
            plan_item_id: Set(None),
            order_index: Set(order_index),
            title: Set(title.to_owned()),
            status: Set(status.to_owned()),
            approved_revision_id: Set(approved_revision_id.map(|id| id.to_string())),
            revision: Set(1),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        }
        .insert(&store.database)
        .await
        .unwrap();
        chapter_id
    }

    async fn insert_body_document(store: &Store, version_id: Uuid, chapter_id: Uuid) -> Uuid {
        let now = utc_now_millis();
        let document_id = Uuid::new_v4();
        document::ActiveModel {
            id: Set(db_id(document_id)),
            version_id: Set(db_id(version_id)),
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
        document_id
    }

    /// 追加一份已提交正文修订，并使它成为该文档的当前修订。
    async fn commit_body(
        store: &Store,
        document_id: Uuid,
        revision_no: i64,
        content: &str,
    ) -> Uuid {
        let now = utc_now_millis();
        let revision_id = Uuid::new_v4();
        document_revision::ActiveModel {
            id: Set(db_id(revision_id)),
            document_id: Set(db_id(document_id)),
            revision_no: Set(revision_no),
            content: Set(content.to_owned()),
            structured_payload: Set(None),
            word_count: Set(0),
            created_by: Set("agent".to_owned()),
            source_revision_id: Set(None),
            created_at: Set(now),
        }
        .insert(&store.database)
        .await
        .unwrap();
        let mut active = document::Entity::find_by_id(db_id(document_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        active.current_revision_id = Set(Some(revision_id.to_string()));
        active.revision = Set(revision_no);
        active.update(&store.database).await.unwrap();
        revision_id
    }

    #[tokio::test]
    async fn export_reads_committed_chains_in_plan_then_extra_order() {
        let (_data_dir, store) = open().await;
        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("雨夜", "雨幕", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let version_id = project.version.id;

        let first = insert_chapter(&store, version_id, "main", 1, "车站", "approved", None).await;
        let first_document = insert_body_document(&store, version_id, first).await;
        let first_revision = commit_body(&store, first_document, 1, "雨落在站台上。").await;
        mark_approved(&store, first, first_revision).await;

        // 第二章通过后又被编辑过：已提交正文不再等于通过的那一份。
        let second = insert_chapter(&store, version_id, "main", 2, "来信", "approved", None).await;
        let second_document = insert_body_document(&store, version_id, second).await;
        let second_approved = commit_body(&store, second_document, 1, "信纸发黄。").await;
        mark_approved(&store, second, second_approved).await;
        commit_body(&store, second_document, 2, "信纸已经发黄。").await;

        // 第三章还没有任何正文，也没有正文文档槽。
        let third = insert_chapter(&store, version_id, "main", 3, "空章", "planned", None).await;

        let extra =
            insert_chapter(&store, version_id, "extra", 1, "尾声", "in_progress", None).await;
        let extra_document = insert_body_document(&store, version_id, extra).await;
        let extra_revision = commit_body(&store, extra_document, 1, "雨停了。").await;
        mark_approved(&store, extra, extra_revision).await;

        let export = store.build_version_export(version_id).await.unwrap();
        assert_eq!(export.novel_title, "雨幕");
        let labels: Vec<&str> = export
            .chapters
            .iter()
            .map(|chapter| chapter.label.as_str())
            .collect();
        // 正文按计划顺序在前，番外按自身顺序在后。
        assert_eq!(
            labels,
            ["第1章 车站", "第2章 来信", "第3章 空章", "番外 1 尾声"]
        );
        assert_eq!(export.chapters[0].content, "雨落在站台上。");
        assert!(!export.chapters[0].draft);
        assert_eq!(export.chapters[1].content, "信纸已经发黄。");
        assert!(export.chapters[1].draft);
        assert_eq!(export.chapters[2].content, "");
        assert!(export.chapters[2].draft);
        assert_eq!(export.chapters[3].content, "雨停了。");
        assert!(!export.chapters[3].draft);
        assert_eq!(export.approved_chapters(), 2);

        let markdown = export.render(ExportFormat::Markdown);
        assert!(markdown.contains("## 第2章 来信（草稿）"));
        assert!(markdown.contains("## 第3章 空章（草稿）\n\n（本章尚无正文）"));
        assert_eq!(export.file_name(ExportFormat::Text), "雨幕.txt");

        // 章节被软删除后不再进入导出。
        let mut active = chapter::Entity::find_by_id(db_id(third))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        active.deleted_at = Set(Some(utc_now_millis()));
        active.update(&store.database).await.unwrap();
        let after_delete = store.build_version_export(version_id).await.unwrap();
        assert_eq!(after_delete.chapters.len(), 3);
    }

    async fn mark_approved(store: &Store, chapter_id: Uuid, revision_id: Uuid) {
        let mut active = chapter::Entity::find_by_id(db_id(chapter_id))
            .one(&store.database)
            .await
            .unwrap()
            .unwrap()
            .into_active_model();
        active.approved_revision_id = Set(Some(revision_id.to_string()));
        active.status = Set("approved".to_owned());
        active.update(&store.database).await.unwrap();
    }

    #[tokio::test]
    async fn export_rejects_unknown_versions_and_covers_empty_ones() {
        let (_data_dir, store) = open().await;
        assert!(matches!(
            store
                .build_version_export(Uuid::new_v4())
                .await
                .unwrap_err(),
            StoreError::VersionNotFound
        ));

        let project = store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("空稿本", "尚未开篇", None::<&str>).unwrap(),
            )
            .await
            .unwrap();
        let empty = store
            .build_version_export(project.version.id)
            .await
            .unwrap();
        assert_eq!(empty.novel_title, "尚未开篇");
        assert!(empty.chapters.is_empty());
        assert_eq!(empty.approved_chapters(), 0);
        let rendered = empty.render(ExportFormat::Text);
        assert!(rendered.contains("尚未开篇"));
        assert!(rendered.contains("本稿本还没有章节。"));
        assert_eq!(empty.file_name(ExportFormat::Markdown), "尚未开篇.md");
    }
}
