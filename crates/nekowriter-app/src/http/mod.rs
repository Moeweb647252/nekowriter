//! HTTP 层：错误映射、DTO、路由、中间件与各域 handler。
//!
//! 这一层是按域从 `app/src/main.rs` 搬过来的（`docs/08` 的分批计划），每一批只移动、
//! 不改行为：被搬走的项在 `main.rs`（crate 根）里保留同名的 `pub(crate) use`，于是
//! `crate::ApiError`、`crate::AppState` 这些调用点一个字都不用改。
//!
//! 已经搬过来的：P1.1 的错误映射（`error` 子模块）、P1.2 的请求/响应形状（`dto` 子模块）。
//! P1.4 的共享状态（`state` 子模块）。
//! P1.5 的写信任边界（`gate` 子模块）、静态资源与前端入口（`static_files` 子模块），
//! 以及本文件里的 `router`、`api_not_found`、`shutdown_service`。
//!
//! 各域 handler 的文件划分由 `docs/08` 的「Phase 1 的 handler 模块边界（P1.5 定稿）」定下，
//! P1.6–P1.9 只按那张表搬；`router` 用一条 `use crate::{...}` 把 handler 名字接进来，所以路由
//! 表本身在搬迁过程中一行都不改。
//! P1.6 把 handler 挪到了 `handlers/` 下（`system`、`prompts` 已就位），那条 `use` 也相应变短。
//! P1.7 接上项目/版本/导出/项目设置（`projects`）。
//! P1.8 接上章节域与准备域（`chapters`、`preparation`）——这一批之后 `use crate::{...}` 只剩
//! 事件面与五个域模块，每搬一个域就短一行。
//! P1.9 把事件面接上（`events`）：那条 `use crate::{...}` 至此只剩五个域模块
//! （`dashboard` `knowledge` `manuscript` `recycle` `templates`），路由表每个 `.route(...)`
//! 仍是一字未改。
//!
//! P1.10 把 `main.rs` 的测试搬进了各目的模块，其中跨模块共用的 HTTP 测试装置与需要真实路由
//! 的契约测试在 `tests` 子模块里（只在 `cfg(test)` 下编译）。

use std::path::Path;

use crate::http::dto::ShutdownResponse;
use crate::http::error::ApiError;
use crate::http::gate::require_trusted_write;
use crate::http::handlers::chapters::{
    add_extra_chapter, advance_to_next_chapter, answer_question, append_chapter_body,
    confirm_novel_completion, continue_writing_run, decide_chapter_body_proposal, get_chapter_body,
    get_chapter_guidance, get_chapter_outline, get_document_revision, list_chapters,
    replace_chapter_body, request_chapter_body_pause, resume_chapter_body_pause,
    save_chapter_guidance, set_approval_mode, start_chapter_outline, stop_writing_run,
    submit_chapter_final_review, submit_chapter_outline, submit_run_feedback,
};
use crate::http::handlers::events::{get_event_cursor, stream_events};
use crate::http::handlers::preparation::{
    cancel_review_delegation, decide_review, delegate_review, get_preparation, save_novel_brief,
    start_preparation, submit_preparation_artifacts,
};
use crate::http::handlers::projects::{
    copy_version, create_project, create_version, export_version, get_project,
    get_project_settings, get_version_snapshot, list_projects, list_versions,
    save_project_settings, select_version,
};
use crate::http::handlers::prompts::{
    get_project_prompts, get_system_prompts, preview_prompt, reset_project_prompt_override,
    reset_system_prompt_override, save_project_prompt_override, save_system_prompt_override,
};
use crate::http::handlers::system::{
    get_capabilities, get_system_settings, health, save_system_settings, test_model_connection,
};
use crate::http::state::AppState;
use crate::http::static_files::{serve_client_route, serve_index_html};
use crate::{dashboard, knowledge, manuscript, recycle, templates};
use axum::{
    Json, Router,
    extract::State,
    http::Uri,
    middleware,
    routing::{get, put},
};
use tower_http::{services::ServeDir, trace::TraceLayer};

pub(crate) mod dto;
pub(crate) mod error;
pub(crate) mod gate;
pub(crate) mod handlers;
pub(crate) mod state;
pub(crate) mod static_files;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) fn router(state: AppState, web_dir: &Path) -> Router {
    let root_index_html = web_dir.join("index.html");
    let explicit_index_html = root_index_html.clone();
    // 深链接回落用同一份入口 HTML：前端用 path 路由，磁盘上没有 `/works/...` 这样的文件。
    let fallback_index_html = root_index_html.clone();
    let static_files = web_dir.to_owned();
    let api = Router::new()
        .route("/health", get(health))
        .route("/system/shutdown", axum::routing::post(shutdown_service))
        .route("/capabilities", get(get_capabilities))
        .route(
            "/settings",
            get(get_system_settings).put(save_system_settings),
        )
        .route(
            "/settings/test-model",
            axum::routing::post(test_model_connection),
        )
        .route(
            "/settings/embedding",
            get(knowledge::get_embedding_settings).put(knowledge::save_embedding_settings),
        )
        .route(
            "/settings/test-embedding",
            axum::routing::post(knowledge::test_embedding_connection),
        )
        .route(
            "/knowledge/spaces",
            get(knowledge::list_knowledge_spaces).post(knowledge::create_knowledge_space),
        )
        .route(
            "/knowledge/spaces/{space_id}",
            axum::routing::patch(knowledge::rename_knowledge_space)
                .delete(knowledge::delete_knowledge_space),
        )
        .route(
            "/knowledge/entries",
            get(knowledge::list_knowledge_entries).post(knowledge::create_knowledge_entry),
        )
        .route(
            "/knowledge/entries/{entry_id}",
            get(knowledge::get_knowledge_entry)
                .put(knowledge::update_knowledge_entry)
                .delete(knowledge::delete_knowledge_entry),
        )
        .route(
            "/knowledge/search",
            axum::routing::post(knowledge::search_knowledge),
        )
        .route(
            "/knowledge/reindex",
            axum::routing::post(knowledge::reindex_knowledge),
        )
        .route(
            "/knowledge/accumulate",
            axum::routing::post(knowledge::accumulate_knowledge),
        )
        .route("/projects", get(list_projects).post(create_project))
        .route(
            "/projects/{project_id}",
            get(get_project).delete(recycle::delete_project),
        )
        .route(
            "/projects/{project_id}/settings",
            get(get_project_settings).put(save_project_settings),
        )
        .route(
            "/projects/{project_id}/versions",
            get(list_versions).post(create_version),
        )
        .route(
            "/projects/{project_id}/selected-version",
            axum::routing::post(select_version),
        )
        .route("/versions/{version_id}/snapshot", get(get_version_snapshot))
        .route("/versions/{version_id}/export", get(export_version))
        .route(
            "/versions/{version_id}/copy",
            axum::routing::post(copy_version),
        )
        .route(
            "/versions/{version_id}/approval-mode",
            axum::routing::post(set_approval_mode),
        )
        .route("/versions/{version_id}/chapters", get(list_chapters))
        .route(
            "/versions/{version_id}/manuscript",
            get(manuscript::get_manuscript_window),
        )
        .route(
            "/versions/{version_id}/manuscript/search",
            get(manuscript::search_manuscript),
        )
        .route(
            "/versions/{version_id}/chapters/next",
            axum::routing::post(advance_to_next_chapter),
        )
        .route(
            "/versions/{version_id}/completion",
            axum::routing::post(confirm_novel_completion),
        )
        .route(
            "/versions/{version_id}/extras",
            axum::routing::post(add_extra_chapter),
        )
        .route(
            "/chapters/{chapter_id}/guidance",
            get(get_chapter_guidance).put(save_chapter_guidance),
        )
        .route(
            "/chapters/{chapter_id}/outline/start",
            axum::routing::post(start_chapter_outline),
        )
        .route(
            "/chapters/{chapter_id}/outline",
            get(get_chapter_outline).post(submit_chapter_outline),
        )
        .route("/chapters/{chapter_id}/body", get(get_chapter_body))
        .route(
            "/documents/{document_id}/revisions/{revision_id}",
            get(get_document_revision),
        )
        .route(
            "/chapters/{chapter_id}/body/append",
            axum::routing::post(append_chapter_body),
        )
        .route(
            "/chapters/{chapter_id}/body/replace",
            axum::routing::post(replace_chapter_body),
        )
        .route(
            "/chapters/{chapter_id}/body/proposals/{tool_call_id}/decision",
            axum::routing::post(decide_chapter_body_proposal),
        )
        .route(
            "/runs/{run_id}/feedback",
            axum::routing::post(submit_run_feedback),
        )
        .route(
            "/runs/{run_id}/continue",
            axum::routing::post(continue_writing_run),
        )
        .route("/runs/{run_id}/stop", axum::routing::post(stop_writing_run))
        .route(
            "/questions/{question_id}/answer",
            axum::routing::post(answer_question),
        )
        .route(
            "/runs/{run_id}/pause",
            axum::routing::post(request_chapter_body_pause),
        )
        .route(
            "/runs/{run_id}/resume",
            axum::routing::post(resume_chapter_body_pause),
        )
        .route(
            "/chapters/{chapter_id}/body/review",
            axum::routing::post(submit_chapter_final_review),
        )
        .route("/events/cursor", get(get_event_cursor))
        .route("/events", get(stream_events))
        .route("/versions/{version_id}/preparation", get(get_preparation))
        .route("/versions/{version_id}/brief", put(save_novel_brief))
        .route("/prompts/system", get(get_system_prompts))
        .route(
            "/prompts/system/{prompt_key}",
            put(save_system_prompt_override).delete(reset_system_prompt_override),
        )
        .route("/prompts/preview", axum::routing::post(preview_prompt))
        .route(
            "/prompt-templates",
            get(templates::list_prompt_templates).post(templates::create_prompt_template),
        )
        .route(
            "/prompt-templates/{template_id}",
            get(templates::get_prompt_template)
                .patch(templates::update_prompt_template)
                .delete(templates::delete_prompt_template),
        )
        .route("/dashboard", get(dashboard::get_dashboard))
        .route("/recycle-bin", get(recycle::get_recycle_bin))
        .route(
            "/recycle-bin/{kind}/{id}/restore",
            axum::routing::post(recycle::restore_recycle_item),
        )
        .route(
            "/recycle-bin/{kind}/{id}/purge",
            axum::routing::post(recycle::purge_recycle_item),
        )
        .route(
            "/chapters/{chapter_id}",
            axum::routing::delete(recycle::delete_extra_chapter),
        )
        .route(
            "/versions/{version_id}",
            axum::routing::delete(recycle::delete_version),
        )
        .route("/projects/{project_id}/prompts", get(get_project_prompts))
        .route(
            "/projects/{project_id}/prompts/{prompt_key}",
            put(save_project_prompt_override).delete(reset_project_prompt_override),
        )
        .route(
            "/versions/{version_id}/preparation/start",
            axum::routing::post(start_preparation),
        )
        .route(
            "/versions/{version_id}/preparation/artifacts",
            axum::routing::post(submit_preparation_artifacts),
        )
        .route(
            "/reviews/{review_id}/decision",
            axum::routing::post(decide_review),
        )
        .route(
            "/reviews/{review_id}/delegate",
            axum::routing::post(delegate_review),
        )
        .route(
            "/reviews/{review_id}/cancel-delegation",
            axum::routing::post(cancel_review_delegation),
        )
        .fallback(api_not_found)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_trusted_write,
        ))
        .with_state(state);

    Router::new()
        .nest("/api", api)
        .route("/", get(move || serve_index_html(root_index_html.clone())))
        .route(
            "/index.html",
            get(move || serve_index_html(explicit_index_html.clone())),
        )
        .fallback_service(
            ServeDir::new(static_files)
                .append_index_html_on_directories(true)
                .fallback(get(move |uri: Uri| {
                    let index_html = fallback_index_html.clone();
                    async move { serve_client_route(uri, index_html).await }
                })),
        )
        .layer(TraceLayer::new_for_http())
}

async fn api_not_found() -> ApiError {
    ApiError::NotFound
}

/// 显式退出本地服务：与进程信号走同一条收尾路径。
///
/// 响应先写回客户端，服务随后停止接受连接并取消在途请求；界面据此显示连接已断开。
async fn shutdown_service(
    State(state): State<AppState>,
) -> Result<Json<ShutdownResponse>, ApiError> {
    if state.shutdown.is_requested() {
        return Ok(Json(ShutdownResponse {
            status: "already_shutting_down",
        }));
    }
    state.shutdown.request_after_response();
    Ok(Json(ShutdownResponse {
        status: "shutting_down",
    }))
}
