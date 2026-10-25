//! Serves the single-page application (spec/01 §12).
//!
//! Three sources, in priority order:
//! 1. `--web-dir` / `NW_WEB_DIR`, when set (development without embedding);
//! 2. the assets embedded by the `embed-web` feature (the release binary);
//! 3. a plain hint page for `/`, telling the reader to start Vite.
//!
//! Unknown paths that are not under `/api` fall back to `index.html`, so a deep link
//! (`/books/1/chapters/2`) can be reloaded in the browser. `/api` paths never reach this module:
//! the API router is nested at `/api`, which matches every path under it.

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};

use crate::state::AppState;

/// The fallback for every path the API router does not match (spec/01 §12, spec/10 §1).
pub async fn serve(State(state): State<AppState>, method: Method, uri: Uri) -> Response {
    // Static files are reads. A `POST /unknown` is not a page request; answer it as such rather
    // than returning the SPA shell for an API-looking call that missed its endpoint.
    if method != Method::GET && method != Method::HEAD {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    }
    let path = uri.path().trim_start_matches('/');
    // The API router handles `/api`; a request that still arrives here (a nested 404) must not be
    // rewritten into an HTML page, which would break the JSON error contract of spec/10 §1.
    if path == "api" || path.starts_with("api/") {
        return plain(StatusCode::NOT_FOUND, "not found");
    }
    if let Some(dir) = state.config().web_dir.as_deref() {
        return serve_disk(dir, path);
    }
    #[cfg(feature = "embed-web")]
    {
        serve_embedded(path)
    }
    #[cfg(not(feature = "embed-web"))]
    {
        hint(path)
    }
}

/// The plain page served at `/` when neither `--web-dir` nor the `embed-web` feature supplies
/// assets (spec/01 §12).
pub fn hint_page() -> Html<&'static str> {
    Html(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>NekoWriter</title></head><body style=\"font-family: sans-serif; margin: 3rem\">\
         <h1>NekoWriter API is running</h1>\
         <p>Run the Vite dev server to use the studio:</p>\
         <pre>pnpm --dir web dev</pre>\
         <p>Then open <a href=\"http://localhost:5173\">http://localhost:5173</a>.</p>\
         <p>Health: <a href=\"/api/health\">/api/health</a></p>\
         </body></html>",
    )
}

#[cfg(not(feature = "embed-web"))]
fn hint(path: &str) -> Response {
    if path.is_empty() {
        hint_page().into_response()
    } else {
        plain(StatusCode::NOT_FOUND, "not found")
    }
}

/// Serves a file from `dir`, falling back to that directory's `index.html`.
fn serve_disk(dir: &Path, path: &str) -> Response {
    if let Some(relative) = safe_relative(path)
        && let Some(response) = read_disk_file(dir, &relative)
    {
        return response;
    }
    // SPA fallback: a deep link has no file of its own.
    match read_disk_file(dir, Path::new("index.html")) {
        Some(response) => response,
        None => plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the web directory has no index.html; run `pnpm --dir web build`",
        ),
    }
}

/// Reads one file under `dir` and builds its response, or `None` when it is not a regular file.
fn read_disk_file(dir: &Path, relative: &Path) -> Option<Response> {
    let full = dir.join(relative);
    if !full.is_file() {
        return None;
    }
    let bytes = std::fs::read(&full).ok()?;
    let key = relative.to_string_lossy().replace('\\', "/");
    Some(asset(bytes, content_type(&key), cache_control(&key)))
}

/// Rejects traversal (`..`, backslashes, NUL) and returns a path relative to the asset root.
///
/// Percent-encoded separators (`..%2F`) cannot escape: the path is never decoded and only an
/// existing file under `dir` is read.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\\') || segment.contains('\0') {
            return None;
        }
        out.push(segment);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Builds an asset response with its type and caching policy. The security headers of
/// spec/01 §10.6 are added by the outer middleware for every response, so they are not repeated
/// here.
fn asset(bytes: Vec<u8>, content_type: &'static str, cache_control: &'static str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}

/// Hashed build assets never change under their name; everything else revalidates.
fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    }
}

/// The content type of a static file, by extension. Kept explicit instead of using a MIME-guessing
/// dependency: the set of files a Vite build emits is small and known.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.') {
        Some((_, "html")) => "text/html; charset=utf-8",
        Some((_, "js")) | Some((_, "mjs")) => "text/javascript; charset=utf-8",
        Some((_, "css")) => "text/css; charset=utf-8",
        Some((_, "json")) | Some((_, "map")) => "application/json",
        Some((_, "svg")) => "image/svg+xml",
        Some((_, "png")) => "image/png",
        Some((_, "jpg")) | Some((_, "jpeg")) => "image/jpeg",
        Some((_, "webp")) => "image/webp",
        Some((_, "gif")) => "image/gif",
        Some((_, "ico")) => "image/x-icon",
        Some((_, "woff")) => "font/woff",
        Some((_, "woff2")) => "font/woff2",
        Some((_, "ttf")) => "font/ttf",
        Some((_, "txt")) => "text/plain; charset=utf-8",
        Some((_, "webmanifest")) => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn plain(status: StatusCode, message: &'static str) -> Response {
    (status, message).into_response()
}

/// The assets compiled into the binary by the `embed-web` feature (spec/01 §12).
#[cfg(feature = "embed-web")]
#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
struct WebAssets;

#[cfg(feature = "embed-web")]
fn serve_embedded(path: &str) -> Response {
    if let Some(response) = embedded_file(path) {
        return response;
    }
    match embedded_file("index.html") {
        Some(response) => response,
        None => plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the binary has no embedded index.html; build it with the embed-web feature",
        ),
    }
}

#[cfg(feature = "embed-web")]
fn embedded_file(path: &str) -> Option<Response> {
    if path.is_empty() {
        return None;
    }
    let file = WebAssets::get(path)?;
    Some(asset(
        file.data.into_owned(),
        content_type(path),
        cache_control(path),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_relative_rejects_traversal() {
        assert!(safe_relative("../etc/passwd").is_none());
        assert!(safe_relative("assets/../../etc/passwd").is_none());
        assert!(safe_relative("assets/..\\evil.js").is_none());
        assert!(safe_relative("").is_none());
        assert!(safe_relative("/").is_none());
        assert_eq!(
            safe_relative("assets/index-abc.js"),
            Some(PathBuf::from("assets/index-abc.js"))
        );
        // Percent-encoded separators are not decoded, so they cannot become `..`.
        assert_eq!(
            safe_relative("%2e%2e/secret"),
            Some(PathBuf::from("%2e%2e/secret"))
        );
    }

    #[test]
    fn content_types_cover_a_vite_build() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/index-abc123.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type("assets/index-abc123.css"),
            "text/css; charset=utf-8"
        );
        assert_eq!(content_type("assets/logo.svg"), "image/svg+xml");
        assert_eq!(content_type("favicon.ico"), "image/x-icon");
        assert_eq!(content_type("assets/font.woff2"), "font/woff2");
        assert_eq!(content_type("unknown"), "application/octet-stream");
    }

    #[test]
    fn hashed_assets_are_cached_and_the_shell_is_not() {
        assert!(cache_control("assets/index-abc.js").contains("immutable"));
        assert_eq!(cache_control("index.html"), "no-cache");
    }
}
