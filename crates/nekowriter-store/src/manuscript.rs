//! 长篇阅读的读取：有序目录、窗口正文与整稿查找。
//!
//! 阅读区按窗口取正文，所以打开一部几百章的作品只读取真正要显示的那几章；目录
//! 与内容分离查询，章节顺序和显示名全部来自 `nekowriter_core::manuscript`，与
//! 导出使用同一套规则。查找需要读遍全部已提交正文——这是查找语义本身的要求，
//! 而且查找只在用户按下查找时发生，不参与打开作品的开销。

use std::collections::{HashMap, HashSet};

use nekowriter_core::{
    ChapterKind, ChapterNamer, Manuscript, ManuscriptChapter, ManuscriptMatch, ManuscriptWindow,
    VersionId, manuscript_sort_key, search_manuscript as search_snapshot,
};
use sea_orm::{ConnectionTrait, EntityTrait, QueryFilter, TransactionTrait};
use uuid::Uuid;

use crate::{
    CHAPTER_BODY_ROLE, Store, StoreError, db_id,
    entity::{chapter, document, document_revision, novel_version},
    parse_optional_id,
};

/// 目录里的一章：不含正文，因此读取整份目录也不会把作品读进内存。
struct IndexRow {
    chapter_id: Uuid,
    label: String,
    revision_id: Option<Uuid>,
    draft: bool,
}

/// 一章当前已提交的正文槽：修订 ID 与字数。
pub(crate) struct BodySlot {
    pub revision_id: Uuid,
    pub word_count: i64,
}

impl Store {
    /// 读取阅读区要显示的窗口。
    ///
    /// # Errors
    ///
    /// 稿本不存在或已删除时返回 [`StoreError::VersionNotFound`]；正文修订链损坏时
    /// 返回 [`StoreError::CorruptRecord`]。
    pub async fn read_manuscript_window(
        &self,
        version_id: VersionId,
        offset: usize,
        limit: usize,
    ) -> Result<ManuscriptWindow, StoreError> {
        let transaction = self.database.begin().await?;
        let window = read_manuscript_window(&transaction, version_id, offset, limit).await?;
        // 只读事务：读完即回滚，不留下任何写入痕迹。
        transaction.rollback().await?;
        Ok(window)
    }

    /// 读取整份稿本的已提交正文快照，供导出与查找使用。
    ///
    /// # Errors
    ///
    /// 与 [`Self::read_manuscript_window`] 相同。
    pub async fn load_manuscript(&self, version_id: VersionId) -> Result<Manuscript, StoreError> {
        let transaction = self.database.begin().await?;
        let manuscript = load_manuscript(&transaction, version_id).await?;
        transaction.rollback().await?;
        Ok(manuscript)
    }

    /// 在全部已提交正文里查找一处文本。
    ///
    /// # Errors
    ///
    /// 与 [`Self::read_manuscript_window`] 相同；查询文本由调用方先经
    /// `nekowriter_core::parse_search_query` 校验，空查询在这里没有命中。
    pub async fn search_manuscript(
        &self,
        version_id: VersionId,
        query: &str,
    ) -> Result<Vec<ManuscriptMatch>, StoreError> {
        let manuscript = self.load_manuscript(version_id).await?;
        Ok(search_snapshot(&manuscript.chapters, query))
    }
}

/// 在一个读取事务里组装窗口，因此窗口内的每章都来自同一时刻。
pub(crate) async fn read_manuscript_window<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
    offset: usize,
    limit: usize,
) -> Result<ManuscriptWindow, StoreError> {
    let (_, index) = load_manuscript_index(connection, version_id).await?;
    let total_chapters = index.len();
    let start = offset.min(total_chapters);
    let end = start.saturating_add(limit).min(total_chapters);
    let window = &index[start..end];
    let contents =
        load_revision_contents(connection, window.iter().filter_map(|row| row.revision_id)).await?;
    let chapters = window
        .iter()
        .map(|row| ManuscriptChapter {
            chapter_id: row.chapter_id,
            label: row.label.clone(),
            revision_id: row.revision_id,
            draft: row.draft,
            content: row
                .revision_id
                .and_then(|revision_id| contents.get(&revision_id))
                .cloned()
                .unwrap_or_default(),
        })
        .collect();
    Ok(ManuscriptWindow {
        total_chapters,
        offset: start,
        chapters,
    })
}

/// 在一个读取事务里组装整份快照。
pub(crate) async fn load_manuscript<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<Manuscript, StoreError> {
    let (novel_title, index) = load_manuscript_index(connection, version_id).await?;
    let contents =
        load_revision_contents(connection, index.iter().filter_map(|row| row.revision_id)).await?;
    let chapters = index
        .into_iter()
        .map(|row| ManuscriptChapter {
            content: row
                .revision_id
                .and_then(|revision_id| contents.get(&revision_id))
                .cloned()
                .unwrap_or_default(),
            chapter_id: row.chapter_id,
            label: row.label,
            revision_id: row.revision_id,
            draft: row.draft,
        })
        .collect();
    Ok(Manuscript {
        novel_title,
        chapters,
    })
}

/// 按阅读顺序读出整份目录与小说标题。
///
/// 顺序键、显示名与草稿判定都由 `nekowriter_core::manuscript` 决定，因此目录、
/// 阅读窗口与导出不可能给出三种不同的答案。
async fn load_manuscript_index<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<(String, Vec<IndexRow>), StoreError> {
    let version = novel_version::Entity::find_by_id(db_id(version_id))
        .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
        .one(connection)
        .await?
        .ok_or(StoreError::VersionNotFound)?;
    let mut chapters: Vec<(ChapterKind, chapter::Model)> = chapter::Entity::find()
        .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(chapter::Entity::COLUMN.deleted_at.is_null())
        .all(connection)
        .await?
        .into_iter()
        .map(|model| Ok((manuscript_kind_from_db(&model.kind)?, model)))
        .collect::<Result<_, StoreError>>()?;
    chapters.sort_by(|(left_kind, left), (right_kind, right)| {
        manuscript_sort_key(*left_kind, left.order_index)
            .cmp(&manuscript_sort_key(*right_kind, right.order_index))
            .then(left.id.0.cmp(&right.id.0))
    });

    let body_slots = body_slots_by_chapter(connection, version_id).await?;
    let mut namer = ChapterNamer::default();
    let mut rows = Vec::with_capacity(chapters.len());
    for (kind, model) in chapters {
        let approved_revision = parse_optional_id(
            model.approved_revision_id.as_deref(),
            "chapter.approved_revision_id",
        )?;
        let revision_id = body_slots
            .get(&model.id.0.to_string())
            .map(|slot| slot.revision_id);
        // 只有当现在这份已提交正文就是通过整章审核的那一份时它才是定稿；通过后
        // 又被编辑过的章节仍然是可阅读、可导出的内容，但必须标为草稿。
        let draft = approved_revision.is_none() || approved_revision != revision_id;
        rows.push(IndexRow {
            chapter_id: model.id.0,
            label: namer.label(kind, model.order_index, &model.title),
            revision_id,
            draft,
        });
    }
    Ok((version.novel_title, rows))
}

/// 每章当前已提交的正文修订与字数。
///
/// 目录与阅读窗口共用这一份读取，因此两者对「这一章现在有多少字」不会给出
/// 两个答案；字数也让阅读区能估计每章高度而无需先渲染它。
pub(crate) async fn body_slots_by_chapter<C: ConnectionTrait>(
    connection: &C,
    version_id: VersionId,
) -> Result<HashMap<String, BodySlot>, StoreError> {
    let documents = document::Entity::find()
        .filter(document::Entity::COLUMN.version_id.eq(db_id(version_id)))
        .filter(document::Entity::COLUMN.role.eq(CHAPTER_BODY_ROLE))
        .filter(document::Entity::COLUMN.deleted_at.is_null())
        .all(connection)
        .await?;
    let mut slots = HashMap::new();
    for document in documents {
        let Some(chapter_id) = document.chapter_id else {
            continue;
        };
        let revision_id = parse_optional_id(
            document.current_revision_id.as_deref(),
            "document.current_revision_id",
        )?;
        let Some(revision_id) = revision_id else {
            continue;
        };
        let revision = document_revision::Entity::find_by_id(db_id(revision_id))
            .one(connection)
            .await?
            .ok_or_else(|| StoreError::CorruptRecord(format!("正文修订 {revision_id} 不存在")))?;
        slots.insert(
            chapter_id,
            BodySlot {
                revision_id,
                word_count: revision.word_count,
            },
        );
    }
    Ok(slots)
}

/// 按修订 ID 读取正文原文；缺失的修订是损坏记录，不会静默当作空章。
async fn load_revision_contents<C: ConnectionTrait>(
    connection: &C,
    revision_ids: impl Iterator<Item = Uuid>,
) -> Result<HashMap<Uuid, String>, StoreError> {
    let mut wanted = HashSet::new();
    let ids: Vec<_> = revision_ids
        .filter(|revision_id| wanted.insert(*revision_id))
        .map(db_id)
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let revisions = document_revision::Entity::find()
        .filter(document_revision::Entity::COLUMN.id.is_in(ids.clone()))
        .all(connection)
        .await?;
    let mut contents = HashMap::with_capacity(revisions.len());
    for revision in revisions {
        contents.insert(revision.id.0, revision.content);
    }
    if let Some(missing) = ids
        .iter()
        .find(|revision_id| !contents.contains_key(&revision_id.0))
    {
        return Err(StoreError::CorruptRecord(format!(
            "正文修订 {} 不存在",
            missing.0
        )));
    }
    Ok(contents)
}

fn manuscript_kind_from_db(value: &str) -> Result<ChapterKind, StoreError> {
    match value {
        "main" => Ok(ChapterKind::Main),
        "extra" => Ok(ChapterKind::Extra),
        other => Err(StoreError::CorruptRecord(format!("未知章节类型：{other}"))),
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_core::{
        ChapterSummary, NewProject, count_as_i64, parse_manuscript_window, parse_search_query,
    };
    use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};
    use tempfile::TempDir;

    use super::*;
    use crate::CHAPTER_BODY_ROLE;

    async fn open() -> (TempDir, Store) {
        let data_dir = TempDir::new().unwrap();
        let store = Store::open(data_dir.path()).await.unwrap();
        (data_dir, store)
    }

    async fn new_version(store: &Store) -> Uuid {
        store
            .create_project(
                Uuid::new_v4(),
                NewProject::new("雨夜", "雨幕", None::<&str>).unwrap(),
            )
            .await
            .unwrap()
            .version
            .id
    }

    async fn insert_chapter(
        store: &Store,
        version_id: Uuid,
        kind: &str,
        order_index: i64,
        title: &str,
    ) -> Uuid {
        let chapter_id = Uuid::new_v4();
        let now = nekowriter_core::utc_now_millis();
        chapter::ActiveModel {
            id: Set(db_id(chapter_id)),
            version_id: Set(db_id(version_id)),
            kind: Set(kind.to_owned()),
            plan_item_id: Set(None),
            order_index: Set(order_index),
            title: Set(title.to_owned()),
            status: Set("planned".to_owned()),
            approved_revision_id: Set(None),
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

    /// 在章节的正文文档槽里追加一份已提交正文，并可选地标为通过整章审核的那一份。
    async fn commit_body(
        store: &Store,
        version_id: Uuid,
        chapter_id: Uuid,
        content: &str,
        approve: bool,
    ) -> Uuid {
        let now = nekowriter_core::utc_now_millis();
        let document = document::Entity::find()
            .filter(
                document::Entity::COLUMN
                    .chapter_id
                    .eq(chapter_id.to_string()),
            )
            .filter(document::Entity::COLUMN.role.eq(CHAPTER_BODY_ROLE))
            .one(&store.database)
            .await
            .unwrap();
        let document_id = if let Some(document) = document {
            document.id.0
        } else {
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
        };
        let existing_revisions = document_revision::Entity::find()
            .filter(
                document_revision::Entity::COLUMN
                    .document_id
                    .eq(db_id(document_id)),
            )
            .all(&store.database)
            .await
            .unwrap()
            .len();
        let revision_no = count_as_i64(existing_revisions) + 1;
        let revision_id = Uuid::new_v4();
        document_revision::ActiveModel {
            id: Set(db_id(revision_id)),
            document_id: Set(db_id(document_id)),
            revision_no: Set(revision_no),
            content: Set(content.to_owned()),
            structured_payload: Set(None),
            word_count: Set(count_as_i64(content.chars().count())),
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
        active.update(&store.database).await.unwrap();
        if approve {
            let mut chapter = chapter::Entity::find_by_id(db_id(chapter_id))
                .one(&store.database)
                .await
                .unwrap()
                .unwrap()
                .into_active_model();
            chapter.approved_revision_id = Set(Some(revision_id.to_string()));
            chapter.status = Set("approved".to_owned());
            chapter.update(&store.database).await.unwrap();
        }
        revision_id
    }

    #[tokio::test]
    async fn window_reads_only_the_requested_chapters_in_reading_order() {
        let (_data_dir, store) = open().await;
        let version_id = new_version(&store).await;
        for index in 1..=4 {
            let chapter_id =
                insert_chapter(&store, version_id, "main", index, &format!("第{index}节")).await;
            commit_body(
                &store,
                version_id,
                chapter_id,
                &format!("第{index}节的正文"),
                true,
            )
            .await;
        }
        let extra = insert_chapter(&store, version_id, "extra", 1, "尾声").await;
        commit_body(&store, version_id, extra, "番外的正文", false).await;
        let empty = insert_chapter(&store, version_id, "main", 5, "空章").await;

        let window = store
            .read_manuscript_window(version_id, 0, 2)
            .await
            .unwrap();
        assert_eq!(window.total_chapters, 6);
        assert_eq!(window.offset, 0);
        assert_eq!(
            window
                .chapters
                .iter()
                .map(|chapter| chapter.label.as_str())
                .collect::<Vec<_>>(),
            ["第1章 第1节", "第2章 第2节"]
        );
        assert_eq!(window.chapters[0].content, "第1节的正文");
        assert!(!window.chapters[0].draft);

        // 窗口越过后半部分时仍然只读那几章，并保持「正文在前、番外在后」。
        let tail = store
            .read_manuscript_window(version_id, 4, 10)
            .await
            .unwrap();
        assert_eq!(tail.chapters.len(), 2);
        assert_eq!(tail.chapters[0].label, "第5章 空章");
        assert_eq!(tail.chapters[0].content, "");
        assert!(tail.chapters[0].revision_id.is_none());
        assert!(tail.chapters[0].draft);
        assert_eq!(tail.chapters[1].label, "番外 1 尾声");
        assert_eq!(tail.chapters[1].content, "番外的正文");
        assert!(tail.chapters[1].draft);

        // 越界窗口返回空窗口而不是报错，总长度仍然诚实。
        let beyond = store
            .read_manuscript_window(version_id, 99, 20)
            .await
            .unwrap();
        assert_eq!(beyond.offset, 6);
        assert_eq!(
            beyond.chapters,
            [] as [nekowriter_core::ManuscriptChapter; 0]
        );
        assert_eq!(beyond.total_chapters, 6);

        // 目录与阅读窗口对同一批章节给出同一套显示名与顺序。
        let directory = store.list_chapters(version_id).await.unwrap();
        let labels: Vec<&str> = directory
            .iter()
            .map(|chapter: &ChapterSummary| chapter.label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "第1章 第1节",
                "第2章 第2节",
                "第3章 第3节",
                "第4章 第4节",
                "第5章 空章",
                "番外 1 尾声"
            ]
        );
        assert_eq!(directory[4].id, empty);
    }

    #[tokio::test]
    async fn window_rejects_unknown_versions_and_edited_approved_chapters() {
        let (_data_dir, store) = open().await;
        assert!(matches!(
            store
                .read_manuscript_window(Uuid::new_v4(), 0, 20)
                .await
                .unwrap_err(),
            StoreError::VersionNotFound
        ));

        let version_id = new_version(&store).await;
        let chapter_id = insert_chapter(&store, version_id, "main", 1, "车站").await;
        commit_body(&store, version_id, chapter_id, "雨落在站台上。", true).await;
        // 通过之后又被编辑：已提交正文不再是定稿。
        commit_body(&store, version_id, chapter_id, "雨还在下。", false).await;
        let window = store
            .read_manuscript_window(version_id, 0, 20)
            .await
            .unwrap();
        assert_eq!(window.chapters[0].content, "雨还在下。");
        assert!(window.chapters[0].draft);
    }

    #[tokio::test]
    async fn search_reads_every_committed_chapter_and_returns_scalar_offsets() {
        let (_data_dir, store) = open().await;
        let version_id = new_version(&store).await;
        for index in 1..=3 {
            let chapter_id =
                insert_chapter(&store, version_id, "main", index, &format!("第{index}节")).await;
            commit_body(
                &store,
                version_id,
                chapter_id,
                &format!("第{index}节：雨落在站台上。"),
                true,
            )
            .await;
        }

        let hits = store.search_manuscript(version_id, "雨").await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[2].start_char, 4);
        assert_eq!(hits[2].end_char, 5);
        assert_eq!(hits[2].snippet, "第3节：雨落在站台上。");
        assert_eq!(
            hits[0].chapter_id,
            store.list_chapters(version_id).await.unwrap()[0].id
        );

        assert_eq!(
            store
                .search_manuscript(version_id, "没有这句话")
                .await
                .unwrap(),
            [] as [nekowriter_core::ManuscriptMatch; 0]
        );
        // 空查询不会命中任何东西：它没有可锚定的文本。
        assert_eq!(
            store.search_manuscript(version_id, "").await.unwrap(),
            [] as [nekowriter_core::ManuscriptMatch; 0]
        );
        assert!(matches!(
            store
                .search_manuscript(Uuid::new_v4(), "雨")
                .await
                .unwrap_err(),
            StoreError::VersionNotFound
        ));
    }

    #[test]
    fn manuscript_query_parameters_are_shared_with_the_api() {
        // 路由与前端用同一份规则：这里只确认 core 的解析是唯一入口。
        assert!(parse_manuscript_window(Some(0), Some(50)).is_ok());
        assert!(parse_search_query("雨").is_ok());
    }
}
