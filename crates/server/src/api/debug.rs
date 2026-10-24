//! Debug-only endpoints (spec/10 §2.7). Compiled only in debug builds, so a release binary
//! cannot start a test-only session.

use axum::Json;
use axum::extract::State;

use crate::api::dto::sessions::{DebugEchoDto, DebugEchoInput};
use crate::error::AppResult;
use crate::services::sessions;
use crate::state::AppState;

/// `POST /api/debug/sessions/echo` (spec/10 §2.7, PLAN M3.5): starts a session whose frozen tool
/// list is the test-only tools (`echo`, `test_write`, `test_finish`, `test_slow`), so the agent
/// panel can be driven end to end without any real tool.
pub async fn echo(
    State(state): State<AppState>,
    Json(input): Json<DebugEchoInput>,
) -> AppResult<Json<DebugEchoDto>> {
    Ok(Json(sessions::debug_echo(&state, input.book_id).await?))
}
