//! The model protocol (`docs/product-spec.md` §8): everything the model reads and writes.
//!
//! - [`client`]: the Chat Completions client.
//! - [`stage`]: per-stage contracts: prompts, tools, submission parsing.
//! - [`round`]: classifying an answer as a submission or a protocol error.
//! - [`context`]: assembling a request and checking that it fits.
//! - [`render`]: turning stored outcomes into model text.
//! - [`prompts`] and [`model_text`]: the Chinese model-facing text in `resources/`.
//!
//! This crate stores nothing and never decides what is approved; the store records the
//! outcomes and the application orchestrates.

pub mod client;
pub mod context;
pub mod model_text;
pub mod prompts;
pub mod render;
pub mod round;
pub mod schema;
pub mod stage;
