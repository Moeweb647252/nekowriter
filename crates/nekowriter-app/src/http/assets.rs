//! The web UI, embedded in the binary.
//!
//! A path with a file extension is a static asset and is served as is, or answered with
//! 404 when missing. Any other path is a client-side route and gets `index.html`, so a
//! page can be reloaded or opened directly.

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

/// The production build of the web UI. A build without it still compiles (for tests and
/// development, where Vite serves the UI); `build.rs` refuses release builds without it.
#[derive(Embed)]
#[folder = "../../webui/dist/"]
#[allow_missing = true]
struct WebUi;

const INDEX: &str = "index.html";

pub(crate) fn ui_embedded() -> bool {
    WebUi::get(INDEX).is_some()
}

pub(crate) async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { INDEX } else { path };
    if let Some(response) = file(path) {
        return response;
    }
    let is_asset = path
        .rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'));
    if !is_asset && let Some(response) = file(INDEX) {
        return response;
    }
    StatusCode::NOT_FOUND.into_response()
}

fn file(path: &str) -> Option<Response> {
    let file = WebUi::get(path)?;
    // Vite puts content-hashed files under `assets/`; they never change.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = Response::new(Body::from(file.data));
    let headers = response.headers_mut();
    if let Ok(mime) = HeaderValue::from_str(file.metadata.mimetype()) {
        headers.insert(header::CONTENT_TYPE, mime);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    Some(response)
}
