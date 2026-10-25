//! M8 review, finding F5: `text/plain` and `text/markdown` bodies pass through **as is**
//! (spec/07 §7 step 4). The HTML pipeline's whitespace collapse (step 6) was applied to them too,
//! which changes Markdown hard line breaks (two trailing spaces) and strips repeated blank lines.

use nekowriter::web::fetch::{self, Fetcher, PageCache};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Fetches a body served with `content_type` through a stand-in public host.
async fn fetch_raw(content_type: &str, body: &str) -> String {
    let server = MockServer::start().await;
    let address = *server.address();
    let fetcher = Fetcher::with_host_overrides(vec![(
        "public.example".to_owned(),
        std::net::SocketAddr::new(address.ip(), address.port()),
    )])
    .expect("fetcher");
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, content_type))
        .mount(&server)
        .await;
    let url = format!("http://public.example:{}/page", address.port());
    fetch::fetch(&fetcher, &PageCache::new(), &url)
        .await
        .expect("fetch")
        .markdown
}

#[tokio::test]
async fn a_markdown_page_keeps_its_bytes() {
    // A Markdown hard break is two trailing spaces, and a fenced block may contain blank runs:
    // neither may be altered by the HTML extraction rules (spec/07 §7 steps 4 and 6).
    let body = "# Title  \n\nline with a hard break  \nnext line\n\n\n\ncode:\n\n```\nfn x() {}\n\n\n```\n";
    assert_eq!(fetch_raw("text/markdown; charset=utf-8", body).await, body);
}

#[tokio::test]
async fn a_plain_text_page_keeps_its_bytes() {
    let body = "line one  \n\n\n\nline two\t\n";
    assert_eq!(fetch_raw("text/plain; charset=utf-8", body).await, body);
}
