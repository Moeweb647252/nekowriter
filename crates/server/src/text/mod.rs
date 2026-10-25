//! Text handling: chapter content operations, paragraph alignment, word counting, CJK-aware
//! joining and language helpers (spec/03 §8, spec/02 §4, spec/11 §8).
//!
//! Everything here is pure: no database, no HTTP, no prompts. Services call these functions
//! inside their write transactions and the web client mirrors the word-count rule.

pub mod align;
pub mod content;
pub mod join;
pub mod language;
pub mod wordcount;

pub use align::{set_full_text, split_document};
pub use content::{
    AppliedContent, TextError, allocate_pid, apply_inputs, apply_ops, content_word_count,
    keep_next_pid, transform_ops,
};
pub use join::append_segment;
pub use join::{join_texts, normalize_segment, split_paragraphs};
pub use language::{display_name, is_unspaced, primary_subtag};
pub use wordcount::count_words;
