//! `web_fetch` (spec/07 §7): the SSRF-guarded page fetcher and its HTML → Markdown pipeline.
//!
//! The client is built once (a `reqwest::Client` is a connection pool) with four hard properties:
//!
//! 1. a custom DNS resolver that refuses loopback, private, link-local, CGNAT, multicast,
//!    broadcast, documentation and unspecified addresses (spec/01 §10.4);
//! 2. a redirect policy that re-validates every hop: at most 5 redirects, http(s) only, and no
//!    literal private address;
//! 3. `no_proxy()`: a system proxy would resolve hosts itself and bypass the resolver (D-045);
//! 4. a 20 s timeout and a 5 MB streamed body cap.
//!
//! Extraction is `dom_smoothie` Readability over the decoded HTML, falling back to `<body>`, then
//! `htmd` HTML → Markdown. The converted page is cached (LRU 32, 30 minutes) so the `offset`
//! paging of step 7 never refetches.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

use futures::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use super::{WEB_TIMEOUT, check_url};

/// The body cap of spec/07 §7 step 3 (5 MB).
pub const BODY_CAP: usize = 5 * 1024 * 1024;

/// The redirect cap of spec/07 §7 step 2.
pub const MAX_REDIRECTS: usize = 5;

/// The page cache size and lifetime of spec/07 §7 step 7.
pub const CACHE_PAGES: usize = 32;
pub const CACHE_TTL: Duration = Duration::from_secs(30 * 60);

/// The default and maximum `max_chars` of the tool (spec/05 §7).
pub const DEFAULT_MAX_CHARS: usize = 8_000;
pub const MAX_CHARS: usize = 20_000;
/// The tool schema's minimum, mirrored here so a slice is never degenerate.
pub const MIN_MAX_CHARS: usize = 1_000;

/// Why a `web_fetch` call failed (spec/07 §7 step 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchFailure {
    /// Step 1: not http(s), too long, or carrying credentials.
    InvalidUrl,
    /// Step 2: the address is loopback or private, or a redirect target is not http(s).
    Blocked,
    TooManyRedirects,
    Timeout,
    TooLarge,
    /// An HTTP status ≥ 400; the code travels with the failure.
    Status,
    /// Anything else.
    Network,
}

impl FetchFailure {
    /// The `reason` code passed to the model-facing template.
    pub fn as_str(self) -> &'static str {
        match self {
            FetchFailure::InvalidUrl => "invalid_url",
            FetchFailure::Blocked => "blocked",
            FetchFailure::TooManyRedirects => "too_many_redirects",
            FetchFailure::Timeout => "timeout",
            FetchFailure::TooLarge => "too_large",
            FetchFailure::Status => "status",
            FetchFailure::Network => "network",
        }
    }
}

/// One fetched page after extraction (spec/07 §7 step 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedDocument {
    /// The URL that was actually read (after redirects).
    pub url: String,
    pub title: String,
    /// The converted Markdown of the whole page, before paging.
    pub markdown: String,
    /// True when the document came from the cache (no request was made).
    pub cached: bool,
}

impl FetchedDocument {
    pub fn total_chars(&self) -> usize {
        self.markdown.chars().count()
    }
}

/// A redirect target the guard refuses (spec/07 §7 step 2), carried inside the request error.
#[derive(Debug)]
struct BlockedRedirect(String);

impl fmt::Display for BlockedRedirect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "blocked redirect target {}", self.0)
    }
}

impl std::error::Error for BlockedRedirect {}

/// More redirects than [`MAX_REDIRECTS`] (spec/07 §7 step 2).
#[derive(Debug)]
struct TooManyRedirects;

impl fmt::Display for TooManyRedirects {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("too many redirects")
    }
}

impl std::error::Error for TooManyRedirects {}

/// The marker the DNS resolver returns for a refused address (spec/07 §7 step 2).
#[derive(Debug)]
struct BlockedAddress(String);

impl fmt::Display for BlockedAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "blocked address for host {}", self.0)
    }
}

impl std::error::Error for BlockedAddress {}

// ───────────────────────────── the address policy ─────────────────────────────

/// The IPv4 ranges that may never be fetched (spec/01 §10.4, spec/07 §7 step 2).
const BLOCKED_V4: [(Ipv4Addr, u8); 14] = [
    (Ipv4Addr::new(0, 0, 0, 0), 8),       // "this network" / unspecified
    (Ipv4Addr::new(10, 0, 0, 0), 8),      // RFC 1918
    (Ipv4Addr::new(100, 64, 0, 0), 10),   // CGNAT 100.64.0.0/10
    (Ipv4Addr::new(127, 0, 0, 0), 8),     // loopback
    (Ipv4Addr::new(169, 254, 0, 0), 16),  // link-local
    (Ipv4Addr::new(172, 16, 0, 0), 12),   // RFC 1918
    (Ipv4Addr::new(192, 0, 0, 0), 24),    // IETF protocol assignments
    (Ipv4Addr::new(192, 0, 2, 0), 24),    // TEST-NET-1 (documentation)
    (Ipv4Addr::new(192, 168, 0, 0), 16),  // RFC 1918
    (Ipv4Addr::new(198, 18, 0, 0), 15),   // benchmarking
    (Ipv4Addr::new(198, 51, 100, 0), 24), // TEST-NET-2 (documentation)
    (Ipv4Addr::new(203, 0, 113, 0), 24),  // TEST-NET-3 (documentation)
    (Ipv4Addr::new(224, 0, 0, 0), 4),     // multicast
    (Ipv4Addr::new(240, 0, 0, 0), 4),     // reserved, includes broadcast
];

/// True when the address must never be fetched (spec/07 §7 step 2).
pub fn is_blocked_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(address: Ipv4Addr) -> bool {
    if address == Ipv4Addr::BROADCAST {
        return true;
    }
    BLOCKED_V4
        .iter()
        .any(|(base, prefix)| in_v4_net(address, *base, *prefix))
}

fn is_blocked_v6(address: Ipv6Addr) -> bool {
    // An IPv4-mapped or IPv4-compatible address is judged by its IPv4 meaning, so
    // `::ffff:127.0.0.1` cannot slip past the loopback rule.
    if let Some(v4) = mapped_v4(address) {
        return is_blocked_v4(v4);
    }
    let segments = address.segments();
    let unspecified = address == Ipv6Addr::UNSPECIFIED;
    let loopback = address == Ipv6Addr::LOCALHOST;
    // fc00::/7 unique local, fe80::/10 link-local, ff00::/8 multicast,
    // 2001:db8::/32 documentation, 100::/64 discard, 64:ff9b::/96 NAT64.
    let unique_local = segments[0] & 0xfe00 == 0xfc00;
    let link_local = segments[0] & 0xffc0 == 0xfe80;
    let multicast = segments[0] & 0xff00 == 0xff00;
    let documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
    let discard = segments[0] == 0x0100 && segments[1..4] == [0, 0, 0];
    let nat64 = segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0, 0, 0, 0];
    unspecified
        || loopback
        || unique_local
        || link_local
        || multicast
        || documentation
        || discard
        || nat64
}

/// The IPv4 address an IPv6 address embeds, when it embeds one.
fn mapped_v4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = address.segments();
    // ::ffff:a.b.c.d (IPv4-mapped) and ::a.b.c.d (IPv4-compatible, deprecated).
    let compatible = segments[..6].iter().all(|segment| *segment == 0);
    if compatible {
        return Some(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ));
    }
    let mapped_prefix = segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff;
    if mapped_prefix {
        return Some(Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        ));
    }
    None
}

fn in_v4_net(address: Ipv4Addr, base: Ipv4Addr, prefix: u8) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    u32::from(address) & mask == u32::from(base) & mask
}

/// True when the URL itself may be fetched: http(s), and for a literal IP host, a public address
/// (spec/07 §7 steps 1–2). DNS names are left to [`GuardedResolver`].
pub fn check_target(url: &url::Url) -> Result<(), FetchFailure> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(FetchFailure::Blocked);
    }
    if let Some(host) = url.host() {
        match host {
            url::Host::Ipv4(v4) if is_blocked_v4(v4) => return Err(FetchFailure::Blocked),
            url::Host::Ipv6(v6) if is_blocked_v6(v6) => return Err(FetchFailure::Blocked),
            _ => {}
        }
    }
    Ok(())
}

// ───────────────────────────── resolver and redirects ─────────────────────────────

/// The DNS resolver of the fetch client (spec/07 §7 step 2).
///
/// It resolves the name and refuses the connection when **any** resolved address is blocked, so a
/// host that resolves to both a public and a private address cannot be used to reach the private
/// one.
struct GuardedResolver;

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| Box::new(error) as BoxError)?
                .collect();
            if addresses.is_empty() {
                return Err(Box::new(NoAddresses(host)) as BoxError);
            }
            if let Some(blocked) = addresses.iter().find(|addr| is_blocked_ip(addr.ip())) {
                return Err(
                    Box::new(BlockedAddress(format!("{host} -> {}", blocked.ip()))) as BoxError,
                );
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[derive(Debug)]
struct NoAddresses(String);

impl fmt::Display for NoAddresses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no addresses for host {}", self.0)
    }
}

impl std::error::Error for NoAddresses {}

/// The redirect policy of the fetch client (spec/07 §7 step 2): at most [`MAX_REDIRECTS`], and
/// every target re-validated for scheme and literal addresses.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error(TooManyRedirects);
        }
        let next = attempt.url().clone();
        if check_target(&next).is_err() {
            return attempt.error(BlockedRedirect(next.to_string()));
        }
        attempt.follow()
    })
}

/// Walks an error's source chain looking for a marker type.
fn chain_has<T: std::error::Error + 'static>(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(item) = current {
        if item.is::<T>() {
            return true;
        }
        current = item.source();
    }
    false
}

/// The same walk from a `reqwest::Error`.
fn chain_has_in<T: std::error::Error + 'static>(error: &reqwest::Error) -> bool {
    chain_has::<T>(error)
}

/// Classifies a transport failure (spec/07 §7 step 9).
fn classify(error: &reqwest::Error) -> FetchFailure {
    if chain_has::<BlockedRedirect>(error) || chain_has::<BlockedAddress>(error) {
        return FetchFailure::Blocked;
    }
    if chain_has_in::<TooManyRedirects>(error) {
        return FetchFailure::TooManyRedirects;
    }
    if error.is_timeout() {
        return FetchFailure::Timeout;
    }
    FetchFailure::Network
}

// ───────────────────────────── the fetcher ─────────────────────────────

/// The guarded page fetcher (spec/07 §7).
pub struct Fetcher {
    http: reqwest::Client,
}

impl Fetcher {
    /// The production client. It has **no** host override: every name is resolved by
    /// [`GuardedResolver`] and every answer is checked against the blocked ranges.
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::build(Vec::new())
    }

    /// A client that resolves the listed names to fixed addresses instead of using DNS.
    ///
    /// **Test-only, and it must stay test-only.** `wiremock` (like every test double) listens on
    /// loopback, which the guard correctly refuses, so the positive `web_fetch` paths would be
    /// untestable without a way to stand a local server in for a public host. The override replaces
    /// the DNS answer for *those exact names only*: the guard, the redirect policy, the timeout, the
    /// body cap and the extractor are the production ones, and any other host — including a literal
    /// private IP — still goes through [`GuardedResolver`] / [`check_target`] and is refused.
    ///
    /// **Production code must never call this.** It is compiled only for tests and the `testing`
    /// feature, and only `crates/server/tests/` uses it (see the `web_fetch_guard` tests, which also
    /// assert that a non-overridden name and a literal private IP are still refused).
    #[cfg(any(test, feature = "testing"))]
    pub fn with_host_overrides(
        overrides: Vec<(String, SocketAddr)>,
    ) -> Result<Self, reqwest::Error> {
        Self::build(overrides)
    }

    fn build(overrides: Vec<(String, SocketAddr)>) -> Result<Self, reqwest::Error> {
        let mut builder = reqwest::Client::builder()
            .timeout(WEB_TIMEOUT)
            // D-045: a proxy resolves hosts itself and would bypass the guard.
            .no_proxy()
            .user_agent(super::user_agent())
            .redirect(redirect_policy())
            .dns_resolver(std::sync::Arc::new(GuardedResolver));
        for (host, address) in &overrides {
            // `resolve_to_addrs` is applied on top of the resolver, so an override replaces only
            // the lookup of that exact name; every other name reaches `GuardedResolver`.
            builder = builder.resolve_to_addrs(host, &[*address]);
        }
        Ok(Self {
            http: builder.build()?,
        })
    }

    /// Fetches and converts one page (spec/07 §7 steps 1–6).
    pub async fn fetch(&self, raw_url: &str) -> Result<FetchedDocument, FetchError> {
        let url = check_url(raw_url).map_err(|reason| FetchError::failed(reason, raw_url))?;
        check_target(&url).map_err(|reason| FetchError::failed(reason, raw_url))?;

        let response = self
            .http
            .get(url.clone())
            .header(
                reqwest::header::ACCEPT,
                "text/html,text/plain;q=0.9,*/*;q=0.1",
            )
            .send()
            .await
            .map_err(|error| FetchError::failed(classify(&error), raw_url))?;

        let status = response.status();
        if status.as_u16() >= 400 {
            return Err(FetchError::Failed {
                reason: FetchFailure::Status,
                status: Some(status.as_u16()),
                url: raw_url.to_owned(),
            });
        }
        // Every followable redirect hop was re-validated by the policy (spec/07 §7 step 2). A 3xx
        // that reaches this point was *not* followed: reqwest only hands back a redirect response
        // when it cannot represent the Location as an HTTP URI at all (`file:`, `mailto:`,
        // `javascript:`, …) or when the header is missing — the policy never sees those. Returning
        // it as a page would tell the model the page was empty, so report a failure instead. A
        // non-http target is a refused redirect (step 2); a header-less 3xx is a malformed
        // response, which step 9 classes as `network`.
        if status.is_redirection() {
            let has_location = response.headers().contains_key(reqwest::header::LOCATION);
            return Err(FetchError::Failed {
                reason: if has_location {
                    FetchFailure::Blocked
                } else {
                    FetchFailure::Network
                },
                status: None,
                url: raw_url.to_owned(),
            });
        }
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let kind = classify_content_type(content_type.as_deref());
        if let ContentKind::Unsupported { content_type } = &kind {
            return Err(FetchError::Unsupported {
                content_type: content_type.clone(),
                url: raw_url.to_owned(),
            });
        }
        let bytes = self.read_body(response, raw_url).await?;
        let charset = content_type
            .as_deref()
            .and_then(charset_of_content_type)
            .map(str::to_owned);
        let decoded = decode_body(&bytes, charset.as_deref());
        let (title, markdown) = extract(&decoded, kind, &final_url);
        Ok(FetchedDocument {
            url: final_url,
            title,
            markdown,
            cached: false,
        })
    }

    /// Reads the body up to [`BODY_CAP`] and aborts beyond it (spec/07 §7 step 3).
    ///
    /// The cap is applied to the **stream**, chunk by chunk, before a chunk is retained: the body is
    /// never buffered in full and then measured, which would make a 5 MB "cap" no cap at all
    /// against a server that sends more. The first chunk that would cross the limit ends the read
    /// and drops the connection, so at most [`BODY_CAP`] bytes are ever held in memory.
    async fn read_body(
        &self,
        response: reqwest::Response,
        raw_url: &str,
    ) -> Result<Vec<u8>, FetchError> {
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| FetchError::failed(classify(&error), raw_url))?;
            check_cap(body.len(), chunk.len())
                .map_err(|reason| FetchError::failed(reason, raw_url))?;
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

/// Decides whether a chunk may be retained (spec/07 §7 step 3).
///
/// Pure so the streaming invariant can be tested without a socket: `retained + incoming` must stay
/// within [`BODY_CAP`], so the buffer never grows past the cap.
fn check_cap(retained: usize, incoming: usize) -> Result<(), FetchFailure> {
    if retained.saturating_add(incoming) > BODY_CAP {
        return Err(FetchFailure::TooLarge);
    }
    Ok(())
}

/// A failed `web_fetch` (spec/07 §7 steps 4 and 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// `error.fetch_failed` with its reason code and, for a status, the HTTP status.
    Failed {
        reason: FetchFailure,
        status: Option<u16>,
        url: String,
    },
    /// `error.unsupported_content_type` (spec/07 §7 step 4).
    Unsupported { content_type: String, url: String },
}

impl FetchError {
    /// The `fetch_failed` reason, when the failure is one.
    pub fn reason(&self) -> Option<FetchFailure> {
        match self {
            FetchError::Failed { reason, .. } => Some(*reason),
            FetchError::Unsupported { .. } => None,
        }
    }

    /// A generic failure with no HTTP status.
    fn failed(reason: FetchFailure, url: &str) -> Self {
        FetchError::Failed {
            reason,
            status: None,
            url: url.to_owned(),
        }
    }
}

/// What kind of body the response carries (spec/07 §7 step 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentKind {
    Html,
    PlainText,
    Unsupported { content_type: String },
}

/// Reads the media type of a `Content-Type` header; a missing one is HTML.
pub fn classify_content_type(header: Option<&str>) -> ContentKind {
    let Some(header) = header else {
        return ContentKind::Html;
    };
    let essence = header
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/html" | "application/xhtml+xml" | "text/xhtml" | "" => ContentKind::Html,
        "text/plain" | "text/markdown" => ContentKind::PlainText,
        other => ContentKind::Unsupported {
            content_type: other.to_owned(),
        },
    }
}

/// The `charset=` parameter of a `Content-Type` header (spec/07 §7 step 5).
pub fn charset_of_content_type(header: &str) -> Option<&str> {
    header.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        if name.trim().eq_ignore_ascii_case("charset") {
            Some(value.trim().trim_matches('"'))
        } else {
            None
        }
    })
}

/// Decodes a body with the charset of spec/07 §7 step 5: the header, then a BOM, then a `<meta>`
/// sniff of the first 4 KB, then UTF-8 with replacement.
pub fn decode_body(bytes: &[u8], header_charset: Option<&str>) -> String {
    // 1. The `Content-Type` charset wins.
    if let Some(charset) = header_charset
        && let Some(encoding) = encoding_rs::Encoding::for_label(charset.trim().as_bytes())
    {
        return decode_with(encoding, bytes);
    }
    // 2. A BOM is authoritative.
    if let Some((encoding, bom_length)) = encoding_rs::Encoding::for_bom(bytes) {
        return decode_with(encoding, &bytes[bom_length..]);
    }
    // 3. `<meta charset>` / `<meta http-equiv="content-type">` in the first 4 KB.
    if let Some(encoding) = sniff_meta_charset(&bytes[..bytes.len().min(4_096)]) {
        return decode_with(encoding, bytes);
    }
    // 4. UTF-8, lossily.
    decode_with(encoding_rs::UTF_8, bytes)
}

fn decode_with(encoding: &'static encoding_rs::Encoding, bytes: &[u8]) -> String {
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

/// Finds a declared charset in the head of the document (spec/07 §7 step 5).
fn sniff_meta_charset(head: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let text = String::from_utf8_lossy(head);
    let lowered = text.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(position) = lowered[search_from..].find("<meta") {
        let start = search_from + position;
        let end = lowered[start..]
            .find('>')
            .map(|offset| start + offset)
            .unwrap_or(lowered.len());
        let tag = &lowered[start..end];
        if let Some(charset) = extract_meta_charset(tag) {
            return encoding_rs::Encoding::for_label(charset.as_bytes());
        }
        search_from = end.max(start + 1);
        if search_from >= lowered.len() {
            break;
        }
    }
    None
}

/// The charset a single `<meta>` tag declares, if any.
fn extract_meta_charset(tag: &str) -> Option<String> {
    for attribute in tag.split_whitespace() {
        let attribute = attribute.trim_matches(|ch| ch == '"' || ch == '\'' || ch == '/');
        if let Some(value) = attribute.strip_prefix("charset=") {
            let value = value.trim_matches(|ch| ch == '"' || ch == '\'' || ch == ';');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

// ───────────────────────────── extraction ─────────────────────────────

/// Extracts the title and the Markdown of a decoded body (spec/07 §7 step 6).
pub fn extract(html: &str, kind: ContentKind, url: &str) -> (String, String) {
    match kind {
        // Step 4 says `text/plain` and `text/markdown` pass through **as is**, so the whitespace
        // rules of the HTML→Markdown pipeline (step 6) must not touch them: a Markdown hard break
        // is two trailing spaces, and a fenced block may legitimately contain blank runs.
        ContentKind::PlainText => (String::new(), html.to_owned()),
        ContentKind::Unsupported { .. } => (String::new(), String::new()),
        ContentKind::Html => {
            let (title, content_html) = readable_article(html, url);
            let markdown = html_to_markdown(&content_html);
            let title = if title.trim().is_empty() {
                document_title(html).unwrap_or_default()
            } else {
                title
            };
            (title.trim().to_owned(), markdown)
        }
    }
}

/// Runs Readability; on failure the whole body is used (spec/07 §7 step 6).
fn readable_article(html: &str, url: &str) -> (String, String) {
    match dom_smoothie::Readability::new(html, Some(url), None) {
        Ok(mut readability) => match readability.parse() {
            Ok(article) => {
                let content = article.content.to_string();
                if content.trim().is_empty() {
                    (article.title, body_html(html))
                } else {
                    (article.title, content)
                }
            }
            // A page Readability cannot parse still has usable text: convert its body.
            Err(_) => (String::new(), body_html(html)),
        },
        Err(_) => (String::new(), body_html(html)),
    }
}

/// The `<body>` of a document, without the head (spec/07 §7 step 6).
///
/// The DOM is already parsed by `htmd` (html5ever), so this only trims the document shell. It
/// uses a case-insensitive search rather than a second parse tree.
pub fn body_html(html: &str) -> String {
    let lowered = html.to_ascii_lowercase();
    let start = match lowered.find("<body") {
        Some(position) => match lowered[position..].find('>') {
            Some(offset) => position + offset + 1,
            None => return html.to_owned(),
        },
        None => return html.to_owned(),
    };
    let end = lowered[start..]
        .rfind("</body>")
        .map(|offset| start + offset)
        .unwrap_or(html.len());
    let body = &html[start..end];
    if body.trim().is_empty() {
        html.to_owned()
    } else {
        body.to_owned()
    }
}

/// HTML → Markdown with the tags of spec/07 §7 step 6 skipped.
pub fn html_to_markdown(html: &str) -> String {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "nav", "footer", "form", "iframe"])
        .build();
    match converter.convert(html) {
        Ok(markdown) => collapse_blank_lines(&markdown),
        Err(_) => collapse_blank_lines(html),
    }
}

/// Collapses 3+ blank lines into one blank line and trims trailing spaces (spec/07 §7 step 6).
pub fn collapse_blank_lines(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut blank_run = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            lines.push(String::new());
        } else {
            blank_run = 0;
            lines.push(trimmed.to_owned());
        }
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// The `<title>` of a document, decoded (a fallback for a page Readability cannot title).
fn document_title(html: &str) -> Option<String> {
    let lowered = html.to_ascii_lowercase();
    let start = lowered.find("<title")?;
    let after_tag = start + lowered[start..].find('>')? + 1;
    let end = lowered[after_tag..].find("</title>")? + after_tag;
    let title = html[after_tag..end].trim();
    if title.is_empty() {
        None
    } else {
        Some(html_unescape(title))
    }
}

/// Decodes the handful of entities a title can carry.
fn html_unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

// ───────────────────────────── paging ─────────────────────────────

/// One page of a converted document (spec/07 §7 step 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSlice {
    pub text: String,
    pub offset: usize,
    pub end: usize,
    pub total: usize,
    pub next_offset: Option<usize>,
}

/// Slices `markdown` at `[offset, offset + max_chars)`, cutting at a line boundary.
pub fn slice_page(markdown: &str, offset: usize, max_chars: usize) -> PageSlice {
    let chars: Vec<char> = markdown.chars().collect();
    let total = chars.len();
    let max_chars = max_chars.clamp(MIN_MAX_CHARS, MAX_CHARS);
    let start = offset.min(total);
    let mut end = (start + max_chars).min(total);
    if end < total {
        // Prefer a paragraph break, then a line break, so a page never ends mid-sentence.
        let window: String = chars[start..end].iter().collect();
        if let Some(position) = window.rfind('\n') {
            let candidate = start + window[..position].chars().count();
            if candidate > start {
                end = candidate;
            }
        }
    }
    let text: String = chars[start..end].iter().collect();
    let next_offset = if end < total { Some(end) } else { None };
    PageSlice {
        text,
        offset: start,
        end,
        total,
        next_offset,
    }
}

// ───────────────────────────── the page cache ─────────────────────────────

/// The per-process converted-page cache (spec/07 §7 step 7: LRU 32 pages, 30 minutes).
///
/// The spec calls it per session; one cache per process is a superset (a page fetched by one
/// session is reusable by another), which only ever saves a request.
#[derive(Default)]
pub struct PageCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    /// Access order, oldest first; the last element was used most recently.
    order: Vec<String>,
}

struct CacheEntry {
    document: FetchedDocument,
    stored: Instant,
}

impl PageCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a cached page, refreshing its LRU position; an expired entry is dropped.
    pub fn get(&self, url: &str) -> Option<FetchedDocument> {
        let mut inner = self.lock();
        let expired = inner
            .entries
            .get(url)
            .is_some_and(|entry| entry.stored.elapsed() > CACHE_TTL);
        if expired {
            inner.entries.remove(url);
            inner.order.retain(|item| item != url);
            return None;
        }
        let document = inner.entries.get(url)?.document.clone();
        inner.order.retain(|item| item != url);
        inner.order.push(url.to_owned());
        Some(FetchedDocument {
            cached: true,
            ..document
        })
    }

    /// Stores a page under the URL the caller **requested**, evicting the least recently used
    /// entry beyond [`CACHE_PAGES`].
    ///
    /// The key is the requested URL, not [`FetchedDocument::url`]: the latter is the final URL
    /// after redirects, and keying on it would make a redirected page unreachable from the URL the
    /// tool will ask for again, so every `offset` read would refetch and re-follow the redirect
    /// chain (spec/07 §7 step 7). Two requested URLs that redirect to the same final URL get one
    /// entry each, which is safe: a page slice is always computed from the extracted Markdown and
    /// never depends on how the document was reached.
    pub fn put(&self, requested_url: &str, document: FetchedDocument) {
        let mut inner = self.lock();
        let url = requested_url.to_owned();
        inner.entries.insert(
            url.clone(),
            CacheEntry {
                document,
                stored: Instant::now(),
            },
        );
        inner.order.retain(|item| item != &url);
        inner.order.push(url);
        while inner.order.len() > CACHE_PAGES {
            let evicted = inner.order.remove(0);
            inner.entries.remove(&evicted);
        }
    }

    /// The number of cached pages (tests assert eviction).
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl fmt::Debug for PageCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageCache")
            .field("len", &self.len())
            .finish()
    }
}

/// A page with its slice: what the tool needs (spec/05 §7 `web_fetch`).
pub async fn fetch(
    fetcher: &Fetcher,
    cache: &PageCache,
    url: &str,
) -> Result<FetchedDocument, FetchError> {
    if let Some(hit) = cache.get(url) {
        return Ok(hit);
    }
    let document = fetcher.fetch(url).await?;
    // Keyed by the requested URL so the next `offset` read of the same tool call is a cache hit
    // even when the page redirected (spec/07 §7 step 7).
    cache.put(url, document.clone());
    Ok(document)
}

/// Polls a resolver future to completion (used only by the resolver unit test).
#[allow(dead_code)]
fn poll_resolving(resolving: Resolving) -> Result<Vec<SocketAddr>, BoxError> {
    use std::future::Future;
    use std::pin::Pin;
    let mut future = resolving;
    let mut context = Context::from_waker(futures::task::noop_waker_ref());
    loop {
        match Pin::new(&mut future).poll(&mut context) {
            Poll::Ready(result) => {
                return result.map(|addrs| addrs.collect::<Vec<SocketAddr>>());
            }
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().expect("ip")
    }

    #[test]
    fn every_documented_address_class_is_blocked() {
        for address in [
            "127.0.0.1",        // loopback
            "127.255.255.254",  // loopback range
            "0.0.0.0",          // unspecified
            "10.1.2.3",         // RFC 1918
            "172.16.0.1",       // RFC 1918
            "172.31.255.255",   // RFC 1918 end
            "192.168.1.1",      // RFC 1918
            "169.254.169.254",  // link-local (cloud metadata)
            "100.64.0.1",       // CGNAT
            "100.127.255.255",  // CGNAT end
            "224.0.0.1",        // multicast
            "239.255.255.250",  // multicast
            "255.255.255.255",  // broadcast
            "192.0.2.10",       // TEST-NET-1
            "198.51.100.7",     // TEST-NET-2
            "203.0.113.9",      // TEST-NET-3
            "198.18.0.1",       // benchmarking
            "::1",              // IPv6 loopback
            "::",               // IPv6 unspecified
            "fe80::1",          // IPv6 link-local
            "fc00::1",          // IPv6 unique local
            "fd12:3456::1",     // IPv6 unique local
            "ff02::1",          // IPv6 multicast
            "2001:db8::1",      // IPv6 documentation
            "::ffff:127.0.0.1", // IPv4-mapped loopback
            "::ffff:10.0.0.1",  // IPv4-mapped private
        ] {
            assert!(is_blocked_ip(ip(address)), "{address} must be blocked");
        }
    }

    #[test]
    fn public_addresses_are_allowed() {
        for address in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2001:4860:4860::8888",
        ] {
            assert!(!is_blocked_ip(ip(address)), "{address} must be allowed");
        }
    }

    #[test]
    fn a_literal_private_ip_target_is_refused() {
        for raw in [
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://10.0.0.5/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.1.1/",
            "https://192.168.0.1/",
        ] {
            let url = url::Url::parse(raw).expect("url");
            assert_eq!(check_target(&url), Err(FetchFailure::Blocked), "{raw}");
        }
        // A public literal IP is fine.
        let url = url::Url::parse("http://93.184.216.34/").expect("url");
        assert_eq!(check_target(&url), Ok(()));
    }

    #[test]
    fn a_non_http_redirect_target_is_refused() {
        let url = url::Url::parse("ftp://example.com/x").expect("url");
        assert_eq!(check_target(&url), Err(FetchFailure::Blocked));
    }

    #[test]
    fn content_types_map_to_the_documented_pipeline() {
        assert_eq!(classify_content_type(None), ContentKind::Html);
        assert_eq!(
            classify_content_type(Some("text/html; charset=gbk")),
            ContentKind::Html
        );
        assert_eq!(
            classify_content_type(Some("application/xhtml+xml")),
            ContentKind::Html
        );
        assert_eq!(
            classify_content_type(Some("text/plain")),
            ContentKind::PlainText
        );
        assert_eq!(
            classify_content_type(Some("text/markdown; charset=utf-8")),
            ContentKind::PlainText
        );
        assert_eq!(
            classify_content_type(Some("application/pdf")),
            ContentKind::Unsupported {
                content_type: "application/pdf".to_owned()
            }
        );
    }

    #[test]
    fn a_page_ends_at_a_line_boundary_when_one_fits() {
        // The contract of spec/07 §7 step 7 is a cut "at a line boundary", not "on a digit": a
        // page ends just before a newline when the window contains one, and at the character
        // boundary only when it does not. Both cases are asserted here.
        let markdown = (1..=400)
            .map(|index| format!("line {index} of a long page"))
            .collect::<Vec<_>>()
            .join("\n");
        let slice = slice_page(&markdown, 0, DEFAULT_MAX_CHARS);
        assert!(markdown.starts_with(&slice.text));
        assert!(slice.next_offset.is_some(), "the page leaves a remainder");
        assert_eq!(
            markdown.chars().nth(slice.end),
            Some('\n'),
            "the cut lands just before a line break"
        );
        assert!(
            !slice.text.ends_with('\n'),
            "the boundary itself is not part of the page"
        );
        let next_offset = slice.next_offset.expect("a remainder");
        assert_eq!(next_offset, slice.end);
        let next = slice_page(&markdown, next_offset, DEFAULT_MAX_CHARS);
        assert_eq!(next.offset, next_offset);
        assert_eq!(next.total, slice.total);
        assert!(next.end > slice.end, "the next page moves forward");

        // Without any line break there is no boundary to prefer, so the character cut is used.
        let solid = "x".repeat(DEFAULT_MAX_CHARS * 3);
        let cut = slice_page(&solid, 0, DEFAULT_MAX_CHARS);
        assert_eq!(cut.end, DEFAULT_MAX_CHARS);
        assert_eq!(cut.text, "x".repeat(DEFAULT_MAX_CHARS));

        // A short page fits in one slice and reports no continuation hint.
        let tiny = slice_page("just one line", 0, DEFAULT_MAX_CHARS);
        assert!(tiny.next_offset.is_none());
        assert_eq!(tiny.text, "just one line");
        assert_eq!(tiny.offset, 0);
        assert_eq!(tiny.end, tiny.total);
    }

    #[test]
    fn blank_lines_collapse_and_trailing_spaces_go() {
        let text = "a  \n\n\n\nb\n\n\n";
        assert_eq!(collapse_blank_lines(text), "a\n\nb");
    }

    #[test]
    fn the_body_cap_is_enforced_per_streamed_chunk() {
        // The cap is checked before a chunk is retained, so a server that streams more than the cap
        // is cut off mid-stream: the retained size never exceeds the limit.
        let chunk = 64 * 1024;
        let mut retained = 0usize;
        let mut chunks = 0usize;
        while check_cap(retained, chunk).is_ok() {
            retained += chunk;
            chunks += 1;
            assert!(chunks < BODY_CAP, "the cap must stop the read");
        }
        assert!(retained <= BODY_CAP, "the buffer never exceeds the cap");
        assert_eq!(retained, BODY_CAP / chunk * chunk);
        // Exactly at the cap is still allowed; one byte more is not.
        assert_eq!(check_cap(BODY_CAP, 0), Ok(()));
        assert_eq!(check_cap(BODY_CAP - 1, 1), Ok(()));
        assert_eq!(check_cap(BODY_CAP, 1), Err(FetchFailure::TooLarge));
        // A single chunk larger than the cap is refused without ever being retained.
        assert_eq!(check_cap(0, BODY_CAP + 1), Err(FetchFailure::TooLarge));
        // The arithmetic saturates instead of overflowing on a hostile length.
        assert_eq!(check_cap(usize::MAX, 1), Err(FetchFailure::TooLarge));
    }

    #[test]
    fn the_body_is_extracted_without_the_head() {
        let html = "<html><head><title>T</title></head><body><p>Hello</p></body></html>";
        assert_eq!(body_html(html), "<p>Hello</p>");
        assert_eq!(body_html("<p>fragment</p>"), "<p>fragment</p>");
        assert_eq!(document_title(html), Some("T".to_owned()));
    }

    #[test]
    fn a_meta_charset_is_sniffed_from_the_head() {
        let head = br#"<html><head><meta charset="gbk"><title>x</title></head>"#;
        assert_eq!(
            sniff_meta_charset(head).map(|encoding| encoding.name()),
            Some("GBK")
        );
        let http_equiv = br#"<meta http-equiv="Content-Type" content="text/html; charset=big5">"#;
        // The `http-equiv` form declares the charset *inside* another attribute's quoted value.
        // The whitespace split happens to separate `charset=big5` from the surrounding
        // `content="…;`, so the same scan finds it — asserted, not merely "some or none".
        assert_eq!(
            sniff_meta_charset(http_equiv).map(|encoding| encoding.name()),
            Some("Big5")
        );
    }

    #[test]
    fn the_cache_is_keyed_by_the_requested_url_not_the_final_one() {
        // A redirected page is stored under the URL the caller asked for, so the next read of
        // that URL is a hit even though the document reports its post-redirect URL (spec/07 §7
        // step 7).
        let cache = PageCache::new();
        let requested = "https://example.com/old";
        let final_url = "https://example.com/new";
        cache.put(
            requested,
            FetchedDocument {
                url: final_url.to_owned(),
                title: "T".to_owned(),
                markdown: "body".to_owned(),
                cached: false,
            },
        );
        let hit = cache.get(requested).expect("the requested URL is the key");
        assert!(hit.cached);
        assert_eq!(hit.url, final_url, "the document keeps its final URL");
        assert!(
            cache.get(final_url).is_none(),
            "the final URL was never asked for"
        );
    }

    #[test]
    fn the_cache_evicts_the_least_recently_used_page() {
        let cache = PageCache::new();
        for index in 0..CACHE_PAGES + 4 {
            let url = format!("https://example.com/{index}");
            cache.put(
                &url,
                FetchedDocument {
                    url: url.clone(),
                    title: String::new(),
                    markdown: "x".to_owned(),
                    cached: false,
                },
            );
        }
        assert_eq!(cache.len(), CACHE_PAGES);
        assert!(cache.get("https://example.com/0").is_none());
        assert!(cache.get("https://example.com/3").is_none());
        assert!(cache.get("https://example.com/4").is_some());
        // Reading a page counts as use, so it survives the next eviction.
        let _ = cache.get("https://example.com/4");
        cache.put(
            "https://example.com/new",
            FetchedDocument {
                url: "https://example.com/new".to_owned(),
                title: String::new(),
                markdown: "x".to_owned(),
                cached: false,
            },
        );
        assert!(cache.get("https://example.com/4").is_some());
        assert!(cache.get("https://example.com/5").is_none());
    }
}
