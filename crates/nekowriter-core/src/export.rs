//! 稿本正文导出的纯逻辑：格式、章节顺序、草稿标记与下载文件名。
//!
//! 导出只是一次呈现，不是另一份数据：这里不保存任何内容，只把调用方从已提交
//! 修订链读出的章节按顺序渲染成 Markdown 或纯文本。文件名由小说标题推出并做过
//! 消毒，因此不可能带上路径分隔符或穿越片段。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 导出格式。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// `.md`：标题用 `#`/`##`，正文原样保留。
    Markdown,
    /// `.txt`：标题与正文都是纯文本，不带任何标记符号。
    Text,
}

impl ExportFormat {
    /// 解析 `?format=` 查询参数；`md` 与 `txt` 之外的取值一律拒绝。
    ///
    /// # Errors
    ///
    /// 取值不是 `md` 或 `txt` 时返回 [`ExportQueryError::InvalidFormat`]。
    pub fn parse(value: &str) -> Result<Self, ExportQueryError> {
        match value {
            "md" => Ok(Self::Markdown),
            "txt" => Ok(Self::Text),
            other => Err(ExportQueryError::InvalidFormat {
                value: other.to_owned(),
            }),
        }
    }

    /// 文件扩展名。
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
        }
    }

    /// 响应的媒体类型。
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Text => "text/plain; charset=utf-8",
        }
    }
}

/// 导出查询参数不合法时的稳定错误分类。
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExportQueryError {
    #[error("导出格式只支持 md 或 txt，收到 {value:?}")]
    InvalidFormat { value: String },
}

/// 导出中的一章。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportChapter {
    /// 章节在目录中的显示名，例如「第 2 章 雨夜」或「番外 1 尾声」。
    pub label: String,
    /// 已提交的正文文本；尚未写过正文时为空字符串。
    pub content: String,
    /// 当前已提交正文是否就是通过整章审核的那一份。为假时导出明确标为草稿。
    pub draft: bool,
}

/// 一次导出的完整内容。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionExport {
    pub novel_title: String,
    pub generated_at_ms: i64,
    pub chapters: Vec<ExportChapter>,
}

impl VersionExport {
    /// 通过整章审核的章节数。
    #[must_use]
    pub fn approved_chapters(&self) -> usize {
        self.chapters
            .iter()
            .filter(|chapter| !chapter.draft)
            .count()
    }

    /// 渲染成目标格式。
    ///
    /// Markdown 用 `#` 标小说标题、`##` 标章节；纯文本把同样两行写成裸文本。
    /// 草稿章节在标签后追加「（草稿）」，两种格式都一样，不伪装为定稿。
    #[must_use]
    pub fn render(&self, format: ExportFormat) -> String {
        let mut out = String::new();
        match format {
            ExportFormat::Markdown => {
                out.push_str("# ");
                out.push_str(&self.novel_title);
                out.push_str("\n\n");
            }
            ExportFormat::Text => {
                out.push_str(&self.novel_title);
                out.push_str("\n\n");
            }
        }
        out.push_str(&self.summary_line());
        out.push('\n');
        if self.chapters.is_empty() {
            out.push_str("\n本稿本还没有章节。\n");
            return out;
        }
        for chapter in &self.chapters {
            out.push('\n');
            if format == ExportFormat::Markdown {
                out.push_str("## ");
            }
            out.push_str(&chapter.label);
            if chapter.draft {
                out.push_str("（草稿）");
            }
            out.push_str("\n\n");
            if chapter.content.is_empty() {
                out.push_str("（本章尚无正文）\n");
                continue;
            }
            out.push_str(&chapter.content);
            if !chapter.content.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }

    /// 一句话说明这份导出的口径：导出时间与已通过比例。
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "由 NekoWriter 导出 · {} · 正文 {} / {} 章已通过整章审核",
            format_date(self.generated_at_ms),
            self.approved_chapters(),
            self.chapters.len()
        )
    }

    /// 下载文件名，含扩展名。
    #[must_use]
    pub fn file_name(&self, format: ExportFormat) -> String {
        format!(
            "{}.{}",
            sanitize_file_stem(&self.novel_title),
            format.extension()
        )
    }
}

/// 把小说标题整理成一个安全的文件名主干。
///
/// 去掉路径分隔符、控制字符与 Windows 保留字符，折叠空白与连续点号，再剪掉首尾
/// 的点号与空格。结果既不含路径分隔符，也不会出现 `..` 或开头带点的隐藏名。
#[must_use]
pub fn sanitize_file_stem(title: &str) -> String {
    let mut stem = String::new();
    let mut last_was_space = false;
    let mut last_was_dot = false;
    for character in title.chars() {
        let is_stripped = character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\u{0}'
            );
        if is_stripped {
            continue;
        }
        if character.is_whitespace() {
            if !last_was_space && !stem.is_empty() {
                stem.push(' ');
            }
            last_was_space = true;
            last_was_dot = false;
            continue;
        }
        if character == '.' {
            // 连续点号是穿越片段的写法，也是难看的文件名，一律只保留一个。
            if !last_was_dot && !stem.is_empty() {
                stem.push('.');
            }
            last_was_dot = true;
            last_was_space = false;
            continue;
        }
        stem.push(character);
        last_was_dot = false;
        last_was_space = false;
    }
    let stem = stem.trim_matches([' ', '.']);
    if stem.is_empty() {
        return "nekowriter-export".to_owned();
    }
    stem.chars().take(80).collect()
}

/// 把毫秒时间戳渲染成 `YYYY-MM-DD HH:MM:SS UTC`。
///
/// 导出不是给人看的界面，因此固定用 UTC，不依赖调用方时区；它在每个平台上都
/// 输出同一份文本，用于核对导出内容时可复现。
fn format_date(timestamp_ms: i64) -> String {
    let seconds = timestamp_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

/// 由 Unix 纪元天数推出公历年月日（Howard Hinnant 的 `civil_from_days`）。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn export(chapters: Vec<ExportChapter>) -> VersionExport {
        VersionExport {
            novel_title: "雨幕".to_owned(),
            generated_at_ms: 1_789_185_600_000,
            chapters,
        }
    }

    fn chapter(label: &str, content: &str, draft: bool) -> ExportChapter {
        ExportChapter {
            label: label.to_owned(),
            content: content.to_owned(),
            draft,
        }
    }

    #[test]
    fn markdown_marks_drafts_and_text_stays_plain() {
        let export = export(vec![
            chapter("第 1 章 车站", "雨落在站台上。", false),
            chapter("番外 1 尾声", "", true),
        ]);
        let markdown = export.render(ExportFormat::Markdown);
        assert!(markdown.starts_with("# 雨幕\n\n"));
        assert!(markdown.contains("## 第 1 章 车站\n\n雨落在站台上。\n"));
        assert!(markdown.contains("## 番外 1 尾声（草稿）\n\n（本章尚无正文）\n"));
        assert!(!markdown.contains("第 1 章 车站（草稿）"));
        assert_eq!(export.approved_chapters(), 1);

        let text = export.render(ExportFormat::Text);
        assert!(!text.contains('#'));
        assert!(text.starts_with("雨幕\n\n"));
        assert!(text.contains("第 1 章 车站\n\n雨落在站台上。\n"));
        assert!(text.contains("番外 1 尾声（草稿）\n\n（本章尚无正文）\n"));
    }

    #[test]
    fn an_empty_version_exports_its_title_and_says_so() {
        let export = export(Vec::new());
        let markdown = export.render(ExportFormat::Markdown);
        assert!(markdown.contains("# 雨幕"));
        assert!(markdown.contains("本稿本还没有章节。"));
        assert!(markdown.contains("正文 0 / 0 章已通过整章审核"));
        assert_eq!(export.approved_chapters(), 0);
    }

    #[test]
    fn summary_line_is_utc_and_reproducible() {
        let export = export(vec![chapter("第 1 章", "正文", true)]);
        assert!(
            export
                .summary_line()
                .starts_with("由 NekoWriter 导出 · 2026-09-12 ")
        );
        assert!(
            export
                .summary_line()
                .ends_with("正文 0 / 1 章已通过整章审核")
        );
    }

    #[test]
    fn file_names_are_sanitized_and_never_empty() {
        assert_eq!(sanitize_file_stem("雨幕"), "雨幕");
        assert_eq!(sanitize_file_stem("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_file_stem("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(sanitize_file_stem("  空白   折叠  "), "空白 折叠");
        assert_eq!(sanitize_file_stem(".."), "nekowriter-export");
        assert_eq!(sanitize_file_stem(""), "nekowriter-export");
        assert_eq!(sanitize_file_stem("尾点..."), "尾点");
        assert_eq!(sanitize_file_stem("含\n换行"), "含换行");
        assert_eq!(sanitize_file_stem(&"长".repeat(200)).chars().count(), 80);
        // 任何结果都不含路径分隔符，也不以点开头。
        for title in ["../../etc/passwd", "..", ".hidden", "a/../b"] {
            let stem = sanitize_file_stem(title);
            assert!(!stem.contains(['/', '\\']), "{title} → {stem}");
            assert!(!stem.starts_with('.'), "{title} → {stem}");
            assert!(!stem.contains(".."), "{title} → {stem}");
        }

        let export = export(Vec::new());
        assert_eq!(export.file_name(ExportFormat::Markdown), "雨幕.md");
        assert_eq!(export.file_name(ExportFormat::Text), "雨幕.txt");
    }

    #[test]
    fn format_query_only_accepts_md_and_txt() {
        assert_eq!(ExportFormat::parse("md").unwrap(), ExportFormat::Markdown);
        assert_eq!(ExportFormat::parse("txt").unwrap(), ExportFormat::Text);
        assert_eq!(
            ExportFormat::parse("MD").unwrap_err(),
            ExportQueryError::InvalidFormat {
                value: "MD".to_owned()
            }
        );
        assert_eq!(
            ExportFormat::Markdown.media_type(),
            "text/markdown; charset=utf-8"
        );
        assert_eq!(ExportFormat::Text.extension(), "txt");
    }
}
