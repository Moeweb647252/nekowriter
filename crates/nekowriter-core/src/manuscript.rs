//! 长篇阅读与导出共用的章节顺序、命名与已提交正文快照。
//!
//! 目录、阅读区和导出必须对「这部作品由哪些章组成、按什么顺序读、每章叫什么」
//! 给出同一个答案，所以这些判定只在这里定义一次。这里同样是整稿查找的唯一实现：
//! 命中位置与引用使用同一套单位（Unicode scalar value，左闭右开），因此查找结果
//! 可以直接被阅读区高亮，也可以被引用校验复核。

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ChapterId, ChapterKind, ExportChapter, RevisionId, VersionExport};

/// 一次窗口读取允许的最大章节数。
pub const MAX_MANUSCRIPT_LIMIT: i64 = 50;
/// 未指定窗口长度时使用的默认值；初次打开阅读区只加载这么多章。
pub const DEFAULT_MANUSCRIPT_LIMIT: i64 = 20;
/// 查找文本的最大长度（按 scalar 计）。
pub const MAX_SEARCH_QUERY_CHARS: usize = 80;
/// 一次查找最多返回的命中数。
pub const MAX_SEARCH_RESULTS: usize = 50;
/// 命中摘要前后保留的上下文字符数。
const SNIPPET_CONTEXT_CHARS: usize = 24;

/// 阅读与导出共用的章节顺序键：正文按计划顺序在前，番外按自身顺序在后。
///
/// 目录、阅读窗口与导出都按这个键排序，因此三者不会因为各自写一套 SQL 排序而分叉。
#[must_use]
pub const fn manuscript_sort_key(kind: ChapterKind, order_index: i64) -> (u8, i64) {
    let section = match kind {
        ChapterKind::Main => 0,
        ChapterKind::Extra => 1,
    };
    (section, order_index)
}

/// 按阅读顺序依次为章节命名。
///
/// 正文用计划里的章节号；番外用它在番外中的出现次序编号，因此番外的编号不会
/// 因为正文的章数而跳号。调用方只需按阅读顺序喂入。
#[derive(Clone, Copy, Debug, Default)]
pub struct ChapterNamer {
    extra_seen: i64,
}

impl ChapterNamer {
    /// 一章的显示名，例如 `第2章 来信` 或 `番外 1 尾声`。
    #[must_use]
    pub fn label(&mut self, kind: ChapterKind, order_index: i64, title: &str) -> String {
        match kind {
            ChapterKind::Main => format!("第{order_index}章 {title}"),
            ChapterKind::Extra => {
                self.extra_seen += 1;
                format!("番外 {} {title}", self.extra_seen)
            }
        }
    }
}

/// 一章已提交的正文快照。
///
/// 内容只来自 `documents` / `document_revisions` 的修订链，没有另存副本，也没有
/// 导出缓存；`draft` 表示当前这份已提交正文不是通过整章审核的那一份。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptChapter {
    pub chapter_id: ChapterId,
    /// 目录中的显示名，与导出里的章节标题完全一致。
    pub label: String,
    /// 当前已提交的正文修订；本章尚未写过正文时为空。
    pub revision_id: Option<RevisionId>,
    /// 当前已提交正文不是通过整章审核的那一份。
    pub draft: bool,
    /// 已提交的正文原文；没有正文时为空字符串。
    pub content: String,
}

/// 整份稿本的有序正文快照：目录、阅读与导出的共同来源。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manuscript {
    pub novel_title: String,
    pub chapters: Vec<ManuscriptChapter>,
}

impl Manuscript {
    /// 导出只是这份快照的呈现，不产生第二份数据。
    #[must_use]
    pub fn export(&self, generated_at_ms: i64) -> VersionExport {
        VersionExport {
            novel_title: self.novel_title.clone(),
            generated_at_ms,
            chapters: self
                .chapters
                .iter()
                .map(|chapter| ExportChapter {
                    label: chapter.label.clone(),
                    content: chapter.content.clone(),
                    draft: chapter.draft,
                })
                .collect(),
        }
    }
}

/// 一次阅读窗口的内容。
///
/// 阅读区按窗口取正文，因此打开一部几百章的作品只需要读它真正要显示的那几章；
/// `total_chapters` 让虚拟列表不必先加载全文就知道总长度。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptWindow {
    pub total_chapters: usize,
    /// 本窗口第一个章节在阅读顺序中的下标。
    pub offset: usize,
    pub chapters: Vec<ManuscriptChapter>,
}

/// 整稿查找的一处命中。
///
/// 位置以 Unicode scalar value 计数，采用左闭右开区间，与引用完全相同；`snippet`
/// 只是结果列表的展示副本，定位一律以偏移为准。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManuscriptMatch {
    pub chapter_id: ChapterId,
    pub revision_id: RevisionId,
    #[serde(with = "crate::decimal_i64")]
    pub start_char: i64,
    #[serde(with = "crate::decimal_i64")]
    pub end_char: i64,
    pub snippet: String,
}

/// 阅读与查找查询参数不合法时的稳定错误分类。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ManuscriptQueryError {
    #[error("阅读窗口的起始位置不能为负数：{offset}")]
    InvalidOffset { offset: i64 },
    #[error("阅读窗口长度必须在 1 到 {MAX_MANUSCRIPT_LIMIT} 之间，收到 {limit}")]
    InvalidLimit { limit: i64 },
    #[error("查找内容不能为空")]
    EmptyQuery,
    #[error("查找内容最多 {MAX_SEARCH_QUERY_CHARS} 个字符")]
    QueryTooLong,
}

/// 解析阅读窗口参数：缺省时给初次打开阅读区用的窗口。
///
/// # Errors
///
/// 起始位置为负、长度为 0 或超过 [`MAX_MANUSCRIPT_LIMIT`] 时返回错误。
pub fn parse_manuscript_window(
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<(usize, usize), ManuscriptQueryError> {
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err(ManuscriptQueryError::InvalidOffset { offset });
    }
    let limit = limit.unwrap_or(DEFAULT_MANUSCRIPT_LIMIT);
    if !(1..=MAX_MANUSCRIPT_LIMIT).contains(&limit) {
        return Err(ManuscriptQueryError::InvalidLimit { limit });
    }
    let offset =
        usize::try_from(offset).map_err(|_| ManuscriptQueryError::InvalidOffset { offset })?;
    let limit = usize::try_from(limit).map_err(|_| ManuscriptQueryError::InvalidLimit { limit })?;
    Ok((offset, limit))
}

/// 解析查找文本：去掉首尾空白后按 scalar 计数限制长度。
///
/// # Errors
///
/// 去掉首尾空白后为空，或超过 [`MAX_SEARCH_QUERY_CHARS`] 时返回错误。
pub fn parse_search_query(raw: &str) -> Result<String, ManuscriptQueryError> {
    let query = raw.trim();
    if query.is_empty() {
        return Err(ManuscriptQueryError::EmptyQuery);
    }
    if query.chars().count() > MAX_SEARCH_QUERY_CHARS {
        return Err(ManuscriptQueryError::QueryTooLong);
    }
    Ok(query.to_owned())
}

/// 在一份已加载的稿本快照里查找全部命中。
///
/// 匹配逐字进行，仅对 ASCII 字母不区分大小写——这样的折叠不改变字符数量，因此
/// 返回的偏移永远落在原文的正确位置上。命中不重叠，按阅读顺序返回，最多
/// [`MAX_SEARCH_RESULTS`] 条。尚未写过正文的章节没有可定位的修订，不参与查找。
#[must_use]
pub fn search_manuscript(chapters: &[ManuscriptChapter], query: &str) -> Vec<ManuscriptMatch> {
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for chapter in chapters {
        let Some(revision_id) = chapter.revision_id else {
            continue;
        };
        let haystack: Vec<(usize, char)> = chapter.content.char_indices().collect();
        if haystack.len() < needle.len() {
            continue;
        }
        let mut start = 0;
        while start + needle.len() <= haystack.len() {
            let hit = needle.iter().enumerate().all(|(offset, expected)| {
                haystack[start + offset].1.eq_ignore_ascii_case(expected)
            });
            if !hit {
                start += 1;
                continue;
            }
            let end = start + needle.len();
            matches.push(ManuscriptMatch {
                chapter_id: chapter.chapter_id,
                revision_id,
                // 一篇文章的字符数远不会接近 i64 上限；超出时钳制而不是回绕。
                start_char: i64::try_from(start).unwrap_or(i64::MAX),
                end_char: i64::try_from(end).unwrap_or(i64::MAX),
                snippet: match_snippet(&chapter.content, start, end, &haystack),
            });
            if matches.len() == MAX_SEARCH_RESULTS {
                return matches;
            }
            // 命中不重叠：从本次命中之后继续，避免 `aa` 在 `aaa` 里报两次半截结果。
            start = end;
        }
    }
    matches
}

/// 命中前后的上下文，压成单行便于在结果列表里阅读。
fn match_snippet(content: &str, start: usize, end: usize, haystack: &[(usize, char)]) -> String {
    let context_start = start.saturating_sub(SNIPPET_CONTEXT_CHARS);
    let context_end = (end + SNIPPET_CONTEXT_CHARS).min(haystack.len());
    let byte_start = haystack.get(context_start).map_or(0, |(index, _)| *index);
    let byte_end = haystack
        .get(context_end)
        .map_or(content.len(), |(index, _)| *index);
    let mut snippet = String::new();
    if context_start > 0 {
        snippet.push('…');
    }
    for character in content[byte_start..byte_end].chars() {
        snippet.push(if character.is_control() {
            ' '
        } else {
            character
        });
    }
    if context_end < haystack.len() {
        snippet.push('…');
    }
    snippet
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn chapter(
        id: u128,
        label: &str,
        revision_id: Option<u128>,
        content: &str,
    ) -> ManuscriptChapter {
        ManuscriptChapter {
            chapter_id: Uuid::from_u128(id),
            label: label.to_owned(),
            revision_id: revision_id.map(Uuid::from_u128),
            draft: false,
            content: content.to_owned(),
        }
    }

    #[test]
    fn main_chapters_read_before_extras_and_extras_number_themselves() {
        let order = |kind, index| manuscript_sort_key(kind, index);
        assert!(order(ChapterKind::Main, 12) < order(ChapterKind::Extra, 1));
        assert!(order(ChapterKind::Main, 1) < order(ChapterKind::Main, 2));

        let mut namer = ChapterNamer::default();
        assert_eq!(namer.label(ChapterKind::Main, 1, "车站"), "第1章 车站");
        assert_eq!(namer.label(ChapterKind::Extra, 1, "尾声"), "番外 1 尾声");
        assert_eq!(
            namer.label(ChapterKind::Extra, 2, "又一场雨"),
            "番外 2 又一场雨"
        );
    }

    #[test]
    fn window_parameters_are_strict() {
        assert_eq!(parse_manuscript_window(None, None).unwrap(), (0, 20));
        assert_eq!(
            parse_manuscript_window(Some(40), Some(50)).unwrap(),
            (40, 50)
        );
        assert_eq!(
            parse_manuscript_window(Some(-1), None).unwrap_err(),
            ManuscriptQueryError::InvalidOffset { offset: -1 }
        );
        assert_eq!(
            parse_manuscript_window(None, Some(0)).unwrap_err(),
            ManuscriptQueryError::InvalidLimit { limit: 0 }
        );
        assert_eq!(
            parse_manuscript_window(None, Some(MAX_MANUSCRIPT_LIMIT + 1)).unwrap_err(),
            ManuscriptQueryError::InvalidLimit {
                limit: MAX_MANUSCRIPT_LIMIT + 1
            }
        );
    }

    #[test]
    fn search_queries_are_trimmed_and_bounded() {
        assert_eq!(parse_search_query("  雨  ").unwrap(), "雨");
        assert_eq!(
            parse_search_query("   ").unwrap_err(),
            ManuscriptQueryError::EmptyQuery
        );
        let too_long = "雨".repeat(MAX_SEARCH_QUERY_CHARS + 1);
        assert_eq!(
            parse_search_query(&too_long).unwrap_err(),
            ManuscriptQueryError::QueryTooLong
        );
    }

    #[test]
    fn search_returns_scalar_offsets_over_the_committed_text() {
        let chapters = vec![
            chapter(1, "第1章 车站", Some(11), "雨落在站台上。雨声很密。"),
            chapter(2, "第2章 来信", None, ""),
            chapter(3, "第3章 站台", Some(33), "Rains fall here."),
        ];
        let hits = search_manuscript(&chapters, "雨");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].chapter_id, Uuid::from_u128(1));
        assert_eq!(hits[0].revision_id, Uuid::from_u128(11));
        // 「雨落在站台上。」里第一个「雨」在下标 0，第二个在 7。
        assert_eq!((hits[0].start_char, hits[0].end_char), (0, 1));
        assert_eq!((hits[1].start_char, hits[1].end_char), (7, 8));
        assert_eq!(hits[1].snippet, "雨落在站台上。雨声很密。");

        // ASCII 大小写折叠不改变偏移。
        let latin = search_manuscript(&chapters, "rains");
        assert_eq!(latin.len(), 1);
        assert_eq!((latin[0].start_char, latin[0].end_char), (0, 5));

        // 没有正文的章节不会产生不可定位的命中。
        assert!(search_manuscript(&chapters, "信纸").is_empty());
    }

    #[test]
    fn search_caps_results_and_does_not_overlap() {
        let content = "雨".repeat(2 * MAX_SEARCH_RESULTS + 10);
        let chapters = vec![chapter(1, "第1章 雨", Some(11), &content)];
        let hits = search_manuscript(&chapters, "雨雨");
        assert_eq!(hits.len(), MAX_SEARCH_RESULTS);
        assert_eq!((hits[0].start_char, hits[0].end_char), (0, 2));
        assert_eq!((hits[1].start_char, hits[1].end_char), (2, 4));
    }

    #[test]
    fn export_is_a_view_of_the_same_snapshot() {
        let manuscript = Manuscript {
            novel_title: "雨幕".to_owned(),
            chapters: vec![
                chapter(1, "第1章 车站", Some(11), "雨落在站台上。"),
                ManuscriptChapter {
                    draft: true,
                    ..chapter(2, "第2章 来信", Some(22), "信纸发黄。")
                },
            ],
        };
        let export = manuscript.export(1_700_000_000_000);
        assert_eq!(export.novel_title, "雨幕");
        assert_eq!(export.chapters.len(), 2);
        assert_eq!(export.chapters[1].label, "第2章 来信");
        assert!(export.chapters[1].draft);
        assert_eq!(export.approved_chapters(), 1);
    }
}
