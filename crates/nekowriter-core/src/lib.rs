//! Domain types for NekoWriter.
//!
//! This crate performs no I/O. Each type derives every representation it needs (JSON
//! through serde, SQLite columns through SeaORM value types, TypeScript through ts-rs), so
//! a concept is defined exactly once and the database, the HTTP API and the web UI cannot
//! drift apart.
//!
//! JSON conventions: object keys are `camelCase`; enum values and event tags are
//! `snake_case`.

pub mod body;
pub mod call;
pub mod chapter;
pub mod citation;
pub mod conflict;
pub mod dashboard;
pub mod document;
pub mod event;
pub mod feedback;
pub mod field;
pub mod ids;
pub mod knowledge;
pub mod model;
pub mod preparation;
pub mod project;
pub mod prompt;
pub mod prompt_template;
pub mod question;
pub mod review;
pub mod run;
pub mod run_message;
pub mod stage;
pub mod submission;
pub mod template;
pub mod text;
pub mod time;
pub mod usage;
pub mod workflow;
