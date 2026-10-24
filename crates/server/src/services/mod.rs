//! Service module tree (spec/01 §4.2): business rules and transactions.
//!
//! Every write operation has an outer function that opens [`crate::db::Db::write`] and publishes
//! the events of the committed transaction, and an inner function that takes the transaction and
//! returns the events to emit. Handlers and agent tools call these; entities never leave this
//! layer.

pub mod app_settings;
pub mod books;
pub mod canon;
pub mod change_log;
pub mod chapters;
pub mod characters;
pub mod memory;
pub mod prompts;
pub mod providers;
pub mod reviews;
pub mod sessions;
pub mod setting_docs;
pub mod tracks;
pub mod versions;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, DbErr, Statement};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::json::{from_json, to_json};
use crate::error::{AppError, ErrorCode};

/// The current UTC time in the representation the schema stores.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Serializes a typed JSON column value, mapping a failure to an internal error (it can only be
/// a programming defect).
pub(crate) fn json<T: serde::Serialize>(value: &T) -> Result<String, AppError> {
    to_json(value).map_err(|err| AppError::internal(format!("cannot serialize JSON column: {err}")))
}

/// Deserializes a typed JSON column, mapping a failure to an internal error.
pub(crate) fn parse_json<T: for<'de> serde::Deserialize<'de>>(
    column: &str,
    raw: &str,
) -> Result<T, AppError> {
    from_json(raw).map_err(|err| AppError::internal(format!("corrupt {column} column: {err}")))
}

/// Reads a plain-TEXT enum column (`CHECK (col IN (...))`, spec/02 §3) into its Rust enum.
pub(crate) fn enum_of<T: std::str::FromStr<Err = String>>(
    column: &str,
    raw: &str,
) -> Result<T, AppError> {
    raw.parse::<T>()
        .map_err(|_| AppError::internal(format!("unknown {column} value {raw:?}")))
}

/// Runs a raw SELECT that returns at most one row and reads one `i64` column.
pub(crate) async fn query_i64(
    conn: &impl ConnectionTrait,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<Option<i64>, AppError> {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            sql,
            values,
        ))
        .await?;
    match row {
        Some(row) => Ok(Some(row.try_get::<i64>("", "value").map_err(|err| {
            AppError::internal(format!("cannot read count: {err}"))
        })?)),
        None => Ok(None),
    }
}

/// Builds a conflict error carrying `details` (spec/10 §4).
pub(crate) fn conflict(code: ErrorCode, message: impl Into<String>, details: Value) -> AppError {
    AppError::new(code, message).with_details(details)
}

/// Builds a `validation.failed` error naming the offending field (spec/10 §4 `details.fields`).
///
/// The UI localizes the code and uses the field name to point at the control, so every input
/// validation in the services goes through this helper.
pub(crate) fn field_error(field: &str, message: impl Into<String>) -> AppError {
    AppError::invalid_fields(serde_json::Map::from_iter([(
        field.to_owned(),
        Value::String(message.into()),
    )]))
}

/// The id of a running writer session for a chapter, if any (spec/08 §1.4 locking).
pub(crate) async fn running_writer_session(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Option<i32>, AppError> {
    first_session(conn, chapter_id, "role = 'writer' AND status = 'running'").await
}

/// The id of the first session of `chapter_id` matching `condition_sql`.
async fn first_session(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
    condition_sql: &str,
) -> Result<Option<i32>, AppError> {
    let sql = format!(
        "SELECT id AS value FROM agent_sessions WHERE chapter_id = ? AND {condition_sql} \
         ORDER BY id LIMIT 1"
    );
    query_i64(conn, &sql, vec![chapter_id.into()])
        .await
        .map(|found| found.map(|value| value as i32))
}

/// Any open session of a chapter (status not completed, superseded or abandoned).
pub(crate) async fn open_chapter_session(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Option<i32>, AppError> {
    first_session(
        conn,
        chapter_id,
        "status NOT IN ('completed','superseded','abandoned')",
    )
    .await
}

/// Any open writer session of a chapter.
pub(crate) async fn open_writer_session(
    conn: &impl ConnectionTrait,
    chapter_id: i32,
) -> Result<Option<i32>, AppError> {
    first_session(
        conn,
        chapter_id,
        "role = 'writer' AND status NOT IN ('completed','superseded','abandoned')",
    )
    .await
}

/// True when the book's conception session is running (setting materials are locked then,
/// spec/08 §1.4).
pub(crate) async fn conception_session_running(
    conn: &impl ConnectionTrait,
    book_id: i32,
) -> Result<Option<i32>, AppError> {
    let sql = "SELECT id AS value FROM agent_sessions \
               WHERE book_id = ? AND kind = 'conception' AND status = 'running' \
               ORDER BY id DESC LIMIT 1";
    query_i64(conn, sql, vec![book_id.into()])
        .await
        .map(|found| found.map(|value| value as i32))
}

// ───────────────────────────── transactional writes ─────────────────────────────

/// The wire form of an [`AppError`] carried through `DbErr::Custom`.
///
/// [`crate::db::Db::write`] speaks `DbErr`, but services want typed errors. This wrapper encodes
/// an `AppError` as JSON inside `DbErr::Custom` and decodes it again, so no informative error is
/// ever lost to the transaction boundary. The marker keeps real database errors distinct.
const ERROR_MARKER: &str = "nekowriter-error:";

#[derive(Debug, Serialize, Deserialize)]
struct WireError {
    code: ErrorCode,
    message: String,
    details: Option<Value>,
}

/// Runs one write transaction through [`crate::db::Db::write`] with service-typed errors.
pub async fn write<T, F>(db: &crate::db::Db, f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: Send
        + 'static
        + for<'c> FnOnce(&'c DatabaseTransaction) -> BoxFuture<'c, Result<T, AppError>>,
{
    db.write(|tx| {
        Box::pin(async move {
            f(tx).await.map_err(|error| {
                let wire = WireError {
                    code: error.code,
                    message: error.message.clone(),
                    details: error.details.clone(),
                };
                match serde_json::to_string(&wire) {
                    Ok(json) => DbErr::Custom(format!("{ERROR_MARKER}{json}")),
                    // The error cannot be encoded (it never happens); keep the message.
                    Err(_) => DbErr::Custom(error.to_string()),
                }
            })
        })
    })
    .await
    .map_err(decode_db_error)
}

/// Recovers an `AppError` encoded by [`write`], or converts a genuine database error.
fn decode_db_error(error: DbErr) -> AppError {
    if let DbErr::Custom(message) = &error
        && let Some(json) = message.strip_prefix(ERROR_MARKER)
        && let Ok(wire) = serde_json::from_str::<WireError>(json)
    {
        return AppError {
            code: wire.code,
            message: wire.message,
            details: wire.details,
        };
    }
    AppError::from(error)
}
