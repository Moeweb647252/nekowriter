//! Request and response DTOs (spec/01 §4: separate from entities, exported with ts-rs).

pub mod content_ops;

pub use content_ops::{ContentOp, ContentOpInput};
