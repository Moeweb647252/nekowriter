//! Domain types for NekoWriter.
//!
//! This crate performs no I/O. Each type derives every representation it needs (JSON
//! through serde, SQLite columns through SeaORM value types, TypeScript through ts-rs), so
//! a concept is defined exactly once and the database, the HTTP API and the web UI cannot
//! drift apart.
//!
//! JSON conventions: object keys are `camelCase`; enum values and event tags are
//! `snake_case`.

pub mod conflict;
pub mod event;
pub mod field;
pub mod ids;
pub mod project;
pub mod time;
