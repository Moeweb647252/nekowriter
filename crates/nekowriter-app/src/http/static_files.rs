//! 静态资源与前端入口：静态目录未命中时的回落，以及入口 HTML 的响应头。
//!
//! 从 `app/src/main.rs` 整块搬来（`docs/08` 的 P1.5），代码一行未改，只有可见性从私有放宽到
//! `pub(crate)`（`http::router` 要挂这两条路由与回落，入口 HTML 的响应头测试要直接调它）。

use std::path::PathBuf;

use axum::{
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use tracing::warn;

/// 静态目录未命中时的回落：只有前端路由形的路径回落到入口 HTML。
///
/// 前端用 path 路由，磁盘上没有 `/works/<id>` 这样的文件，直接打开或刷新深链接必须由
/// 路由接管，否则用户拿到的是一个空白 404。带扩展名的路径是静态资源（脚本、样式、
/// 图片、字体）：磁盘上没有就如实 404，回落到 HTML 会把真正的缺失掩盖成界面故障——
/// 浏览器把 HTML 当脚本执行（MIME 不匹配）或把坏资源当成加载成功。
pub(crate) async fn serve_client_route(uri: Uri, index_html: PathBuf) -> Response {
    if is_static_asset_path(uri.path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve_index_html(index_html).await
}

/// 末段带 `.` 的路径按静态资源看待；前端路由段（UUID、页面名）里不含点。
fn is_static_asset_path(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|segment| segment.contains('.'))
}

/// 入口 HTML 不能被启发式缓存：每次启动新版本时，它必须引用同一构建产物里的懒加载 chunk。
pub(crate) async fn serve_index_html(path: PathBuf) -> Response {
    match tokio::fs::read(&path).await {
        Ok(content) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                ),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            ],
            content,
        )
            .into_response(),
        Err(error) => {
            warn!(path = %path.display(), %error, "未找到前端入口文件");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use nekowriter_agent::engine::ACCUMULATION_ENGINES;
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::http::tests::{accumulation_app, spawn_app_with_web_dir};

    use super::*;

    #[tokio::test]
    async fn index_html_is_revalidated_before_loading_route_chunks() {
        let web_dir = TempDir::new().unwrap();
        let index_html = web_dir.path().join("index.html");
        std::fs::write(&index_html, "<!doctype html><title>NekoWriter</title>").unwrap();

        let response = serve_index_html(index_html).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-cache"))
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/html; charset=utf-8"))
        );
    }

    /// 路由层的三条边界：深链接回落、静态资源、以及 `/api` 不被回落吞掉。
    ///
    /// 前端用 path 路由，而磁盘上没有 `/works/...` 这样的文件：发行目录（而不是 Vite
    /// 开发服务器）里必须回落到入口 HTML，否则刷新或直接打开一个深链接只会看到 404。
    /// 反过来，未知的 `/api/*` 必须仍然是 JSON 错误信封，不能被回落成一份 HTML。
    #[tokio::test]
    async fn deep_links_fall_back_to_the_entry_html_without_swallowing_api_errors() {
        let dir = TempDir::new().unwrap();
        let web_dir = dir.path().join("dist");
        tokio::fs::create_dir_all(web_dir.join("assets"))
            .await
            .unwrap();
        tokio::fs::write(web_dir.join("index.html"), "<!doctype html>入口")
            .await
            .unwrap();
        tokio::fs::write(web_dir.join("assets").join("app.js"), "console.log(1)")
            .await
            .unwrap();

        let (_data_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app_with_web_dir(state, &web_dir);
        let client = reqwest::Client::new();

        let deep_link = client
            .get(format!("{base}/works/abc/versions/def"))
            .send()
            .await
            .unwrap();
        assert_eq!(deep_link.status(), StatusCode::OK);
        assert!(deep_link.text().await.unwrap().contains("入口"));

        let asset = client
            .get(format!("{base}/assets/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(asset.text().await.unwrap(), "console.log(1)");

        let unknown_api = client.get(format!("{base}/api/nope")).send().await.unwrap();
        assert_eq!(unknown_api.status(), StatusCode::NOT_FOUND);
        let body: Value = unknown_api.json().await.unwrap();
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    /// 静态目录与 `/api` 的边界：前端深链接回落，静态资源与未知接口如实报错。
    ///
    /// 走真实 HTTP 而不是读代码：`/api/*` 必须拿到 JSON 错误信封（不被 SPA 回落吞掉），
    /// 缺失的静态资源必须 404（不被回落成 200 + HTML）。
    #[tokio::test]
    async fn static_dir_serves_client_routes_and_reports_missing_assets_honestly() {
        let web_dir = TempDir::new().unwrap();
        std::fs::write(
            web_dir.path().join("index.html"),
            "<!doctype html><div id=\"root\">NEKOWRITER_INDEX</div>",
        )
        .unwrap();
        std::fs::create_dir_all(web_dir.path().join("assets")).unwrap();
        std::fs::write(
            web_dir.path().join("assets/app.js"),
            "console.log('real-asset');",
        )
        .unwrap();

        let (_dir, state) = accumulation_app(ACCUMULATION_ENGINES).await;
        let base = spawn_app_with_web_dir(state, web_dir.path());
        let client = reqwest::Client::new();

        // 前端路由（深链接）回落到入口 HTML：直接打开或刷新这些地址必须是界面而不是 404。
        for path in ["/", "/settings", "/works/abc/versions/def"] {
            let response = client.get(format!("{base}{path}")).send().await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "{path} 未回落到入口 HTML"
            );
            let content_type = content_type(&response);
            assert!(
                content_type.starts_with("text/html"),
                "{path} 的内容类型是 {content_type}"
            );
            let body = response.text().await.unwrap();
            assert!(
                body.contains("NEKOWRITER_INDEX"),
                "{path} 拿到的不是入口 HTML"
            );
        }

        // 磁盘上存在的静态资源按真实路径返回。
        let asset = client
            .get(format!("{base}/assets/app.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        assert!(asset.text().await.unwrap().contains("real-asset"));

        // 不存在的静态资源如实 404。（存量检查：回落成 200 + HTML 会让浏览器把它当脚本执行。）
        let missing = client
            .get(format!("{base}/assets/removed-chunk.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            missing.status(),
            StatusCode::NOT_FOUND,
            "缺失的静态资源被回落成了 HTML"
        );

        // 未知的 `/api` 路径是 JSON 错误信封，而不是入口 HTML。
        let unknown = client
            .get(format!("{base}/api/definitely-not-a-route"))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let content_type = content_type(&unknown);
        assert!(
            content_type.starts_with("application/json"),
            "未知接口的内容类型是 {content_type}"
        );
        let body: Value = unknown.json().await.unwrap();
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    fn content_type(response: &reqwest::Response) -> String {
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    }
}
