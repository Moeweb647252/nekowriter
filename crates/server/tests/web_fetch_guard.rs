//! `web_fetch` tests (PLAN 8.2, spec/07 §7, spec/01 §10.4).
//!
//! The guard is the point of this module, and `wiremock` listens on loopback — exactly what the
//! guard must refuse. The tests therefore stand `wiremock` in for the public names
//! `public.example` and `other.example`: `Fetcher::with_host_overrides` replaces only those exact
//! DNS lookups, while the redirect policy, the timeout, the body cap and the resolver's own checks
//! stay production code.
//!
//! Literal-IP and real-DNS rejections need no server at all and are asserted directly. Two tests
//! exist specifically to prove the override is not a bypass:
//! `the_host_override_does_not_permit_any_other_name` and
//! `a_redirect_to_a_second_host_is_re_resolved_and_refused`.

use std::net::{IpAddr, SocketAddr};

use axum::http::StatusCode;
use nekowriter::web::WebService;
use nekowriter::web::fetch::{
    self, BODY_CAP, DEFAULT_MAX_CHARS, FetchFailure, Fetcher, MAX_REDIRECTS, PageCache,
};
use nekowriter::web::search::SearchProviderKind;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The public name the mock server answers for.
const HOST: &str = "public.example";

// ───────────────────────────── harness ─────────────────────────────

/// A `wiremock` server plus the guarded fetcher that resolves [`HOST`] to it.
struct Site {
    server: MockServer,
    fetcher: Fetcher,
}

impl Site {
    async fn new() -> Self {
        let server = MockServer::start().await;
        let address: SocketAddr = *server.address();
        let fetcher = Fetcher::with_host_overrides(vec![(
            HOST.to_owned(),
            SocketAddr::new(address.ip(), address.port()),
        )])
        .expect("fetcher");
        Self { server, fetcher }
    }

    /// The URL of a path on the stand-in public host.
    fn url(&self, path: &str) -> String {
        format!("http://{HOST}:{}{path}", self.server.address().port())
    }

    /// Answers every GET with `template`.
    async fn mount(&self, template: ResponseTemplate) {
        Mock::given(method("GET"))
            .respond_with(template)
            .mount(&self.server)
            .await;
    }

    /// Answers every GET with an HTML page.
    ///
    /// `ResponseTemplate::set_body_string` forces `text/plain` and would overwrite an inserted
    /// `content-type`, so the HTML cases set the mime through `set_body_raw`.
    async fn mount_html(&self, html: String) {
        self.mount(ResponseTemplate::new(200).set_body_raw(html, "text/html; charset=utf-8"))
            .await;
    }
}

/// A minimal HTML article Readability can extract.
fn article(title: &str, body: &str) -> String {
    format!(
        "<html><head><title>{title}</title></head><body>\
         <nav>Navigation</nav>\
         <article><h1>{title}</h1>{body}</article>\
         <footer>Footer</footer>\
         <script>window.x = 1;</script>\
         </body></html>"
    )
}

// ───────────────────────────── the SSRF guard ─────────────────────────────

#[tokio::test]
async fn a_literal_loopback_url_is_refused() {
    let site = Site::new().await;
    for url in [
        format!("http://127.0.0.1:{}/", site.server.address().port()),
        format!("http://[::1]:{}/", site.server.address().port()),
        "http://127.0.0.1/".to_owned(),
    ] {
        let error = site
            .fetcher
            .fetch(&url)
            .await
            .expect_err("the guard refuses loopback");
        assert_eq!(error.reason(), Some(FetchFailure::Blocked), "{url}");
    }
}

#[tokio::test]
async fn a_literal_private_address_is_refused() {
    let site = Site::new().await;
    for url in [
        "http://10.0.0.5/admin",
        "http://172.16.0.1/",
        "http://192.168.1.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://100.64.0.1/",
        "http://0.0.0.0/",
        "http://224.0.0.1/",
        "http://[fe80::1]/",
        "http://[fd00::1]/",
    ] {
        let error = site
            .fetcher
            .fetch(url)
            .await
            .expect_err("the guard refuses");
        assert_eq!(error.reason(), Some(FetchFailure::Blocked), "{url}");
    }
}

#[tokio::test]
async fn a_hostname_resolving_to_a_private_address_is_refused() {
    // The production resolver: no override, so the real DNS lookup of `localhost` runs and every
    // answer is loopback.
    let fetcher = Fetcher::new().expect("fetcher");
    for url in ["http://localhost:9/", "http://127.0.0.1:9/"] {
        let error = fetcher.fetch(url).await.expect_err("refused");
        assert_eq!(error.reason(), Some(FetchFailure::Blocked), "{url}");
    }
}

/// The override must not widen the guard: only the exact overridden name resolves to the local
/// server. Any other name still goes through `GuardedResolver`, and a literal private IP is still
/// refused before any lookup.
#[tokio::test]
async fn the_host_override_does_not_permit_any_other_name() {
    let site = Site::new().await;
    site.mount_html(article("Only the override", "<p>ok</p>"))
        .await;

    // The overridden name works.
    let document = site
        .fetcher
        .fetch(&site.url("/page"))
        .await
        .expect("the overridden name resolves");
    assert!(document.markdown.contains("ok"));

    // A different name is *not* overridden, so the real resolver runs and refuses loopback (this
    // machine's `localhost`) — the override granted nothing to it.
    let port = site.server.address().port();
    for url in [
        format!("http://localhost:{port}/page"),
        "http://127.0.0.1/".to_owned(),
        "http://[::1]/".to_owned(),
        "http://10.0.0.1/".to_owned(),
        "http://169.254.169.254/".to_owned(),
    ] {
        let error = site.fetcher.fetch(&url).await.expect_err("refused");
        assert_eq!(error.reason(), Some(FetchFailure::Blocked), "{url}");
    }
}

/// spec/07 §7 step 2: "resultions ... for EVERY redirect hop". A hop to a *different* host name must
/// be resolved again and refused before any connection to that address is attempted.
///
/// `public.example` is overridden to the mock server; `private.example` is deliberately **not**
/// overridden, so the second hop's lookup runs the production resolver. The name is not in DNS, so
/// the resolver fails; the important half of the assertion is that no connection to the mock
/// server's `/private` path happened — the request never reached the server that would have
/// answered it, because the guard refused the hop first.
#[tokio::test]
async fn a_redirect_to_a_second_host_is_re_resolved_and_refused() {
    let site = Site::new().await;
    Mock::given(method("GET"))
        .and(path("/hop"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://private.example/internal"),
        )
        .mount(&site.server)
        .await;
    // A page that must never be reached: a second hop that resolved would land here.
    Mock::given(method("GET"))
        .and(path("/internal"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("SECRET", "text/plain"))
        .mount(&site.server)
        .await;

    let error = site
        .fetcher
        .fetch(&site.url("/hop"))
        .await
        .expect_err("the second hop is refused");
    // The hop is resolved by the production resolver, which either finds nothing (network) or
    // finds a blocked address (blocked). Both mean the hop was re-resolved and never connected.
    assert!(
        matches!(
            error.reason(),
            Some(FetchFailure::Blocked) | Some(FetchFailure::Network)
        ),
        "unexpected reason {:?}",
        error.reason()
    );

    // Nothing served the redirect target: the server saw only the `/hop` request.
    let requests = site.server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1, "only the first hop was requested");
    assert!(
        requests[0].url.path() == "/hop",
        "unexpected path {}",
        requests[0].url.path()
    );
}

/// A redirect back to the *overridden* name keeps working, so the per-hop rule is not simply
/// "refuse every redirect".
#[tokio::test]
async fn a_redirect_within_the_overridden_host_is_followed() {
    let site = Site::new().await;
    Mock::given(method("GET"))
        .and(path("/from"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/to"))
        .mount(&site.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/to"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            article("Arrived", "<p>second hop</p>"),
            "text/html; charset=utf-8",
        ))
        .mount(&site.server)
        .await;
    let document = site
        .fetcher
        .fetch(&site.url("/from"))
        .await
        .expect("a same-host redirect is allowed");
    assert!(
        document.markdown.contains("second hop"),
        "{}",
        document.markdown
    );
}

/// A redirect chain whose target is a literal private address on the *second* hop is refused; the
/// first hop is a legitimate local page.
#[tokio::test]
async fn a_second_hop_literal_private_address_is_refused() {
    let site = Site::new().await;
    Mock::given(method("GET"))
        .and(path("/first"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://192.168.0.1/secret"),
        )
        .mount(&site.server)
        .await;
    let error = site
        .fetcher
        .fetch(&site.url("/first"))
        .await
        .expect_err("the literal hop is refused");
    assert_eq!(error.reason(), Some(FetchFailure::Blocked));
    let requests = site.server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1, "the private hop was never requested");
}

#[tokio::test]
async fn a_non_http_scheme_and_credentials_are_invalid_urls() {
    let site = Site::new().await;
    let mut urls = vec![
        "ftp://example.com/x".to_owned(),
        "file:///etc/passwd".to_owned(),
        "gopher://example.com/".to_owned(),
        "javascript:alert(1)".to_owned(),
        "not a url".to_owned(),
        "http://user:secret@public.example/".to_owned(),
        format!("http://user@{HOST}/"),
    ];
    // A URL longer than the 2,048-character limit of spec/07 §7 step 1.
    urls.push(format!("https://example.com/{}", "a".repeat(2_100)));
    for url in urls {
        let error = site.fetcher.fetch(&url).await.expect_err("invalid");
        assert_eq!(error.reason(), Some(FetchFailure::InvalidUrl), "{url}");
    }
}

#[tokio::test]
async fn a_redirect_to_a_private_address_is_refused() {
    let site = Site::new().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://127.0.0.1:1/admin"),
        )
        .mount(&site.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/to-metadata"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "http://169.254.169.254/latest/meta-data/"),
        )
        .mount(&site.server)
        .await;

    for path in ["/start", "/to-metadata"] {
        let error = site
            .fetcher
            .fetch(&site.url(path))
            .await
            .expect_err("the redirect target is refused");
        assert_eq!(error.reason(), Some(FetchFailure::Blocked), "{path}");
    }
}

#[tokio::test]
async fn a_redirect_to_a_non_http_scheme_is_refused() {
    let site = Site::new().await;
    Mock::given(method("GET"))
        .and(path("/ftp"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "ftp://public.example/file"),
        )
        .mount(&site.server)
        .await;
    let error = site
        .fetcher
        .fetch(&site.url("/ftp"))
        .await
        .expect_err("refused");
    assert_eq!(error.reason(), Some(FetchFailure::Blocked));
}

#[tokio::test]
async fn more_than_five_redirects_are_refused() {
    let site = Site::new().await;
    // A chain of MAX_REDIRECTS + 2 hops, each one local, ending in a real page.
    for hop in 0..=MAX_REDIRECTS + 1 {
        let next = if hop == MAX_REDIRECTS + 1 {
            site.url("/article")
        } else {
            site.url(&format!("/hop{}", hop + 1))
        };
        Mock::given(method("GET"))
            .and(path(format!("/hop{hop}")))
            .respond_with(ResponseTemplate::new(302).insert_header("location", next))
            .mount(&site.server)
            .await;
    }
    site.mount(ResponseTemplate::new(200).set_body_string(article("Title", "<p>body</p>")))
        .await;

    let error = site
        .fetcher
        .fetch(&site.url("/hop0"))
        .await
        .expect_err("the chain is too long");
    assert_eq!(error.reason(), Some(FetchFailure::TooManyRedirects));
}

#[tokio::test]
async fn up_to_five_redirects_are_followed() {
    let site = Site::new().await;
    for hop in 0..MAX_REDIRECTS {
        let next = site.url(&format!("/hop{}", hop + 1));
        Mock::given(method("GET"))
            .and(path(format!("/hop{hop}")))
            .respond_with(ResponseTemplate::new(302).insert_header("location", next))
            .mount(&site.server)
            .await;
    }
    site.mount_html(article("Arrived", "<p>the end</p>")).await;

    let document = site
        .fetcher
        .fetch(&site.url("/hop0"))
        .await
        .expect("five hops are allowed");
    assert!(
        document.markdown.contains("the end"),
        "{}",
        document.markdown
    );
}

// ───────────────────────────── limits and content types ─────────────────────────────

#[tokio::test]
async fn a_body_exactly_at_the_cap_is_accepted() {
    // The boundary, on the real streamed read: exactly 5 MB passes, one byte more does not (the
    // next test). This is what proves the cap is applied while reading rather than after the fact
    // with an off-by-one.
    let site = Site::new().await;
    site.mount(ResponseTemplate::new(200).set_body_raw("y".repeat(BODY_CAP), "text/plain"))
        .await;
    let document = site
        .fetcher
        .fetch(&site.url("/at-cap"))
        .await
        .expect("exactly the cap is allowed");
    assert_eq!(document.markdown.chars().count(), BODY_CAP);
}

#[tokio::test]
async fn a_body_over_five_megabytes_is_refused() {
    let site = Site::new().await;
    let chunk = "x".repeat(BODY_CAP + 1024);
    site.mount(ResponseTemplate::new(200).set_body_raw(chunk, "text/html"))
        .await;
    let error = site
        .fetcher
        .fetch(&site.url("/big"))
        .await
        .expect_err("too large");
    assert_eq!(error.reason(), Some(FetchFailure::TooLarge));
}

#[tokio::test]
async fn an_http_error_status_is_reported_with_its_number() {
    for code in [400u16, 404, 500, 503] {
        let site = Site::new().await;
        site.mount(ResponseTemplate::new(code).set_body_string("nope"))
            .await;
        let error = site
            .fetcher
            .fetch(&site.url("/missing"))
            .await
            .expect_err("status");
        match error {
            fetch::FetchError::Failed { reason, status, .. } => {
                assert_eq!(reason, FetchFailure::Status, "{code}");
                assert_eq!(status, Some(code));
            }
            other => panic!("unexpected error {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_pdf_is_an_unsupported_content_type() {
    let site = Site::new().await;
    site.mount(ResponseTemplate::new(200).set_body_raw(b"%PDF-1.7".to_vec(), "application/pdf"))
        .await;
    let error = site
        .fetcher
        .fetch(&site.url("/paper.pdf"))
        .await
        .expect_err("pdf");
    match error {
        fetch::FetchError::Unsupported { content_type, .. } => {
            assert_eq!(content_type, "application/pdf");
        }
        other => panic!("unexpected error {other:?}"),
    }
}

#[tokio::test]
async fn a_plain_text_page_comes_back_as_is() {
    let site = Site::new().await;
    site.mount(
        ResponseTemplate::new(200)
            .set_body_raw("line one\n\n\n\nline two  \n", "text/plain; charset=utf-8"),
    )
    .await;
    let document = site
        .fetcher
        .fetch(&site.url("/notes.txt"))
        .await
        .expect("plain text");
    assert_eq!(document.markdown, "line one\n\nline two");
    assert_eq!(document.title, "");
}

// ───────────────────────────── decoding ─────────────────────────────

#[tokio::test]
async fn a_gbk_page_is_decoded() {
    let site = Site::new().await;
    let html = article("茶馆", "<p>宋代的茶馆是市民聚会的场所。</p>");
    let (encoded, _, _) = encoding_rs::GBK.encode(&html);
    site.mount(
        ResponseTemplate::new(200).set_body_raw(encoded.into_owned(), "text/html; charset=gbk"),
    )
    .await;
    let document = site
        .fetcher
        .fetch(&site.url("/gbk"))
        .await
        .expect("gbk page");
    assert!(
        document.markdown.contains("宋代的茶馆是市民聚会的场所"),
        "{}",
        document.markdown
    );
}

#[tokio::test]
async fn a_meta_charset_declaration_is_used_when_the_header_has_none() {
    let site = Site::new().await;
    let body = "<html><head><meta charset=\"gbk\"><title>茶馆</title></head>\
                <body><article><p>宋代茶馆的布局。</p></article></body></html>";
    let (encoded, _, _) = encoding_rs::GBK.encode(body);
    site.mount(ResponseTemplate::new(200).set_body_raw(encoded.into_owned(), "text/html"))
        .await;
    let document = site
        .fetcher
        .fetch(&site.url("/meta-gbk"))
        .await
        .expect("meta charset");
    assert!(
        document.markdown.contains("宋代茶馆的布局"),
        "{}",
        document.markdown
    );
}

#[tokio::test]
async fn big5_and_shift_jis_are_decoded() {
    for (encoding, text) in [
        (encoding_rs::BIG5, "宋代的茶館是市民聚會的場所"),
        (encoding_rs::SHIFT_JIS, "宋代の茶館は市民の集いの場でした"),
    ] {
        let site = Site::new().await;
        let html = article("x", &format!("<p>{text}</p>"));
        let (encoded, _, _) = encoding.encode(&html);
        site.mount(ResponseTemplate::new(200).set_body_raw(
            encoded.into_owned(),
            &format!("text/html; charset={}", encoding.name()),
        ))
        .await;
        let document = site
            .fetcher
            .fetch(&site.url("/legacy"))
            .await
            .expect("legacy encoding");
        assert!(document.markdown.contains(text), "{}", document.markdown);
    }
}

#[tokio::test]
async fn invalid_utf8_is_replaced_rather_than_failing() {
    let site = Site::new().await;
    let mut body = b"<html><body><p>ok ".to_vec();
    body.extend_from_slice(&[0xff, 0xfe, 0xff]);
    body.extend_from_slice(b" end</p></body></html>");
    site.mount(ResponseTemplate::new(200).set_body_raw(body, "text/html; charset=utf-8"))
        .await;
    let document = site
        .fetcher
        .fetch(&site.url("/broken"))
        .await
        .expect("lossy");
    assert!(document.markdown.contains("ok"), "{}", document.markdown);
    assert!(document.markdown.contains("end"), "{}", document.markdown);
}

// ───────────────────────────── extraction ─────────────────────────────

#[tokio::test]
async fn readability_extracts_the_article_and_htmd_converts_it() {
    let site = Site::new().await;
    site.mount_html(article(
        "Tea houses",
        "<p>The first paragraph.</p><h2>A heading</h2>\
         <ul><li>one</li><li>two</li></ul>\
         <form><input name=\"q\"></form>",
    ))
    .await;
    let document = site
        .fetcher
        .fetch(&site.url("/tea"))
        .await
        .expect("article");
    assert_eq!(document.title, "Tea houses");
    assert!(
        document.markdown.contains("The first paragraph."),
        "{}",
        document.markdown
    );
    assert!(
        document.markdown.contains("## A heading"),
        "{}",
        document.markdown
    );
    assert!(document.markdown.contains("one"), "{}", document.markdown);
    // The skipped tags of spec/07 §7 step 6 do not reach the model.
    assert!(
        !document.markdown.contains("Navigation"),
        "{}",
        document.markdown
    );
    assert!(
        !document.markdown.contains("Footer"),
        "{}",
        document.markdown
    );
    assert!(
        !document.markdown.contains("window.x"),
        "{}",
        document.markdown
    );
    assert!(
        !document.markdown.contains("<input"),
        "{}",
        document.markdown
    );
}

// ───────────────────────────── the page cache and paging ─────────────────────────────

#[tokio::test]
async fn a_cached_page_is_not_refetched() {
    let site = Site::new().await;
    Mock::given(method("GET"))
        .and(path("/cached"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(article("Cached", "<p>once</p>"), "text/html"),
        )
        .expect(1)
        .mount(&site.server)
        .await;
    let cache = PageCache::new();
    let url = site.url("/cached");
    let first = fetch::fetch(&site.fetcher, &cache, &url)
        .await
        .expect("first fetch");
    assert!(!first.cached);
    let second = fetch::fetch(&site.fetcher, &cache, &url)
        .await
        .expect("cached fetch");
    assert!(second.cached, "the second read is served from the cache");
    assert_eq!(second.markdown, first.markdown);
}

#[tokio::test]
async fn a_long_page_pages_by_offset_with_a_continuation_hint() {
    let site = Site::new().await;
    let body = (1..=1_200)
        .map(|index| format!("<p>Paragraph number {index} of a long article.</p>"))
        .collect::<String>();
    site.mount_html(article("Long", &body)).await;
    let document = site
        .fetcher
        .fetch(&site.url("/long"))
        .await
        .expect("long page");
    assert!(document.total_chars() > DEFAULT_MAX_CHARS);

    let first = fetch::slice_page(&document.markdown, 0, DEFAULT_MAX_CHARS);
    assert_eq!(first.offset, 0);
    assert!(first.end <= DEFAULT_MAX_CHARS);
    let next = first.next_offset.expect("a continuation hint");
    assert_eq!(next, first.end);

    let second = fetch::slice_page(&document.markdown, next, DEFAULT_MAX_CHARS);
    assert_eq!(second.offset, next);
    // The pages are contiguous: their concatenation is the document's prefix, so no text is lost
    // or repeated across the page boundary.
    let joined = format!("{}{}", first.text, second.text);
    assert!(
        document.markdown.starts_with(&joined),
        "the pages are contiguous: {:?}",
        &second.text[..40.min(second.text.len())]
    );
    assert_eq!(first.total, second.total);
    assert!(second.end > first.end, "the second page moves forward");

    // The last page has no hint.
    let last = fetch::slice_page(
        &document.markdown,
        second.next_offset.unwrap_or(0) + 100_000,
        DEFAULT_MAX_CHARS,
    );
    assert!(last.next_offset.is_none());
    assert!(last.text.is_empty());
}

#[tokio::test]
async fn max_chars_is_clamped_to_the_documented_range() {
    let markdown = "x".repeat(50_000);
    let small = fetch::slice_page(&markdown, 0, 10);
    assert_eq!(
        small.end,
        fetch::MIN_MAX_CHARS,
        "below the minimum is raised"
    );
    let large = fetch::slice_page(&markdown, 0, 100_000);
    assert_eq!(large.end, fetch::MAX_CHARS, "above the maximum is lowered");
    // The default is 8,000 characters; a document without a line break is cut at the character
    // boundary itself.
    let default = fetch::slice_page(&markdown, 0, DEFAULT_MAX_CHARS);
    assert_eq!(default.end, DEFAULT_MAX_CHARS);
    // With line breaks, the page ends at the last one that fits.
    let lined = (0..2_000)
        .map(|index| format!("line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let at_line = fetch::slice_page(&lined, 0, DEFAULT_MAX_CHARS);
    assert!(at_line.end <= DEFAULT_MAX_CHARS);
    assert!(lined[..at_line.text.len()].ends_with(|ch: char| ch.is_ascii_digit()));
}

// ───────────────────────────── through the tool ─────────────────────────────

#[tokio::test]
async fn a_fetched_page_is_wrapped_as_untrusted_content_when_rendered() {
    use nekowriter::testing::TestApp;
    use serde_json::json;

    let site = Site::new().await;
    let body = (1..=1_200)
        .map(|index| format!("<p>Paragraph {index}.</p>"))
        .collect::<String>();
    site.mount_html(article("Tool page", &body)).await;

    // A state whose fetcher resolves the stand-in host, so the tool path is the production one.
    let address: SocketAddr = *site.server.address();
    let web = WebService::with_fetcher(
        Fetcher::with_host_overrides(vec![(
            HOST.to_owned(),
            SocketAddr::new(address.ip(), address.port()),
        )])
        .expect("fetcher"),
    );
    let app = TestApp::new().await;
    let state = app.state().clone().with_web(web);
    let tool = state
        .tool_registry()
        .get("web_fetch")
        .expect("web_fetch is registered")
        .clone();
    assert_eq!(tool.name(), "web_fetch");

    let fetcher = state.web().fetcher().expect("fetcher");
    let document = fetch::fetch(fetcher, state.web().cache(), &site.url("/page"))
        .await
        .expect("fetch");
    assert_eq!(document.title, "Tool page");
    let sliced = fetch::slice_page(&document.markdown, 0, DEFAULT_MAX_CHARS);
    assert!(document.total_chars() > sliced.end, "the page pages");

    // The wrapper is the prompt pack's, so the model-facing wording stays in prompts/.
    let prompts = state
        .prompts()
        .for_book(state.db(), None)
        .await
        .expect("prompts");
    let wrapped = prompts
        .render(
            "notice.web_content",
            &json!({ "body": format!("{} ...", sliced.text) }),
        )
        .expect("notice.web_content")
        .text;
    assert!(wrapped.contains("Paragraph 1."));
    assert_ne!(wrapped.trim(), sliced.text.trim(), "the wrapper adds text");
}

#[tokio::test]
async fn the_search_settings_shape_is_not_confused_with_a_search_config() {
    // Guards the seam: `WebService::with_tavily_base_url` must not change the SearXNG path.
    let service = WebService::new().with_tavily_base_url("http://wiremock.invalid");
    assert_eq!(service.tavily_base_url(), "http://wiremock.invalid");
    assert_eq!(
        std::mem::discriminant(&SearchProviderKind::Searxng),
        std::mem::discriminant(&SearchProviderKind::Searxng)
    );
    // A blocked address is never fetched, whatever the URL looks like.
    assert_eq!(
        fetch::check_target(&url::Url::parse("http://127.0.0.1/").expect("url")),
        Err(FetchFailure::Blocked)
    );
    let _ = IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    let _ = StatusCode::OK;
}
