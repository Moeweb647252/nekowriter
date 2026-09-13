//! 回收站视图。
//!
//! 软删除的对象仍然完整留在原表里（正文、修订链、位置都不动），回收站只负责把它们
//! 列出来、说明恢复前提，并在恢复时把 `deleted_at` 清回去。这里只定义视图类型，
//! 读取与恢复的存储逻辑在 `nekowriter-store::recycle`。

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::decimal_i64;

/// 回收站路由里的对象类别不在支持范围内。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("未知的回收站对象类别：{0}")]
pub struct RecycleQueryError(String);

/// 解析路由里的对象类别；未知值必须报错，不能降级成「什么都找不到」。
///
/// # Errors
///
/// 类别不在 [`RecycleItemKind`] 的稳定字符串集合里时返回 [`RecycleQueryError`]。
pub fn parse_recycle_item_kind(value: &str) -> Result<RecycleItemKind, RecycleQueryError> {
    RecycleItemKind::parse(value).ok_or_else(|| RecycleQueryError(value.to_owned()))
}

/// 回收站里的对象类别。
///
/// 字符串形式是稳定接口：路由参数与持久事件载荷都用它，未知值必须报错而不是忽略。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecycleItemKind {
    /// 整个项目（含它的稿本、章节与修订链，恢复后原样可见）。
    Project,
    /// 整份稿本（含它的章节、文档与修订链，恢复后原样可见）。
    Version,
    /// 番外章。正文计划章不提供独立删除（见 docs/02 §8.4）。
    Chapter,
    /// 用户提示词模板。
    PromptTemplate,
    /// 知识条目。
    KnowledgeEntry,
    /// 空的知识 Space。
    KnowledgeSpace,
}

impl RecycleItemKind {
    /// 路由参数与事件载荷里使用的稳定字符串。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Version => "version",
            Self::Chapter => "chapter",
            Self::PromptTemplate => "prompt_template",
            Self::KnowledgeEntry => "knowledge_entry",
            Self::KnowledgeSpace => "knowledge_space",
        }
    }

    /// 解析路由参数；`None` 表示这是未知类别。调用方通常用
    /// [`parse_recycle_item_kind`] 拿到带错误类型的入口。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "project" => Some(Self::Project),
            "version" => Some(Self::Version),
            "chapter" => Some(Self::Chapter),
            "prompt_template" => Some(Self::PromptTemplate),
            "knowledge_entry" => Some(Self::KnowledgeEntry),
            "knowledge_space" => Some(Self::KnowledgeSpace),
            _ => None,
        }
    }

    /// 界面上的中文类别名。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "项目",
            Self::Version => "稿本",
            Self::Chapter => "章节",
            Self::PromptTemplate => "提示词模板",
            Self::KnowledgeEntry => "知识条目",
            Self::KnowledgeSpace => "知识 Space",
        }
    }
}

/// 被删除对象所属的仍然可见的父对象。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleParent {
    /// 父对象类别的中文名（「项目」「稿本」「知识 Space」）。
    pub kind: String,
    pub id: Uuid,
    pub title: String,
}

/// 回收站里的一条记录。
///
/// `restorable` 与 `purgeable` 在读取时就给出结论与原因，界面因此可以直接说明
/// 「为什么现在恢复不了」，而不是等用户点下去才失败。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleItem {
    pub kind: RecycleItemKind,
    pub id: Uuid,
    /// 列表主行：稿本标题、章节标题、模板标题、条目标题、Space 标题。
    pub title: String,
    /// 列表次行：章节在作品里的位置、条目所属 Space 等。
    pub detail: Option<String>,
    #[serde(with = "decimal_i64")]
    pub deleted_at_ms: i64,
    pub parent: Option<RecycleParent>,
    pub restorable: bool,
    pub restore_blocked_reason: Option<String>,
    pub purgeable: bool,
    pub purge_blocked_reason: Option<String>,
}

/// 回收站快照；按删除时间倒序。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecycleBin {
    pub items: Vec<RecycleItem>,
}
