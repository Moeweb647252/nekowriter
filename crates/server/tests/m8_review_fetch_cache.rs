//! M8 review, finding F2: a page reached through a redirect must be served from the page cache
//! (spec/07 §7 step 7).
//!
//! The converted Markdown is cached "so `offset` calls do not refetch"; the cache used to be
//! written under the post-redirect `document.url` while every lookup used the requested URL, so a
//! redirected page was refetched (and its redirect chain re-followed) on every page read.

use nekowriter::web::fetch::{self, Fetcher, PageCache};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn redirected_site(target_requests: u64) -> (MockServer, Fetcher, String) {
    let server = MockServer::start().await;
    let address = *server.address();
    let fetcher = Fetcher::with_host_overrides(vec![(
        "public.example".to_owned(),
        std::net::SocketAddr::new(address.ip(), address.port()),
    )])
    .expect("fetcher");

    Mock::given(method("GET"))
        .and(path("/hop"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/target"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/target"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<html><head><title>T</title></head><body><article><p>once only</p></article></body></html>",
            "text/html; charset=utf-8",
        ))
        .expect(target_requests)
        .mount(&server)
        .await;

    let url = format!("http://public.example:{}/hop", address.port());
    (server, fetcher, url)
}

#[tokio::test]
async fn a_page_reached_through_a_redirect_is_served_from_the_cache_on_the_next_read() {
    let (server, fetcher, url) = redirected_site(1).await;
    let cache = PageCache::new();

    let first = fetch::fetch(&fetcher, &cache, &url)
        .await
        .expect("first fetch follows the redirect");
    assert!(!first.cached);
    assert!(first.url.contains("/target"), "{}", first.url);
    assert!(first.markdown.contains("once only"));

    // The same requested URL is served from the cache; the mocks' `expect(1)` fail the test when
    // the page is refetched (spec/07 §7 step 7).
    let second = fetch::fetch(&fetcher, &cache, &url)
        .await
        .expect("second fetch");
    assert!(
        second.cached,
        "the redirected page was refetched: {second:?}"
    );
    assert_eq!(second.markdown, first.markdown);
    assert_eq!(second.url, first.url, "the final URL is still reported");
    assert_eq!(
        server.received_requests().await.expect("requests").len(),
        2,
        "one request per hop: the page was not refetched"
    );
}

/// The converse: a requested URL that redirects is cached under its own key, and asking for the
/// final URL directly is a separate entry that is fetched once. Slicing (`offset`/`max_chars`)
/// always applies to the extracted Markdown, so the two entries are interchangeable in content.
#[tokio::test]
async fn the_final_url_can_be_fetched_directly_as_its_own_cache_entry() {
    let (server, fetcher, url) = redirected_site(2).await;
    let cache = PageCache::new();
    let first = fetch::fetch(&fetcher, &cache, &url)
        .await
        .expect("redirected fetch");
    assert!(!first.cached);

    let direct = first.url.clone();
    let second = fetch::fetch(&fetcher, &cache, &direct)
        .await
        .expect("direct fetch");
    assert!(!second.cached, "the final URL has its own entry");
    assert_eq!(second.markdown, first.markdown);
    let third = fetch::fetch(&fetcher, &cache, &direct)
        .await
        .expect("direct fetch again");
    assert!(third.cached, "the direct entry is now cached");

    // The target mock's `expect(1)` counted the redirect hop's request only; the direct fetches
    // are a second and third request, so the counter of /target is 1 + 1 (the third is cached).
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 3, "redirect hop, direct, cached direct");
}
