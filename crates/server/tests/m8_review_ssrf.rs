//! M8 review: adversarial probes of the `web_fetch` SSRF guard (spec/01 §10.4, spec/07 §7).
//!
//! These are the bypass shapes an attacker controls through a URL the model was told to read:
//! non-canonical IPv4 spellings, IPv4-in-IPv6 encodings, userinfo tricks, non-http schemes,
//! over-long URLs, and redirects into each of those. Every case must be refused **as a guard
//! refusal** (`blocked`/`invalid_url`) — not merely fail to connect, which would mean the guard
//! let the request reach the network layer.

use std::net::{IpAddr, SocketAddr};

use nekowriter::web::fetch::{self, FetchFailure, Fetcher, PageCache};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A fetcher whose only overridden name is `public.example`, pointing at a local wiremock server.
async fn site() -> (MockServer, Fetcher, u16) {
    let server = MockServer::start().await;
    let address = *server.address();
    let fetcher = Fetcher::with_host_overrides(vec![(
        "public.example".to_owned(),
        SocketAddr::new(address.ip(), address.port()),
    )])
    .expect("fetcher");
    (server, fetcher, address.port())
}

fn blocked(reason: Option<FetchFailure>) -> bool {
    matches!(
        reason,
        Some(FetchFailure::Blocked) | Some(FetchFailure::InvalidUrl)
    )
}

/// Non-canonical IPv4 spellings and IPv4-in-IPv6 encodings must be resolved to their real address
/// by the URL parser / resolver and refused — never treated as an unrelated domain name.
#[tokio::test]
async fn non_canonical_ip_literals_cannot_reach_loopback() {
    let (server, fetcher, _port) = site().await;
    let cases = [
        ("http://2130706433/", "decimal loopback"),
        ("http://0x7f000001/", "hex loopback"),
        ("http://0x7f.0x0.0x0.0x1/", "dotted hex loopback"),
        ("http://0177.0.0.1/", "octal loopback"),
        ("http://127.1/", "short loopback"),
        ("http://127.0.0.1./", "trailing-dot loopback"),
        ("http://[::ffff:127.0.0.1]/", "IPv4-mapped loopback"),
        (
            "http://[::ffff:7f00:1]/",
            "IPv4-mapped loopback in hex groups",
        ),
        ("http://[::127.0.0.1]/", "IPv4-compatible loopback"),
        ("http://[64:ff9b::127.0.0.1]/", "NAT64 loopback"),
        ("http://[64:ff9b::10.0.0.1]/", "NAT64 private"),
        ("http://[::ffff:169.254.169.254]/", "IPv4-mapped link-local"),
        ("http://0/", "zero address"),
        ("http://localhost/", "name resolving to loopback"),
        (
            "http://localhost:9/",
            "name resolving to loopback with a port",
        ),
    ];
    for (url, label) in cases {
        let error = fetcher
            .fetch(url)
            .await
            .expect_err(&format!("{label} must be refused: {url}"));
        assert!(
            blocked(error.reason()),
            "{label} ({url}) was not a guard refusal: {:?}",
            error.reason()
        );
    }
    // The server never saw a request: nothing was served to a private target.
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .is_empty(),
        "a probe reached the local server"
    );
}

/// Userinfo must not smuggle a private host past the check (`http://name@private/` has the host
/// after the `@`; `http://private@public/` carries credentials and is refused by step 1).
#[tokio::test]
async fn userinfo_tricks_are_invalid_urls() {
    let (_server, fetcher, _port) = site().await;
    for url in [
        "http://127.0.0.1@public.example/",
        "http://public.example@127.0.0.1/",
        "http://user:pass@127.0.0.1/",
        "http://user:pass@public.example/",
        "http://%31%32%37.0.0.1/",
    ] {
        let error = fetcher.fetch(url).await.expect_err(url);
        assert!(
            blocked(error.reason()),
            "{url} must be refused, was {:?}",
            error.reason()
        );
    }
}

/// A redirect is re-validated on every hop: literal private addresses in every spelling, and
/// non-http schemes, are refused before the target is requested (spec/07 §7 step 2).
#[tokio::test]
async fn every_redirect_hop_revalidates_literal_addresses_and_schemes() {
    let (server, fetcher, port) = site().await;
    let targets = [
        ("http://127.0.0.1:1/admin", "loopback"),
        ("http://2130706433:1/admin", "decimal loopback"),
        ("http://[::ffff:10.0.0.1]/admin", "mapped private"),
        (
            "http://169.254.169.254/latest/meta-data/",
            "link-local metadata",
        ),
        ("http://100.64.0.1/", "CGNAT"),
        ("//127.0.0.1/admin", "protocol-relative loopback"),
        ("file:///etc/passwd", "file scheme"),
        ("gopher://public.example/", "gopher scheme"),
        ("mailto:someone@example.com", "mailto scheme"),
        ("javascript:alert(1)", "javascript scheme"),
    ];
    for (index, (target, label)) in targets.iter().enumerate() {
        let from = format!("/from{index}");
        Mock::given(method("GET"))
            .and(path(from.clone()))
            .respond_with(ResponseTemplate::new(302).insert_header("location", *target))
            .mount(&server)
            .await;
        let url = format!("http://public.example:{port}{from}");
        let error = fetcher
            .fetch(&url)
            .await
            .expect_err(&format!("{label} must be refused: {target}"));
        assert!(
            blocked(error.reason()),
            "{label} redirect was not a guard refusal: {:?}",
            error.reason()
        );
    }
    // Only the local hop pages were requested; no target was ever contacted.
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), targets.len(), "one request per first hop");
    assert!(
        requests.iter().all(|request| {
            let path = request.url.path();
            path.starts_with("/from")
        }),
        "a redirect target was requested: {:?}",
        requests
            .iter()
            .map(|request| request.url.path())
            .collect::<Vec<_>>()
    );
}

/// The test-only host override (D-096) must not open any door by itself: it replaces one name's
/// lookup, while literal private addresses and other names still hit the production guard.
#[tokio::test]
async fn the_test_seam_cannot_widen_the_guard() {
    let (_server, fetcher, _port) = site().await;
    // Literal private addresses are refused before any lookup, whatever names are overridden.
    for url in [
        "http://127.0.0.1/",
        "http://[::1]/",
        "http://10.0.0.1/",
        "http://192.168.0.1/",
        "http://100.64.0.1/",
        "http://169.254.169.254/",
        "http://[fd00::1]/",
    ] {
        let error = fetch::fetch(&fetcher, &PageCache::new(), url)
            .await
            .expect_err(url);
        assert!(
            blocked(error.reason()),
            "{url} must stay blocked through the seam, was {:?}",
            error.reason()
        );
    }
    // A name that is not overridden goes through the production resolver: with no such name in
    // DNS it fails as a network error, and a name that does resolve to loopback is blocked. What
    // matters is that the override granted it nothing.
    for url in ["http://not-overridden.example/", "http://other.example/"] {
        let error = fetch::fetch(&fetcher, &PageCache::new(), url)
            .await
            .expect_err(url);
        assert!(
            matches!(
                error.reason(),
                Some(FetchFailure::Blocked) | Some(FetchFailure::Network)
            ),
            "{url} must not be served by the override, was {:?}",
            error.reason()
        );
    }
}

/// A 3xx response that reqwest could not follow must be a failure, not an empty page: the guard
/// never sees a Location that is not an HTTP URI (`file:`, `mailto:`, `javascript:`), and a
/// header-less 3xx is malformed. Returning either as a successful empty document would tell the
/// model the page had no content (spec/07 §7 steps 2 and 9).
#[tokio::test]
async fn an_unfollowable_redirect_is_a_failure_not_an_empty_page() {
    let (server, fetcher, port) = site().await;
    Mock::given(method("GET"))
        .and(path("/no-location"))
        .respond_with(ResponseTemplate::new(302))
        .mount(&server)
        .await;
    let error = fetcher
        .fetch(&format!("http://public.example:{port}/no-location"))
        .await
        .expect_err("a header-less redirect is not a page");
    assert_eq!(
        error.reason(),
        Some(FetchFailure::Network),
        "a malformed redirect is a network failure"
    );
}

/// Belt and braces on the address policy itself: every class the spec names is refused as an
/// `IpAddr`, including the IPv4-in-IPv6 forms.
#[test]
fn the_address_policy_covers_every_specified_class() {
    let blocked_ips = [
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.0.1",
        "169.254.1.1",
        "100.64.0.1",
        "224.0.0.1",
        "255.255.255.255",
        "0.0.0.0",
        "::1",
        "::",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "::ffff:169.254.169.254",
        "::ffff:0.0.0.0",
        "64:ff9b::127.0.0.1",
        "2001:db8::1",
    ];
    for text in blocked_ips {
        let address: IpAddr = text.parse().expect("ip");
        assert!(fetch::is_blocked_ip(address), "{text} must be blocked");
    }
    for text in [
        "8.8.8.8",
        "1.1.1.1",
        "93.184.216.34",
        "2001:4860:4860::8888",
    ] {
        let address: IpAddr = text.parse().expect("ip");
        assert!(!fetch::is_blocked_ip(address), "{text} must be allowed");
    }
}
