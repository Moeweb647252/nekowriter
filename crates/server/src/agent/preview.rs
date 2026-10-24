//! Incremental preview extraction from streaming tool arguments (spec/03 §12).
//!
//! `PreviewExtractor::new(field)` then `feed(&str)` returns the newly decoded characters of the
//! top-level string field `field` as raw JSON fragments arrive. It handles nesting, other keys
//! appearing first, every JSON escape (`\n \t \" \\ \/ \b \f \r \uXXXX` incl. surrogate pairs),
//! and escape sequences split across chunks.
//!
//! The frontend discards a preview when the persisted tool call arrives, so a preview may be
//! incomplete (it stops at the first unescaped `"`). It must never show mangled text: a partial
//! escape is buffered and completed by the next fragment.

/// A streaming preview extractor for one top-level string field.
#[derive(Debug, Clone)]
pub struct PreviewExtractor {
    field: String,
    /// Inside a JSON string (key, extracted value or a skipped value).
    in_string: bool,
    /// The string being read is an object key.
    is_key: bool,
    /// The string being read is the extracted value.
    streaming: bool,
    /// Inside the top-level object, the next string is a key.
    awaiting_key: bool,
    /// The last top-level key equals `field` and its value has not started.
    awaiting_value: bool,
    /// The key being accumulated.
    key: String,
    /// A backslash sequence split across fragments; starts with `\`.
    pending_escape: String,
    /// A high surrogate whose low half has not arrived.
    high_surrogate: Option<u16>,
    /// Container depth inside the top-level object (1 while at its member level).
    depth: i32,
    /// The whole value was read; further input is ignored.
    done: bool,
}

impl PreviewExtractor {
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            in_string: false,
            is_key: false,
            streaming: false,
            awaiting_key: true,
            awaiting_value: false,
            key: String::new(),
            pending_escape: String::new(),
            high_surrogate: None,
            depth: 0,
            done: false,
        }
    }

    /// The field being extracted.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Feeds one raw fragment and returns the newly decoded characters, when there are any.
    pub fn feed(&mut self, chunk: &str) -> Option<String> {
        let mut out = String::new();
        for ch in chunk.chars() {
            if self.done {
                break;
            }
            self.feed_char(ch, &mut out);
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// Feeds one fragment and returns the whole extracted text so far (convenience for callers
    /// that keep no buffer of their own).
    pub fn pump(&mut self, chunk: &str, buffer: &mut String) {
        if let Some(text) = self.feed(chunk) {
            buffer.push_str(&text);
        }
    }

    fn feed_char(&mut self, ch: char, out: &mut String) {
        // A pending escape consumes characters until it is complete.
        if !self.pending_escape.is_empty() {
            self.pending_escape.push(ch);
            if let Some(decoded) = self.complete_escape() {
                if self.streaming {
                    out.push_str(&decoded);
                } else if self.is_key {
                    self.key.push_str(&decoded);
                }
            }
            return;
        }

        if self.in_string {
            match ch {
                '\\' => {
                    self.pending_escape.push('\\');
                }
                '"' => {
                    self.in_string = false;
                    if self.streaming {
                        self.streaming = false;
                        self.done = true;
                    } else if self.is_key {
                        self.is_key = false;
                        self.awaiting_key = false;
                        self.awaiting_value = self.key == self.field;
                    }
                }
                other => {
                    if self.streaming {
                        out.push(other);
                    } else if self.is_key {
                        self.key.push(other);
                    }
                }
            }
            return;
        }

        match ch {
            '{' | '[' => {
                self.depth += 1;
                if self.depth == 1 {
                    self.awaiting_key = true;
                    self.awaiting_value = false;
                }
            }
            '}' | ']' => {
                self.depth -= 1;
                if self.depth <= 0 {
                    self.done = true;
                }
            }
            '"' => {
                self.in_string = true;
                if self.depth == 1 && self.awaiting_key {
                    self.is_key = true;
                    self.key.clear();
                } else if self.depth == 1 && self.awaiting_value {
                    self.streaming = true;
                    self.awaiting_value = false;
                }
            }
            ':' => {
                if self.depth == 1 && self.awaiting_value {
                    // The value starts next; `awaiting_value` stays set until a `"` is seen.
                    self.awaiting_value = true;
                }
            }
            ',' if self.depth == 1 => {
                self.awaiting_key = true;
                self.awaiting_value = false;
            }
            _ => {}
        }
    }

    /// Completes the pending escape when enough characters arrived; returns the decoded text.
    fn complete_escape(&mut self) -> Option<String> {
        let escape = self.pending_escape.clone();
        let rest: String = escape.chars().skip(1).collect();
        let mut chars = rest.chars();
        let first = chars.next()?;
        match first {
            'u' => {
                let hex: String = chars.collect();
                if hex.chars().count() < 4 {
                    return None;
                }
                self.pending_escape.clear();
                let value = u16::from_str_radix(&hex[..4], 16).ok()?;
                self.decode_unicode(value)
            }
            other => {
                self.pending_escape.clear();
                let decoded = match other {
                    '"' => "\"".to_owned(),
                    '\\' => "\\".to_owned(),
                    '/' => "/".to_owned(),
                    'n' => "\n".to_owned(),
                    't' => "\t".to_owned(),
                    'r' => "\r".to_owned(),
                    'b' => "\u{8}".to_owned(),
                    'f' => "\u{c}".to_owned(),
                    other => other.to_string(),
                };
                Some(decoded)
            }
        }
    }

    /// Decodes `\uXXXX`, pairing surrogate halves that arrive in separate fragments.
    fn decode_unicode(&mut self, value: u16) -> Option<String> {
        if (0xD800..0xDC00).contains(&value) {
            // A high surrogate: keep it until the low half arrives.
            self.high_surrogate = Some(value);
            return None;
        }
        if (0xDC00..0xE000).contains(&value) {
            let high = self.high_surrogate.take()?;
            let combined =
                0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(value) - 0xDC00);
            return char::from_u32(combined).map(|ch| ch.to_string());
        }
        char::from_u32(u32::from(value)).map(|ch| ch.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_all(field: &str, payload: &str) -> String {
        let mut extractor = PreviewExtractor::new(field);
        extractor.feed(payload).unwrap_or_default()
    }

    /// Feeds the payload split at **every byte boundary** (and one character at a time) and
    /// checks each run yields the same text as the unsplit run (PLAN M3.3).
    fn extract_at_every_split(field: &str, payload: &str) {
        let expected = extract_all(field, payload);
        for split in 0..=payload.len() {
            if !payload.is_char_boundary(split) {
                continue;
            }
            let mut extractor = PreviewExtractor::new(field);
            let mut text = String::new();
            extractor.pump(&payload[..split], &mut text);
            extractor.pump(&payload[split..], &mut text);
            assert_eq!(
                text, expected,
                "split at {split} of {payload:?} produced {text:?}, expected {expected:?}"
            );
        }
        let mut extractor = PreviewExtractor::new(field);
        let mut text = String::new();
        for ch in payload.chars() {
            extractor.pump(&ch.to_string(), &mut text);
        }
        assert_eq!(text, expected, "per-character feed of {payload:?}");
    }

    #[test]
    fn extracts_a_simple_field() {
        let payload = r#"{"text":"hello"}"#;
        assert_eq!(extract_all("text", payload), "hello");
        extract_at_every_split("text", payload);
    }

    #[test]
    fn ignores_other_keys_that_come_first() {
        let payload = r#"{"paragraph_end":true,"chapter_end":false,"text":"a b"}"#;
        assert_eq!(extract_all("text", payload), "a b");
        extract_at_every_split("text", payload);
    }

    #[test]
    fn handles_nesting_before_the_field() {
        let payload = r#"{"meta":{"a":[1,2,{"b":"}]"}]},"outline":"Chapter one"}"#;
        assert_eq!(extract_all("outline", payload), "Chapter one");
        extract_at_every_split("outline", payload);
    }

    #[test]
    fn nested_keys_do_not_select_the_field() {
        // `text` appears only inside a nested object: the top-level field is absent.
        let payload = r#"{"meta":{"text":"nested"},"outline":"x"}"#;
        assert_eq!(extract_all("text", payload), "");
        extract_at_every_split("text", payload);
    }

    #[test]
    fn decodes_every_escape() {
        let payload = r#"{"text":"a\nb\tc\"d\\e\/f\bg\fh\ri"}"#;
        assert_eq!(
            extract_all("text", payload),
            "a\nb\tc\"d\\e/f\u{8}g\u{c}h\ri"
        );
        extract_at_every_split("text", payload);
    }

    #[test]
    fn decodes_unicode_escapes_including_surrogate_pairs() {
        let payload = r#"{"text":"\u4f60\u597d \ud83d\ude00"}"#;
        assert_eq!(extract_all("text", payload), "你好 😀");
        extract_at_every_split("text", payload);
    }

    #[test]
    fn a_field_that_never_appears_previews_nothing() {
        assert_eq!(extract_all("text", r#"{"outline":"x"}"#), "");
        assert_eq!(extract_all("text", ""), "");
        assert_eq!(extract_all("text", "{"), "");
    }

    #[test]
    fn a_non_string_value_previews_nothing() {
        assert_eq!(extract_all("text", r#"{"text":42}"#), "");
        assert_eq!(extract_all("text", r#"{"text":null}"#), "");
        assert_eq!(extract_all("text", r#"{"text":["a"]}"#), "");
        assert_eq!(extract_all("text", r#"{"text":{"a":"b"}}"#), "");
    }

    #[test]
    fn text_after_the_value_is_ignored() {
        let payload = r#"{"text":"first","other":"second"}"#;
        assert_eq!(extract_all("text", payload), "first");
    }

    #[test]
    fn a_field_with_a_shared_prefix_is_not_confused() {
        let payload = r#"{"text_extra":"no","text":"yes"}"#;
        assert_eq!(extract_all("text", payload), "yes");
        extract_at_every_split("text", payload);
    }

    #[test]
    fn the_preview_survives_realistic_fragments() {
        // The scripted provider sends 16-character argument fragments (spec/04 §7.1).
        let payload = r#"{"text":"这是一个很长的段落，包含引号\"与换行\n以及表情😀。"}"#;
        assert_eq!(
            extract_all("text", payload),
            "这是一个很长的段落，包含引号\"与换行\n以及表情😀。"
        );
        extract_at_every_split("text", payload);
    }

    #[test]
    fn cjk_text_is_extracted_without_loss() {
        let payload = r#"{"text":"剑光落下，天地一静。"}"#;
        assert_eq!(extract_all("text", payload), "剑光落下，天地一静。");
        extract_at_every_split("text", payload);
    }

    #[test]
    fn an_incomplete_payload_still_yields_its_prefix() {
        let payload = r#"{"text":"half writ"#;
        assert_eq!(extract_all("text", payload), "half writ");
        extract_at_every_split("text", payload);
    }
}
