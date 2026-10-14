//! SeaORM entities: the tables of the SQLite database.
//!
//! Entities are the single source of the schema; `crate::schema` creates the tables from
//! them. They are persistence types only and never leave this crate: queries convert rows
//! into the view types that the rest of the application uses.

// SeaORM's `DeriveModelEx` generates async trait methods without `.await` for entities
// that have no relations; the generated code is not ours to change. SeaORM also requires
// every entity struct to be called `Model`, so columns such as `model` or
// `default_model` read to Clippy as repeating the struct's name.
#![allow(clippy::unused_async_trait_impl, clippy::struct_field_names)]

pub(crate) mod budget_charge;
pub(crate) mod chapter;
pub(crate) mod command_receipt;
pub(crate) mod connection_test;
pub(crate) mod document;
pub(crate) mod embedding_profile;
pub(crate) mod embedding_request;
pub(crate) mod embedding_settings;
pub(crate) mod event;
pub(crate) mod feedback;
pub(crate) mod knowledge_chunk;
pub(crate) mod knowledge_entry;
pub(crate) mod knowledge_index;
pub(crate) mod knowledge_space;
pub(crate) mod model_request;
pub(crate) mod project;
pub(crate) mod project_prompt;
pub(crate) mod prompt_template;
pub(crate) mod review;
pub(crate) mod revision;
pub(crate) mod revision_dependency;
pub(crate) mod run;
pub(crate) mod run_message;
pub(crate) mod schema_info;
pub(crate) mod system_prompt;
pub(crate) mod system_settings;
pub(crate) mod tool_call;
pub(crate) mod version;
pub(crate) mod workflow;
