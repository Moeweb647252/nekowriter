//! SeaORM entities: the tables of the SQLite database.
//!
//! Entities are the single source of the schema; `crate::schema` creates the tables from
//! them. They are persistence types only and never leave this crate: queries convert rows
//! into the view types that the rest of the application uses.

// SeaORM's `DeriveModelEx` generates async trait methods without `.await` for entities
// that have no relations; the generated code is not ours to change.
#![allow(clippy::unused_async_trait_impl)]

pub(crate) mod command_receipt;
pub(crate) mod event;
pub(crate) mod project;
pub(crate) mod schema_info;
pub(crate) mod version;
