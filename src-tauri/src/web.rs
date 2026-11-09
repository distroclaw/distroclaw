//! Online search, run by the app rather than the model, so it works with any model. Search
//! providers without keys (DuckDuckGo, then Wikipedia's API), page fetching with guards,
//! readable-text extraction and, for ordinary pages that need JavaScript, a headless render in an
//! installed browser with a throwaway profile. Bot checks are respected: a provider that asks
//! for one is paused, and a page that shows one is never rendered. Nothing here runs unless
//! Online search is on.

use crate::process;
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
pub const RENDER_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_PAGE_BYTES: usize = 1_500_000;
const MAX_REDIRECTS: usize = 3;
/// Below this much readable text, a page that runs scripts is worth rendering.
pub const THIN_PAGE_CHARS: usize = 400;
const BROWSER_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
/// Wikipedia asks API clients to name themselves.
const APP_UA: &str = "DistroClaw/0.1 (desktop app; web search)";
/// How long a provider that answered with a bot check is left alone.
pub const PROVIDER_PAUSE: Duration = Duration::from_secs(15 * 60);

/// Where to search and what may be reached. Tests point the URLs at a local server.
#[derive(Clone, Debug)]
pub struct WebConfig {
    pub search_url: String,
    pub wiki_url: String,
    /// An installed Chromium-family browser for headless rendering, if any.
    pub browser: Option<PathBuf>,
    /// Only for tests: allow loopback and private addresses.
    pub allow_private: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_url: "https://html.duckduckgo.com/html/".into(),
            wiki_url: "https://en.wikipedia.org/w/api.php".into(),
            browser: find_browser(),
            allow_private: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A fetched page as readable text; `scripted` when it runs scripts (a candidate for rendering),
/// `blocked` when it shows a bot check or captcha (never rendered).
#[derive(Debug)]
pub struct Page {
    pub text: String,
    pub scripted: bool,
    pub blocked: bool,
}

/// Cheap to clone: clones share the connection pool and the provider pause.
#[derive(Clone)]
pub struct Web {
    pub cfg: WebConfig,
    http: reqwest::Client,
    /// DuckDuckGo is left alone until then after it asked for a bot check.
    paused_until: Arc<Mutex<Option<Instant>>>,
}

impl Web {
    pub fn new(cfg: WebConfig) -> Self {
        let allow = cfg.allow_private;
        let redirects = reqwest::redirect::Policy::custom(move |attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if !matches!(url.scheme(), "http" | "https") {
                attempt.stop()
            } else if !allow && literal_ip(url).is_some_and(|ip| !is_public(ip)) {
                attempt.error("redirect to a private address")
            } else {
                attempt.follow()
            }
        });
        // No cookie store: nothing is kept between requests. No proxy: a proxy would resolve host
        // names itself, past the public-address check below.
        let http = reqwest::Client::builder()
            .no_proxy()
            .user_agent(BROWSER_UA)
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(Duration::from_secs(4))
            .redirect(redirects)
            .dns_resolver(Arc::new(PublicOnly { allow_private: allow }))
            .build()
            .expect("the web client builds");
        Self { cfg, http, paused_until: Arc::new(Mutex::new(None)) }
    }

    /// Up to `max` results from the first provider that answers. A provider that asks for a bot
    /// check is paused for `PROVIDER_PAUSE`; it is never worked around.
    pub async fn search(&self, query: &str, max: usize) -> Result<Vec<Hit>, String> {
        let mut last = String::from("No search provider answered.");
        for provider in [Provider::DuckDuckGo, Provider::Wikipedia] {
            let paused = self.paused_until.lock().unwrap().is_some_and(|t| Instant::now() < t);
            if provider == Provider::DuckDuckGo && paused {
                continue;
            }
            match self.search_with(provider, query).await {
                Ok(hits) if !hits.is_empty() => return Ok(hits.into_iter().take(max).collect()),
                Ok(_) => last = format!("{provider:?}: no results"),
                Err(Failure::BotCheck) => {
                    *self.paused_until.lock().unwrap() = Some(Instant::now() + PROVIDER_PAUSE);
                    last = format!("{provider:?} asked for a bot check; paused");
                }
                Err(Failure::Other(e)) => last = format!("{provider:?}: {e}"),
            }
            log::info!("web search: {last}");
        }
        Err(last)
    }

    async fn search_with(&self, provider: Provider, query: &str) -> Result<Vec<Hit>, Failure> {
        let (url, ua) = match provider {
            Provider::DuckDuckGo => (&self.cfg.search_url, BROWSER_UA),
            Provider::Wikipedia => (&self.cfg.wiki_url, APP_UA),
        };
        let mut req = self.http.get(url).header(reqwest::header::USER_AGENT, ua);
        req = match provider {
            Provider::Wikipedia => req.query(&[
                ("action", "query"),
                ("list", "search"),
                ("srsearch", query),
                ("format", "json"),
                ("srlimit", "6"),
                ("utf8", "1"),
            ]),
            Provider::DuckDuckGo => req.query(&[("q", query)]),
        };
        let resp = req.send().await.map_err(|e| Failure::Other(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| Failure::Other(e.to_string()))?;
        let refused = status == reqwest::StatusCode::ACCEPTED || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        if refused || is_bot_check(&body) {
            return Err(Failure::BotCheck);
        }
        if status != reqwest::StatusCode::OK {
            return Err(Failure::Other(format!("status {}", status.as_u16())));
        }
        Ok(match provider {
            Provider::DuckDuckGo => parse_ddg(&body, "result__a", "result__snippet"),
            Provider::Wikipedia => parse_wikipedia(&body, &self.cfg.wiki_url),
        })
    }

    /// The page at `url` as readable text, with the size, type and address guards.
    pub async fn fetch(&self, url: &str) -> Result<Page, String> {
        let url = self.checked(url)?;
        let mut resp = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "text/html,text/plain;q=0.9")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status().as_u16()));
        }
        let kind = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let html = kind.contains("html");
        if !html && !kind.contains("text/plain") {
            return Err("not a web page".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() >= MAX_PAGE_BYTES {
                body.truncate(MAX_PAGE_BYTES);
                break;
            }
        }
        let raw = String::from_utf8_lossy(&body);
        Ok(if html {
            let lower = raw.to_ascii_lowercase();
            Page { text: html_to_text(&raw), scripted: lower.contains("<script"), blocked: is_bot_check(&lower) }
        } else {
            Page { text: collapse(&raw), scripted: false, blocked: false }
        })
    }

    /// Renders a page in the installed browser, headless, with a throwaway profile, and returns
    /// its text. `stop` ends the render early. The browser only reaches the page's own host, at
    /// the public address checked here (see `render_args`).
    pub async fn render(&self, url: &str, stop: impl Fn() -> bool + Send + 'static) -> Result<String, String> {
        let browser = self.cfg.browser.clone().ok_or("No browser for rendering is installed.")?;
        let url = self.checked(url)?;
        let host = url.host_str().ok_or("Not a valid web address.")?.to_string();
        // The address check above only sees literal IPs; resolve the host too.
        let ip = match literal_ip(&url) {
            Some(ip) => ip,
            None => {
                let port = url.port_or_known_default().unwrap_or(443);
                let addrs: Vec<IpAddr> = tokio::net::lookup_host((host.as_str(), port))
                    .await
                    .map_err(|e| e.to_string())?
                    .map(|a| a.ip())
                    .collect();
                if addrs.is_empty() || (!self.cfg.allow_private && addrs.iter().any(|ip| !is_public(*ip))) {
                    return Err(format!("{host} points to a private address."));
                }
                addrs.iter().copied().find(IpAddr::is_ipv4).unwrap_or(addrs[0])
            }
        };
        let task = tauri::async_runtime::spawn_blocking(move || {
            render_blocking(&browser, url.as_str(), &host, ip, RENDER_TIMEOUT, &stop)
        });
        let dom = task.await.map_err(|e| e.to_string())??;
        Ok(html_to_text(&dom))
    }

    fn checked(&self, raw: &str) -> Result<Url, String> {
        let url = Url::parse(raw).map_err(|_| "Not a valid web address.".to_string())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("Only http and https pages are read.".into());
        }
        if !self.cfg.allow_private && literal_ip(&url).is_some_and(|ip| !is_public(ip)) {
            return Err("Private addresses are not read.".into());
        }
        Ok(url)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Provider {
    DuckDuckGo,
    Wikipedia,
}

enum Failure {
    /// The provider wants to tell a person from a bot: respected, never worked around.
    BotCheck,
    Other(String),
}

/// A bot check, captcha or challenge page.
fn is_bot_check(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    ["anomaly-modal", "captcha", "cf-chl", "challenge-platform", "are you a robot", "unusual traffic"]
        .iter()
        .any(|m| lower.contains(m))
}

/// Resolves host names to public addresses only, so no page or redirect can reach this computer
/// or the local network.
struct PublicOnly {
    allow_private: bool,
}

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let allow = self.allow_private;
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.filter(|a| allow || is_public(a.ip())).collect();
            if addrs.is_empty() {
                return Err(format!("{host} points to a private address.").into());
            }
            Ok::<reqwest::dns::Addrs, Box<dyn std::error::Error + Send + Sync>>(Box::new(addrs.into_iter()))
        })
    }
}

/// The address when the host is written as one (or is `localhost`).
fn literal_ip(url: &Url) -> Option<IpAddr> {
    let host = url.host_str()?;
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return Some(IpAddr::from([127, 0, 0, 1]));
    }
    host.trim_start_matches('[').trim_end_matches(']').parse().ok()
}

/// Reachable on the public internet: not loopback, private, link-local, shared, multicast…
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return false;
            }
            let seg = v6.segments();
            let v4 = |hi: u16, lo: u16| IpAddr::V4(Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo)));
            // IPv4 written as IPv6 means that IPv4 address: mapped (::ffff:a.b.c.d), the old
            // compatible form (::a.b.c.d), NAT64 (64:ff9b::a.b.c.d) and 6to4 (2002:aabb:ccdd::).
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(mapped));
            }
            if seg[..6] == [0; 6] || seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_public(v4(seg[6], seg[7]));
            }
            if seg[0] == 0x2002 {
                return is_public(v4(seg[1], seg[2]));
            }
            !(v6.is_multicast()
                // Local-use NAT64 (64:ff9b:1::/48): the IPv4 address inside cannot be checked.
                || seg[..3] == [0x64, 0xff9b, 1]
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfe80
                // Site-local (deprecated) and documentation addresses.
                || (seg[0] & 0xffc0) == 0xfec0
                || seg[..2] == [0x2001, 0xdb8])
        }
    }
}

// ---------------------------------------------------------------- parsing

/// `(attributes, inner html)` of every element whose class list has `class`. Enough for the
/// simple, stable markup of the providers; no nesting of the same tag inside.
fn elements_with_class<'a>(html: &'a str, class: &str) -> Vec<(&'a str, &'a str)> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = lower[pos..].find('<') {
        let start = pos + rel;
        pos = start + 1;
        let Some(end_rel) = lower[start..].find('>') else { break };
        let tag = &html[start + 1..start + end_rel];
        let name: String = tag.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if name.is_empty() || !attr(tag, "class").is_some_and(|c| c.split_whitespace().any(|x| x == class)) {
            continue;
        }
        let inner_start = start + end_rel + 1;
        let close = format!("</{}", name.to_ascii_lowercase());
        let inner_end = lower[inner_start..].find(&close).map(|i| inner_start + i).unwrap_or(html.len());
        out.push((tag, &html[inner_start..inner_end]));
        pos = inner_end;
    }
    out
}

/// The value of `name="…"` or `name='…'` in a tag's attribute text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        let offset = tag.len() - value.len();
        let quote = value.chars().next()?;
        return if quote == '"' || quote == '\'' {
            let end = value[1..].find(quote)?;
            Some(&tag[offset + 1..offset + 1 + end])
        } else {
            let end = value.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(value.len());
            Some(&tag[offset..offset + end])
        };
    }
    None
}

/// DuckDuckGo's HTML results page: result links (through its redirect) and snippets, ads left out.
fn parse_ddg(html: &str, link_class: &str, snippet_class: &str) -> Vec<Hit> {
    let snippets: Vec<String> =
        elements_with_class(html, snippet_class).into_iter().map(|(_, inner)| inline_text(inner)).collect();
    let mut hits: Vec<Hit> = Vec::new();
    for (i, (tag, inner)) in elements_with_class(html, link_class).into_iter().enumerate() {
        let Some(url) = attr(tag, "href").and_then(|h| result_url(&decode_entities(h))) else { continue };
        if hits.iter().any(|h| h.url == url) {
            continue;
        }
        let title = inline_text(inner);
        if title.is_empty() {
            continue;
        }
        hits.push(Hit { title, url, snippet: snippets.get(i).cloned().unwrap_or_default() });
    }
    hits
}

/// The real address behind a DuckDuckGo result link; None for ads and non-web links.
fn result_url(href: &str) -> Option<String> {
    let full = if href.starts_with("//") { format!("https:{href}") } else { href.to_string() };
    let url = Url::parse(&full).ok()?;
    let host = url.host_str().unwrap_or("");
    if host.ends_with("duckduckgo.com") {
        if url.path().starts_with("/y.js") {
            return None;
        }
        let target = url.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.into_owned())?;
        return result_url(&target);
    }
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn parse_wikipedia(json: &str, api_url: &str) -> Vec<Hit> {
    let base = api_url.trim_end_matches("/w/api.php");
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    v["query"]["search"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|it| {
                    let title = it["title"].as_str()?.to_string();
                    let mut url = Url::parse(&format!("{base}/wiki/")).ok()?;
                    url.path_segments_mut().ok()?.pop_if_empty().push(&title.replace(' ', "_"));
                    let snippet = inline_text(it["snippet"].as_str().unwrap_or(""));
                    Some(Hit { title, url: url.to_string(), snippet })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Short markup (a title, a snippet) as one line of text.
fn inline_text(html: &str) -> String {
    collapse(&decode_entities(&strip_tags(html, false))).replace('\n', " ")
}

const DROPPED: &[&str] =
    &["script", "style", "noscript", "svg", "nav", "header", "footer", "form", "iframe", "template", "head", "aside"];
/// Formatting inside a sentence: removed without adding a space ("<b>30 Oct</b>." stays "30 Oct.").
const INLINE: &[&str] =
    &["b", "i", "em", "strong", "span", "a", "code", "small", "sup", "sub", "u", "mark", "abbr", "time", "font"];
const BLOCKS: &[&str] = &[
    "p",
    "br",
    "div",
    "li",
    "tr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "article",
    "table",
    "ul",
    "ol",
    "pre",
    "blockquote",
    "dd",
    "dt",
    "hr",
];

/// A web page as readable text: the main part when marked up as such, without scripts,
/// navigation and other chrome, one paragraph per line.
pub fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut body = html;
    for tag in ["main", "article", "body"] {
        if let Some(s) = lower.find(&format!("<{tag}"))
            && let Some(open_end) = lower[s..].find('>')
        {
            let from = s + open_end + 1;
            let to = lower[from..].rfind(&format!("</{tag}")).map(|i| from + i).unwrap_or(html.len());
            let inner = &html[from..to];
            if tag == "body" || strip_tags(inner, true).trim().len() > THIN_PAGE_CHARS / 2 {
                body = inner;
                break;
            }
        }
    }
    collapse(&decode_entities(&strip_tags(body, true)))
}

/// Removes tags (and the contents of `DROPPED` ones and comments); block tags become line breaks
/// when `blocks` is set.
fn strip_tags(html: &str, blocks: bool) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    while i < html.len() {
        let Some(rel) = lower[i..].find('<') else {
            out.push_str(&html[i..]);
            break;
        };
        out.push_str(&html[i..i + rel]);
        let start = i + rel;
        if lower[start..].starts_with("<!--") {
            i = lower[start..].find("-->").map(|e| start + e + 3).unwrap_or(html.len());
            continue;
        }
        let Some(end_rel) = lower[start..].find('>') else { break };
        let tag = &lower[start + 1..start + end_rel];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        i = start + end_rel + 1;
        if !closing && DROPPED.contains(&name.as_str()) && !tag.ends_with('/') {
            let close = format!("</{name}");
            i = lower[i..]
                .find(&close)
                .map(|e| i + e)
                .and_then(|e| lower[e..].find('>').map(|g| e + g + 1))
                .unwrap_or(html.len());
            continue;
        }
        if blocks && BLOCKS.contains(&name.as_str()) {
            out.push('\n');
        } else if !INLINE.contains(&name.as_str()) {
            out.push(' ');
        }
    }
    out
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(';').map(|e| e + 1).filter(|e| *e <= 10);
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "copy" => Some('©'),
            "euro" => Some('€'),
            "middot" => Some('·'),
            n if n.starts_with("#x") || n.starts_with("#X") => {
                u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32)
            }
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One space between words, one line per paragraph, no blank lines.
fn collapse(s: &str) -> String {
    s.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------- headless rendering

/// An installed Chromium-family browser that can render headless.
pub fn find_browser() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Microsoft Edge", "Chromium", "Brave Browser"] {
            let rel = format!("{app}.app/Contents/MacOS/{app}");
            candidates.push(Path::new("/Applications").join(&rel));
            if let Some(home) = std::env::home_dir() {
                candidates.push(home.join("Applications").join(&rel));
            }
        }
    } else if cfg!(windows) {
        let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .iter()
            .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
            .collect();
        for root in roots {
            for rel in [
                r"Google\Chrome\Application\chrome.exe",
                r"Microsoft\Edge\Application\msedge.exe",
                r"BraveSoftware\Brave-Browser\Application\brave.exe",
                r"Chromium\Application\chrome.exe",
            ] {
                candidates.push(root.join(rel));
            }
        }
    } else {
        for bin in
            ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "microsoft-edge", "brave-browser"]
        {
            candidates.push(Path::new("/usr/bin").join(bin));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The browser's arguments for rendering `url`, whose `host` was checked to be at the public
/// address `ip`.
///
/// The browser resolves host names itself, so checking the address beforehand is not enough: the
/// page could resolve again to a private address (DNS rebinding), redirect to one, or load and
/// fetch from other hosts in JavaScript. So the browser is pinned: `--host-resolver-rules` maps
/// the page's host to the checked address and every other host (literal addresses included) to
/// nothing; the first matching rule wins. This was chosen over rendering the already-fetched
/// HTML, which would still let its scripts reach any address. A proxy would resolve names for the
/// browser, past the rules, so none is used; WebRTC sends no UDP outside a proxy, so none at all.
/// Pages that need other hosts (a CDN, a redirect to another name) render less; the fetched text
/// or the search snippet is used then.
fn render_args(profile: &Path, url: &str, host: &str, ip: IpAddr) -> Result<Vec<String>, String> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    // Only plain host names and addresses go into the rules, so a host cannot add rules of its own.
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':')) {
        return Err("Not a valid web address.".into());
    }
    let ip = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    let mut args: Vec<String> = [
        "--headless=new",
        "--disable-gpu",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--incognito",
        "--mute-audio",
        "--hide-scrollbars",
        "--virtual-time-budget=6000",
        "--no-proxy-server",
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
    ]
    .map(String::from)
    .to_vec();
    args.push(format!("--host-resolver-rules=MAP {host} {ip}, MAP * ~NOTFOUND"));
    args.push(format!("--user-data-dir={}", profile.display()));
    args.push("--dump-dom".into());
    args.push(url.into());
    Ok(args)
}

/// Runs the browser headless with a new, empty profile that is deleted afterwards, so the user's
/// cookies, sessions and history are never used. No window is shown.
fn render_blocking(
    browser: &Path,
    url: &str,
    host: &str,
    ip: IpAddr,
    timeout: Duration,
    stop: &dyn Fn() -> bool,
) -> Result<String, String> {
    let profile = std::env::temp_dir().join(format!("distroclaw-render-{}", uuid::Uuid::new_v4()));
    let args = render_args(&profile, url, host, ip)?;
    std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(browser);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let result = (|| {
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let stdout = process::drain(child.stdout.take());
        let started = Instant::now();
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                break;
            }
            if stop() || started.elapsed() >= timeout {
                process::kill_group(&mut child);
                let _ = child.wait();
                return Err(if stop() {
                    "Stopped.".to_string()
                } else {
                    "The page took too long to render.".to_string()
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned())
    })();
    let _ = std::fs::remove_dir_all(&profile);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeSite, Reply};

    const DDG: &str = r#"<div class="result results_links results_links_deep web-result">
      <h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fblog.rust-lang.org%2Freleases%2F&amp;rut=abc">Rust <b>Blog</b> &amp; releases</a></h2>
      <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">Rust 1.99 was released on <b>30 October</b>.</a></div>
      <div class="result result--ad"><a class="result__a" href="https://duckduckgo.com/y.js?ad_domain=x">Ad</a><a class="result__snippet">Buy</a></div>
      <div class="result"><a class="result__a" href="https://www.rust-lang.org/">Rust language</a><a class="result__snippet">Fast and safe.</a></div>"#;

    #[test]
    fn duckduckgo_results_are_decoded_and_ads_dropped() {
        let hits = parse_ddg(DDG, "result__a", "result__snippet");
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].title, "Rust Blog & releases");
        assert_eq!(hits[0].url, "https://blog.rust-lang.org/releases/");
        assert_eq!(hits[0].snippet, "Rust 1.99 was released on 30 October.");
        assert_eq!(hits[1].url, "https://www.rust-lang.org/");
        let single_quotes = r#"<a rel="nofollow" href='https://example.org/a' class='result__a'>Example A</a>
            <a class='result__snippet'>About A.</a>"#;
        let hits = parse_ddg(single_quotes, "result__a", "result__snippet");
        let first = (hits[0].title.as_str(), hits[0].url.as_str(), hits[0].snippet.as_str());
        assert_eq!(first, ("Example A", "https://example.org/a", "About A."));
    }

    #[test]
    fn wikipedia_results_become_article_links() {
        let json = r#"{"query":{"search":[{"title":"Rust (programming language)","snippet":"a <span class=\"searchmatch\">language</span>"}]}}"#;
        let hits = parse_wikipedia(json, "https://en.wikipedia.org/w/api.php");
        assert_eq!(hits[0].url, "https://en.wikipedia.org/wiki/Rust_(programming_language)");
        assert_eq!(hits[0].snippet, "a language");
    }

    #[test]
    fn pages_become_readable_text_without_scripts_or_navigation() {
        let html = r#"<html><head><title>T</title><style>p{}</style></head><body>
            <nav>Home | About</nav><header>Site</header>
            <main><h1>Rust 1.99</h1><p>Released on 30&nbsp;October&#44; 2026 &#x2014; with <b>new</b> features.</p>
            <script>alert('x')</script><!-- note --><p>Second   paragraph.</p></main>
            <footer>© 2026</footer></body></html>"#;
        let text = html_to_text(html);
        assert_eq!(text, "Rust 1.99\nReleased on 30 October, 2026 — with new features.\nSecond paragraph.");
        assert_eq!(decode_entities("a &amp;&amp; b &unknown; &#39;c&#39; & d"), "a && b &unknown; 'c' & d");
    }

    #[test]
    fn only_public_addresses_count() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn ipv6_forms_of_ipv4_addresses_count_as_that_ipv4_address() {
        for ip in [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "::192.168.0.1",
            "64:ff9b::10.0.0.1",
            "64:ff9b::127.0.0.1",
            "64:ff9b::169.254.169.254",
            "2002:0a00:0001::1",
            "2002:7f00:0001::",
            "2002:a9fe:a9fe::1",
            "64:ff9b:1::a00:1",
            "fec0::1",
            "2001:db8::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["::ffff:8.8.8.8", "::8.8.8.8", "64:ff9b::8.8.8.8", "2002:0808:0808::1", "2a00:1450:4001::200e"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn a_render_reaches_only_the_checked_address_of_the_page_host() {
        let profile = Path::new("/tmp/profile");
        let args =
            render_args(profile, "https://example.com/a", "example.com", "93.184.216.34".parse().unwrap()).unwrap();
        assert!(args.contains(&"--host-resolver-rules=MAP example.com 93.184.216.34, MAP * ~NOTFOUND".to_string()));
        assert!(args.contains(&"--no-proxy-server".to_string()));
        assert!(args.contains(&"--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/profile".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("https://example.com/a"));
        let v6 =
            render_args(profile, "https://[2606:4700::1111]/", "[2606:4700::1111]", "2606:4700::1111".parse().unwrap())
                .unwrap();
        assert!(
            v6.contains(&"--host-resolver-rules=MAP 2606:4700::1111 [2606:4700::1111], MAP * ~NOTFOUND".to_string())
        );
        for host in ["a.com,MAP", "a.com MAP * 127.0.0.1", "*.a.com", ""] {
            assert!(render_args(profile, "https://a.com/", host, "1.1.1.1".parse().unwrap()).is_err(), "{host:?}");
        }
    }

    /// A stand-in browser that prints its arguments as the page.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_browser_is_started_with_the_pinned_address() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let browser = dir.path().join("browser");
        std::fs::write(&browser, "#!/bin/sh\necho \"$@\"\n").unwrap();
        std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o755)).unwrap();
        let web = Web::new(WebConfig { browser: Some(browser), allow_private: false, ..WebConfig::default() });
        let text = web.render("http://93.184.216.34/page", || false).await.unwrap();
        assert!(text.contains("--host-resolver-rules=MAP 93.184.216.34 93.184.216.34, MAP * ~NOTFOUND"), "{text}");
        assert!(text.ends_with("http://93.184.216.34/page"), "{text}");
        for private in
            ["http://127.0.0.1/", "http://[::ffff:10.0.0.1]/", "http://[64:ff9b::7f00:1]/", "http://localhost/"]
        {
            assert!(web.render(private, || false).await.is_err(), "{private}");
        }
    }

    #[tokio::test]
    async fn private_addresses_and_other_schemes_are_refused() {
        let site = FakeSite::start(|_| vec![("/a", Reply::html("<p>hi</p>"))]).await;
        let web = Web::new(WebConfig { allow_private: false, browser: None, ..WebConfig::default() });
        for url in
            [format!("{}/a", site.url), "http://localhost/".into(), "file:///etc/passwd".into(), "ftp://x/".into()]
        {
            assert!(web.fetch(&url).await.is_err(), "{url}");
        }
        assert_eq!(site.hits().len(), 0, "nothing was requested");
        let open = Web::new(WebConfig { allow_private: true, browser: None, ..WebConfig::default() });
        assert_eq!(open.fetch(&format!("{}/a", site.url)).await.unwrap().text, "hi");
    }

    #[tokio::test]
    async fn a_bot_check_pauses_the_provider_instead_of_working_around_it() {
        let wiki = r#"{"query":{"search":[{"title":"Rust (programming language)","snippet":"A language"}]}}"#;
        let site = FakeSite::start(|_| {
            vec![
                ("/html/", Reply { status: 202, ..Reply::html("<div class='anomaly-modal'>Are you a robot?</div>") }),
                ("/w/api.php", Reply { content_type: "application/json", ..Reply::html(wiki) }),
            ]
        })
        .await;
        let web = Web::new(WebConfig {
            search_url: format!("{}/html/", site.url),
            wiki_url: format!("{}/w/api.php", site.url),
            browser: None,
            allow_private: true,
        });
        let hits = web.search("rust release", 5).await.unwrap();
        assert!(hits[0].url.ends_with("/wiki/Rust_(programming_language)"), "{hits:?}");
        web.search("rust again", 5).await.unwrap();
        let asked: Vec<String> = site.hits().iter().map(|h| h.split('?').next().unwrap().to_string()).collect();
        assert_eq!(asked, ["/html/", "/w/api.php", "/w/api.php"], "the paused provider is not asked again");
        assert!(is_bot_check("<p>Please complete the CAPTCHA</p>") && !is_bot_check("<p>Rust 1.99</p>"));
    }

    /// Needs internet and a Chromium-family browser: `cargo test real_render -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_render_reads_a_page_without_a_window() {
        let web = Web::new(WebConfig::default());
        let browser = web.cfg.browser.clone().expect("a browser is installed");
        let started = Instant::now();
        let text = web.render("https://example.com/", || false).await.unwrap();
        println!("rendered with {} in {:?}: {}", browser.display(), started.elapsed(), &text[..text.len().min(200)]);
        assert!(text.contains("documentation examples"), "{text}");
    }

    /// Needs a Chromium-family browser: `cargo test real_pinned_render -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_pinned_render_reaches_no_other_host_from_javascript() {
        let site = FakeSite::start(|url| {
            let port = url.rsplit(':').next().unwrap().to_string();
            let page = format!(
                "<html><body><p>Page</p><script>fetch('/allowed');fetch('http://127.0.0.1:{port}/by-address');\
                 fetch('http://other.localhost:{port}/by-name')</script></body></html>"
            );
            vec![("/a", Reply::html(page)), ("/allowed", Reply::html("ok"))]
        })
        .await;
        let port = site.url.rsplit(':').next().unwrap();
        let web = Web::new(WebConfig { allow_private: true, ..WebConfig::default() });
        web.cfg.browser.as_ref().expect("a browser is installed");
        // The page is served from "localhost", pinned to 127.0.0.1.
        let text = web.render(&format!("http://localhost:{port}/a"), || false).await.unwrap();
        assert!(text.contains("Page"), "{text}");
        let hits = site.hits();
        assert!(hits.contains(&"/allowed".to_string()), "the page's own host is reached: {hits:?}");
        assert!(!hits.iter().any(|h| h.starts_with("/by-")), "other hosts are not: {hits:?}");
    }

    /// Needs internet: `cargo test real_search -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_search_finds_results() {
        let web = Web::new(WebConfig::default());
        let hits = web.search("latest stable Rust version", 5).await.unwrap();
        for h in &hits {
            println!("{} | {} | {}", h.title, h.url, h.snippet);
        }
        assert!(!hits.is_empty());
        let page = web.fetch(&hits[0].url).await.unwrap();
        println!("page: {} chars, scripted {}", page.text.len(), page.scripted);
    }
}
