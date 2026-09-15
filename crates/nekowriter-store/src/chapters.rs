//! 章节目录的持久化读取。
//!
//! 章节只会在准备阶段最终通过的事务中从粗纲计划物化。章节指导和细纲命令与
//! 准备阶段最终审核共享同一组事务辅助函数，因此当前保留在 `preparation` 模块；
//! 本模块只负责不改变状态的目录查询。目录与正文内容分开查询：这里不读任何正文，
//! 因此打开一部几百章的作品只需要读它真正要显示的那几章。

use nekowriter_core::{
    ChapterKind, ChapterNamer, ChapterStatus, ChapterSummary, VersionId, manuscript_sort_key,
};
use sea_orm::{EntityTrait, QueryFilter};

use crate::{
    Store, StoreError, db_id,
    entity::{chapter, novel_version},
    manuscript::body_slots_by_chapter,
    parse_optional_id,
};

impl Store {
    /// 按阅读顺序读取稿本的有效章节目录。
    ///
    /// 顺序与显示名来自 `nekowriter_core::manuscript`，因此目录、阅读窗口和导出
    /// 永远给出同一套答案。
    ///
    /// # Errors
    ///
    /// 稿本不存在或持久化枚举/UUID 损坏时返回错误。
    pub async fn list_chapters(
        &self,
        version_id: VersionId,
    ) -> Result<Vec<ChapterSummary>, StoreError> {
        let version_exists = novel_version::Entity::find_by_id(db_id(version_id))
            .filter(novel_version::Entity::COLUMN.deleted_at.is_null())
            .one(&self.database)
            .await?
            .is_some();
        if !version_exists {
            return Err(StoreError::VersionNotFound);
        }

        let mut chapters: Vec<(ChapterKind, chapter::Model)> = chapter::Entity::find()
            .filter(chapter::Entity::COLUMN.version_id.eq(db_id(version_id)))
            .filter(chapter::Entity::COLUMN.deleted_at.is_null())
            .all(&self.database)
            .await?
            .into_iter()
            .map(|model| Ok((chapter_kind_from_db(&model.kind)?, model)))
            .collect::<Result<_, StoreError>>()?;
        chapters.sort_by(|(left_kind, left), (right_kind, right)| {
            manuscript_sort_key(*left_kind, left.order_index)
                .cmp(&manuscript_sort_key(*right_kind, right.order_index))
                .then(left.id.0.cmp(&right.id.0))
        });

        // 目录与阅读窗口共用同一份正文槽读取：字数用于阅读区估计每章高度。
        let body_slots = body_slots_by_chapter(&self.database, version_id).await?;
        let mut namer = ChapterNamer::default();
        chapters
            .into_iter()
            .map(|(kind, model)| {
                let label = namer.label(kind, model.order_index, &model.title);
                let body_word_count = body_slots
                    .get(&model.id.0.to_string())
                    .map_or(0, |slot| slot.word_count);
                chapter_summary_from_model(model, label, body_word_count)
            })
            .collect()
    }
}

fn chapter_summary_from_model(
    model: chapter::Model,
    label: String,
    body_word_count: i64,
) -> Result<ChapterSummary, StoreError> {
    Ok(ChapterSummary {
        id: model.id.into(),
        version_id: model.version_id.into(),
        kind: chapter_kind_from_db(&model.kind)?,
        plan_item_id: parse_optional_id(model.plan_item_id.as_deref(), "chapter.plan_item_id")?,
        order_index: model.order_index,
        title: model.title,
        label,
        body_word_count,
        status: chapter_status_from_db(&model.status)?,
        approved_revision_id: parse_optional_id(
            model.approved_revision_id.as_deref(),
            "chapter.approved_revision_id",
        )?,
        revision: model.revision,
    })
}

fn chapter_kind_from_db(value: &str) -> Result<ChapterKind, StoreError> {
    match value {
        "main" => Ok(ChapterKind::Main),
        "extra" => Ok(ChapterKind::Extra),
        _ => Err(StoreError::CorruptRecord(format!("未知章节类型：{value}"))),
    }
}

fn chapter_status_from_db(value: &str) -> Result<ChapterStatus, StoreError> {
    match value {
        "planned" => Ok(ChapterStatus::Planned),
        "in_progress" => Ok(ChapterStatus::InProgress),
        "approved" => Ok(ChapterStatus::Approved),
        _ => Err(StoreError::CorruptRecord(format!("未知章节状态：{value}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_database_values_are_strict() {
        assert_eq!(chapter_kind_from_db("main").unwrap(), ChapterKind::Main);
        assert_eq!(
            chapter_status_from_db("approved").unwrap(),
            ChapterStatus::Approved
        );
        assert!(chapter_kind_from_db("other").is_err());
        assert!(chapter_status_from_db("done").is_err());
    }
}
