//! Exporting a version's novel as Markdown or plain text (`docs/product-spec.md` §16.4).
//!
//! The text around the chapters (header, chapter labels, draft markers) is in the novel's
//! writing language and lives in `resources/export/`.

use std::sync::LazyLock;

use nekowriter_core::{
    chapter::ChapterKind, project::WritingLanguage, template::ResourceText, time::Timestamp,
};
use nekowriter_store::{ExportedChapter, ExportedVersion};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use time::OffsetDateTime;
use ts_rs::TS;

/// The file format of an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExportFormat {
    Markdown,
    Text,
}

impl ExportFormat {
    pub(crate) fn content_type(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Text => "text/plain; charset=utf-8",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportText {
    header: ResourceText,
    main_chapter: ResourceText,
    extra_chapter: ResourceText,
    draft: ResourceText,
    no_body: ResourceText,
}

static EN: LazyLock<ExportText> = LazyLock::new(|| {
    toml::from_str(include_str!("../resources/export/en.toml"))
        .expect("resources/export/en.toml matches ExportText (checked by the tests)")
});

static ZH_CN: LazyLock<ExportText> = LazyLock::new(|| {
    toml::from_str(include_str!("../resources/export/zh-CN.toml"))
        .expect("resources/export/zh-CN.toml matches ExportText (checked by the tests)")
});

fn export_text(language: WritingLanguage) -> &'static ExportText {
    match language {
        WritingLanguage::ZhCn => &ZH_CN,
        WritingLanguage::En => &EN,
    }
}

/// Renders the novel: its title, a header with the export time and how many plan
/// chapters are approved, then every chapter's label and body.
pub(crate) fn render(novel: &ExportedVersion, format: ExportFormat, at: Timestamp) -> String {
    let text = export_text(novel.writing_language);
    let planned = novel
        .chapters
        .iter()
        .filter(|chapter| chapter.kind == ChapterKind::Main);
    let header = text.header.render(&[
        ("time", &utc_minutes(at)),
        (
            "approved",
            &planned
                .clone()
                .filter(|chapter| chapter.approved)
                .count()
                .to_string(),
        ),
        ("planned", &planned.count().to_string()),
    ]);
    let mut blocks = match format {
        ExportFormat::Markdown => vec![format!("# {}", novel.novel_title), header],
        ExportFormat::Text => vec![novel.novel_title.clone(), header],
    };
    for chapter in &novel.chapters {
        blocks.push(chapter_block(text, chapter, format));
    }
    let mut rendered = blocks.join("\n\n");
    rendered.push('\n');
    rendered
}

fn chapter_block(text: &ExportText, chapter: &ExportedChapter, format: ExportFormat) -> String {
    let label = match chapter.kind {
        ChapterKind::Main => &text.main_chapter,
        ChapterKind::Extra => &text.extra_chapter,
    }
    .render(&[
        ("number", &chapter.number.to_string()),
        ("title", &chapter.title),
    ]);
    let note = |note: &ResourceText| match format {
        ExportFormat::Markdown => format!("*{}*", note.render(&[])),
        ExportFormat::Text => note.render(&[]),
    };
    let mut parts = vec![match format {
        ExportFormat::Markdown => format!("## {label}"),
        ExportFormat::Text => label,
    }];
    match &chapter.body {
        Some(body) => {
            if !chapter.approved {
                parts.push(note(&text.draft));
            }
            parts.push(body.clone());
        }
        None => parts.push(note(&text.no_body)),
    }
    parts.join("\n\n")
}

/// The time as `YYYY-MM-DD HH:MM` in UTC.
fn utc_minutes(at: Timestamp) -> String {
    let time = OffsetDateTime::from_unix_timestamp_nanos(i128::from(at.as_millis()) * 1_000_000)
        .expect("timestamps of this process are valid dates");
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        time.year(),
        u8::from(time.month()),
        time.day(),
        time.hour(),
        time.minute()
    )
}

/// The longest file name stem, in characters.
const STEM_CHARS: usize = 80;
const FALLBACK_STEM: &str = "nekowriter-export";

/// The `Content-Disposition` value that names the file after the novel: an ASCII name for
/// old clients and the full name as RFC 5987 UTF-8 for the rest.
pub(crate) fn content_disposition(novel_title: &str, format: ExportFormat) -> String {
    let stem = file_stem(novel_title);
    let ascii: String = stem
        .chars()
        .map(|character| if character.is_ascii() { character } else { '_' })
        .collect();
    let ascii = if ascii
        .chars()
        .any(|character| character.is_ascii_alphanumeric())
    {
        ascii
    } else {
        FALLBACK_STEM.to_owned()
    };
    let extension = format.extension();
    let encoded = utf8_percent_encode(&stem, RFC_5987_ESCAPED);
    format!("attachment; filename=\"{ascii}.{extension}\"; filename*=UTF-8''{encoded}.{extension}")
}

/// Characters outside RFC 5987's `attr-char`.
const RFC_5987_ESCAPED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// The novel title as a file name stem: without path separators, control characters and
/// characters that file systems reserve, at most [`STEM_CHARS`] characters.
fn file_stem(title: &str) -> String {
    let kept: String = title
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
                )
        })
        .collect();
    // Windows drops trailing dots and spaces; leading dots hide files elsewhere.
    let edge = |character: char| character == '.' || character.is_whitespace();
    let stem: String = kept.trim_matches(edge).chars().take(STEM_CHARS).collect();
    let stem = stem.trim_end_matches(edge);
    if stem.is_empty() {
        FALLBACK_STEM.to_owned()
    } else {
        stem.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(
        kind: ChapterKind,
        number: i32,
        body: Option<&str>,
        approved: bool,
    ) -> ExportedChapter {
        ExportedChapter {
            kind,
            number,
            title: format!("Title {number}"),
            body: body.map(str::to_owned),
            approved,
        }
    }

    fn novel(language: WritingLanguage) -> ExportedVersion {
        ExportedVersion {
            novel_title: "Rain Letters".to_owned(),
            writing_language: language,
            chapters: vec![
                chapter(ChapterKind::Main, 1, Some("Rain fell."), true),
                chapter(ChapterKind::Main, 2, Some("Snow fell."), false),
                chapter(ChapterKind::Main, 3, None, false),
                chapter(ChapterKind::Extra, 1, Some("Fog."), true),
            ],
        }
    }

    // 2026-10-14 03:05:09 UTC.
    const AT: Timestamp = Timestamp::from_millis(1_791_947_109_000);

    #[test]
    fn markdown_lists_every_chapter_with_its_state() {
        assert_eq!(
            render(&novel(WritingLanguage::En), ExportFormat::Markdown, AT),
            "# Rain Letters\n\n\
             Exported 2026-10-14 03:05 (UTC). 1 of 3 planned chapters passed their whole-chapter review.\n\n\
             ## Chapter 1: Title 1\n\nRain fell.\n\n\
             ## Chapter 2: Title 2\n\n*Draft: not approved yet.*\n\nSnow fell.\n\n\
             ## Chapter 3: Title 3\n\n*Not written yet.*\n\n\
             ## Extra 1: Title 1\n\nFog.\n"
        );
    }

    #[test]
    fn plain_text_uses_the_writing_language() {
        let text = render(&novel(WritingLanguage::ZhCn), ExportFormat::Text, AT);
        assert!(text.starts_with("Rain Letters\n\n"), "{text}");
        assert!(!text.contains('#'), "{text}");
        // The Chinese chapter label: "Chapter 2 Title 2", then the draft marker.
        assert!(
            text.contains("\u{7b2c}2\u{7ae0} Title 2\n\n\u{8349}\u{7a3f}"),
            "{text}"
        );
        assert!(text.contains("1/3"), "{text}");
    }

    #[test]
    fn file_names_are_sanitized_with_an_ascii_fallback() {
        assert_eq!(
            content_disposition("Rain: Letters?", ExportFormat::Markdown),
            "attachment; filename=\"Rain Letters.md\"; filename*=UTF-8''Rain%20Letters.md"
        );
        // A title of two ideographs keeps them in the UTF-8 name only.
        assert_eq!(
            content_disposition("\u{96e8}\u{4e66}", ExportFormat::Text),
            "attachment; filename=\"nekowriter-export.txt\"; \
             filename*=UTF-8''%E9%9B%A8%E4%B9%A6.txt"
        );
        assert_eq!(
            content_disposition(" ../\u{7}. ", ExportFormat::Text),
            "attachment; filename=\"nekowriter-export.txt\"; \
             filename*=UTF-8''nekowriter-export.txt"
        );
        let long = "a".repeat(200);
        assert_eq!(file_stem(&long).len(), STEM_CHARS);
    }
}
