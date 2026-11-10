//! Reading a page or a public document for Olymp, with the site's rules respected:
//! - robots.txt is checked before every request, redirects included, and Crawl-delay kept;
//! - one request at a time per site, at the site's pace; a 429 or 503 with Retry-After holds the
//!   site, and a wait longer than `MAX_WAIT` means the page is skipped;
//! - bot checks, logins and paywalls (401, 403) are respected, never worked around;
//! - only public addresses are reached, documents are size-checked before they are downloaded,
//!   and what a file is comes from what the server sends, never from its address.
//!
//! A page that needs JavaScript goes to a local Crawl4AI when one is set up (only after it came
//! from a public address, and its result is dropped if it ended anywhere else), or else to a
//! headless render in an installed browser, once per reply.

use super::robots::{self, Robots};
use super::{
    HOST_GAP, MAX_DOC_BYTES, MAX_PAGE_BYTES, MAX_TURN_BYTES, MAX_WAIT, Olymp, ROBOTS_TOKEN, Session, USER_AGENT,
};
use crate::web::{THIN_PAGE_CHARS, checked_url, html_to_text, is_bot_check, resolve_public};
use reqwest::Url;
use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const MAX_HOPS: usize = 4;
const ACCEPT_DOCS: &str = "text/html,application/xhtml+xml,text/plain;q=0.9,application/pdf;q=0.9,\
    application/vnd.openxmlformats-officedocument.*;q=0.8,application/vnd.oasis.opendocument.*;q=0.8,*/*;q=0.5";
/// Extracting the text of one document.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Page,
    Text,
    Pdf,
    Word,
    Excel,
    PowerPoint,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Text => "text",
            Self::Pdf => "PDF",
            Self::Word => "Word",
            Self::Excel => "Excel",
            Self::PowerPoint => "PowerPoint",
        }
    }

    /// A file name with the extension `attachments::extract` reads it by.
    fn file_name(self, sheet_ext: &str) -> String {
        match self {
            Self::Pdf => "download.pdf".into(),
            Self::Word => "download.docx".into(),
            Self::Excel => format!("download.{sheet_ext}"),
            Self::PowerPoint => "download.pptx".into(),
            Self::Page | Self::Text => "download.txt".into(),
        }
    }
}

#[derive(Debug)]
pub struct Fetched {
    /// Where the page ended up after redirects.
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    pub kind: Kind,
}

/// What a response is, from its Content-Type, or from its first bytes when the server does not
/// say. Returns the kind and, for spreadsheets, the format.
fn kind_of(content_type: &str, head: &[u8]) -> Option<(Kind, &'static str)> {
    let ct = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let by_type = match ct.as_str() {
        "text/html" | "application/xhtml+xml" => Some((Kind::Page, "")),
        "text/plain" | "text/csv" | "text/markdown" => Some((Kind::Text, "")),
        "application/pdf" => Some((Kind::Pdf, "")),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some((Kind::Word, "")),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some((Kind::Excel, "xlsx")),
        "application/vnd.ms-excel" => Some((Kind::Excel, "xls")),
        "application/vnd.oasis.opendocument.spreadsheet" => Some((Kind::Excel, "ods")),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some((Kind::PowerPoint, "")),
        _ => None,
    };
    by_type.or_else(|| sniff(head))
}

/// A file's kind from its first bytes: `%PDF`, an Office zip, an old Excel file.
fn sniff(head: &[u8]) -> Option<(Kind, &'static str)> {
    let start: Vec<u8> =
        head.iter().skip_while(|b| b.is_ascii_whitespace()).take(15).map(u8::to_ascii_lowercase).collect();
    if start.starts_with(b"<!doctype html") || start.starts_with(b"<html") {
        return Some((Kind::Page, ""));
    }
    if head.starts_with(b"%PDF") {
        return Some((Kind::Pdf, ""));
    }
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Some((Kind::Excel, "xls"));
    }
    if head.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(head)).ok()?;
        let names: Vec<String> = zip.file_names().map(str::to_string).collect();
        if names.iter().any(|n| n == "word/document.xml") {
            return Some((Kind::Word, ""));
        }
        if names.iter().any(|n| n == "xl/workbook.xml") {
            return Some((Kind::Excel, "xlsx"));
        }
        if names.iter().any(|n| n.starts_with("ppt/slides/")) {
            return Some((Kind::PowerPoint, ""));
        }
        let mut mimetype = String::new();
        if let Ok(mut f) = zip.by_name("mimetype") {
            use std::io::Read;
            let _ = f.read_to_string(&mut mimetype);
        }
        if mimetype.trim() == "application/vnd.oasis.opendocument.spreadsheet" {
            return Some((Kind::Excel, "ods"));
        }
    }
    None
}

fn origin(url: &Url) -> String {
    format!("{}://{}:{}", url.scheme(), url.host_str().unwrap_or(""), url.port_or_known_default().unwrap_or(0))
}

fn path_and_query(url: &Url) -> String {
    match url.query() {
        Some(q) => format!("{}?{q}", url.path()),
        None => url.path().to_string(),
    }
}

/// The site's robots.txt rules for Olymp, fetched once and kept.
async fn robots_for(o: &Olymp, url: &Url) -> Arc<Robots> {
    let key = origin(url);
    if let Some(r) = o.robots.get(&key) {
        return r;
    }
    let Ok(robots_url) = url.join("/robots.txt") else { return o.robots.put(&key, Robots::deny_all(), false) };
    let answer = o.apis.get(robots_url).header(reqwest::header::USER_AGENT, USER_AGENT).send().await;
    match answer {
        Ok(resp) if resp.status().is_success() => {
            let mut body = Vec::new();
            let mut resp = resp;
            while let Ok(Some(chunk)) = resp.chunk().await {
                body.extend_from_slice(&chunk);
                if body.len() >= robots::MAX_BYTES {
                    body.truncate(robots::MAX_BYTES);
                    break;
                }
            }
            o.robots.put(&key, robots::parse(&String::from_utf8_lossy(&body), ROBOTS_TOKEN), true)
        }
        // RFC 9309: an unavailable robots.txt (4xx) allows everything…
        Ok(resp) if resp.status().is_client_error() => o.robots.put(&key, Robots::allow_all(), true),
        // …one that cannot be reached (5xx, no answer) allows nothing.
        _ => o.robots.put(&key, Robots::deny_all(), false),
    }
}

/// Reads `raw` for the session: a page as text, or a document's text.
pub async fn read(s: &Session, raw: &str) -> Result<Fetched, String> {
    let o = &s.olymp;
    let mut url = checked_url(raw, o.allow_private)?;
    for _ in 0..=MAX_HOPS {
        let host = url.host_str().unwrap_or("").to_string();
        // One request at a time per site (scheme, host and port).
        let turn = format!("host:{}", origin(&url));
        let robots = robots_for(o, &url).await;
        if !robots.allows(&path_and_query(&url)) {
            return Err(format!("{host} does not allow automated reading of this page (robots.txt)."));
        }
        let gap = robots.delay.unwrap_or(HOST_GAP).max(HOST_GAP);
        if gap > MAX_WAIT || !o.take_turn(&turn, gap).await {
            return Err(format!("{host} asks for a slower pace; use another result."));
        }
        let resp = o.pages.get(url.clone()).header(ACCEPT, ACCEPT_DOCS).send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.is_redirection() {
            let next =
                resp.headers().get(LOCATION).and_then(|v| v.to_str().ok()).ok_or("a redirect without a target")?;
            url = checked_url(url.join(next).map_err(|e| e.to_string())?.as_str(), o.allow_private)?;
            continue;
        }
        if status.as_u16() == 429 || status.as_u16() == 503 {
            if let Some(d) = super::providers::retry_after(resp.headers()) {
                o.hold(&turn, d.min(Duration::from_secs(3600)));
            }
            return Err(format!("{host} is busy and asked to come back later."));
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(format!("{host} needs a login or refuses automated reading; it was not read."));
        }
        if !status.is_success() {
            return Err(format!("the page answered with status {}", status.as_u16()));
        }
        return decode(s, &url, resp).await;
    }
    Err("too many redirects".into())
}

async fn decode(s: &Session, url: &Url, mut resp: reqwest::Response) -> Result<Fetched, String> {
    let content_type =
        resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let declared: Option<u64> = resp.headers().get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse().ok());
    let left = {
        let st = s.state.lock().unwrap();
        MAX_TURN_BYTES.saturating_sub(st.bytes)
    };
    let looks_like_page = content_type.contains("html") || content_type.starts_with("text/");
    let cap = if looks_like_page { MAX_PAGE_BYTES } else { MAX_DOC_BYTES.min(left) };
    // A document larger than allowed is refused before it is downloaded, not cut.
    if !looks_like_page && declared.is_some_and(|n| n > cap) {
        return Err(format!(
            "the file is larger than {} MB, so it was not downloaded.",
            cap.div_ceil(1024 * 1024).max(1)
        ));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > cap {
            if !looks_like_page {
                return Err("the file is too large, so it was not read.".into());
            }
            body.truncate(cap as usize);
            break;
        }
    }
    s.state.lock().unwrap().bytes += body.len() as u64;
    let head = &body[..body.len().min(64 * 1024)];
    let (kind, sheet_ext) =
        kind_of(&content_type, head).ok_or_else(|| "it is neither a web page nor a readable document.".to_string())?;
    match kind {
        Kind::Page => {
            let raw = String::from_utf8_lossy(&body);
            let lower = raw.to_ascii_lowercase();
            if is_bot_check(&lower) {
                return Err("the page asks for a bot check, so it was not read.".into());
            }
            let title = title_of(&raw);
            let mut text = html_to_text(&raw);
            if text.chars().count() < THIN_PAGE_CHARS
                && lower.contains("<script")
                && let Some(t) = rendered(s, url).await.filter(|t| t.chars().count() > text.chars().count())
            {
                text = t;
            }
            if text.trim().is_empty() {
                return Err("the page has no readable text.".into());
            }
            Ok(Fetched { url: url.to_string(), title, text, kind })
        }
        Kind::Text => {
            let text = crate::web::collapse(&String::from_utf8_lossy(&body));
            Ok(Fetched { url: url.to_string(), title: None, text, kind })
        }
        _ => {
            let name = kind.file_name(if sheet_ext.is_empty() { "xlsx" } else { sheet_ext });
            let task = tauri::async_runtime::spawn_blocking(move || crate::attachments::extract(&name, &body));
            let extracted = match tokio::time::timeout(EXTRACT_TIMEOUT, task).await {
                Ok(Ok(r)) => r?,
                Ok(Err(e)) => return Err(e.to_string()),
                Err(_) => return Err("reading the file took too long.".into()),
            };
            let title = file_title(url);
            Ok(Fetched { url: url.to_string(), title, text: extracted.text, kind })
        }
    }
}

/// The page's `<title>`.
fn title_of(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let body = start + lower[start..].find('>')? + 1;
    let end = body + lower[body..].find("</title")?;
    let t = crate::web::inline_text(&html[body..end]);
    (!t.is_empty()).then_some(t)
}

/// A document's name from its address: the last path segment, decoded.
fn file_title(url: &Url) -> Option<String> {
    let last = url.path_segments()?.next_back().filter(|s| !s.is_empty())?;
    let decoded: String = url::form_urlencoded_decode(last);
    Some(decoded)
}

mod url {
    /// Percent-decoding of one path segment, without a crate for it.
    pub fn form_urlencoded_decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%'
                && i + 2 < bytes.len()
                && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
            {
                out.push(b);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

/// The text of a page that needs JavaScript: from Crawl4AI when set up, else from a headless
/// render in an installed browser (once per reply).
async fn rendered(s: &Session, url: &Url) -> Option<String> {
    if let Some(base) = s.config.crawl4ai.as_deref() {
        match crawl4ai(s, base, url).await {
            Ok(t) => return Some(t),
            Err(e) => log::info!("olymp: Crawl4AI could not render {url}: {e}"),
        }
    }
    let o = &s.olymp;
    if o.web.cfg.browser.is_none() || std::mem::replace(&mut s.state.lock().unwrap().rendered, true) {
        return None;
    }
    let stop = s.cancel.clone();
    o.web.render(url.as_str(), move || stop.is_cancelled()).await.ok()
}

/// Renders a page with Crawl4AI's REST API (`POST /crawl`). The page was already read from a
/// public address; Crawl4AI is asked to respect robots.txt itself, and its answer is dropped
/// when the page ended up anywhere but the public internet.
async fn crawl4ai(s: &Session, base: &str, url: &Url) -> Result<String, String> {
    let o = &s.olymp;
    resolve_public(url, o.allow_private).await?;
    let base = super::local::validate(base)?;
    let endpoint = base.join("crawl").map_err(|e| e.to_string())?;
    let request = json!({
        "urls": [url.as_str()],
        "browser_config": { "type": "BrowserConfig", "params": { "headless": true, "user_agent": USER_AGENT } },
        "crawler_config": {
            "type": "CrawlerRunConfig",
            "params": { "check_robots_txt": true, "cache_mode": "bypass", "stream": false }
        }
    });
    let resp = o.local.http.post(endpoint).json(&request).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status().as_u16()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let r = &v["results"][0];
    if r["success"].as_bool() == Some(false) {
        return Err(r["error_message"].as_str().unwrap_or("it could not render the page").to_string());
    }
    let landed = r["redirected_url"].as_str().or(r["url"].as_str()).unwrap_or(url.as_str());
    let landed = checked_url(landed, o.allow_private)?;
    // Wherever the render ended up besides the page itself must be on the public internet.
    if landed.host_str() != url.host_str() {
        resolve_public(&landed, false).await.map_err(|_| "the page left the public internet".to_string())?;
    }
    let markdown = match &r["markdown"] {
        serde_json::Value::String(m) => m.clone(),
        m => m["fit_markdown"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .or(m["raw_markdown"].as_str())
            .unwrap_or("")
            .to_string(),
    };
    let text = if markdown.trim().is_empty() {
        html_to_text(r["cleaned_html"].as_str().unwrap_or(""))
    } else {
        crate::web::collapse(&markdown)
    };
    if text.trim().is_empty() { Err("no text came back".into()) } else { Ok(text) }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::olymp::tests::olymp_at;
    use crate::olymp::{Config, Session};
    use crate::service::chat::Cancel;
    use crate::testing::{FakeSite, Reply};

    pub(crate) fn xlsx_bytes() -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o: zip::write::SimpleFileOptions = Default::default();
            let files = [
                (
                    "[Content_Types].xml",
                    r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
                ),
                (
                    "_rels/.rels",
                    r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
                ),
                (
                    "xl/workbook.xml",
                    r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Prices" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
                ),
                (
                    "xl/worksheets/sheet1.xml",
                    r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Fuel</t></is></c><c r="B1" t="inlineStr"><is><t>CHF per litre</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Diesel</t></is></c><c r="B2"><v>1.89</v></c></row></sheetData></worksheet>"#,
                ),
            ];
            for (name, body) in files {
                z.start_file(name, o).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    fn session(base: &str, config: Config) -> Session {
        Session::new(Arc::new(olymp_at(base)), config, vec![], Arc::new(Cancel::default()))
    }

    #[test]
    fn kinds_come_from_the_content_type_or_the_first_bytes() {
        assert_eq!(kind_of("application/pdf", b"").map(|k| k.0), Some(Kind::Pdf));
        assert_eq!(kind_of("text/html; charset=utf-8", b"").map(|k| k.0), Some(Kind::Page));
        assert_eq!(kind_of("application/octet-stream", b"%PDF-1.7").map(|k| k.0), Some(Kind::Pdf));
        assert_eq!(kind_of("application/octet-stream", &xlsx_bytes()), Some((Kind::Excel, "xlsx")));
        assert_eq!(kind_of("application/octet-stream", b"MZ\x90\x00"), None, "a program is not read");
        assert_eq!(kind_of("image/png", b"\x89PNG"), None);
    }

    #[tokio::test]
    async fn robots_txt_is_respected_and_the_page_is_never_requested() {
        let site = FakeSite::start(|_| {
            vec![
                ("/robots.txt", Reply::text("text/plain", "User-agent: *\nDisallow: /private\n")),
                ("/private/page", Reply::html("<main><p>secret</p></main>")),
                ("/open", Reply::html("<html><head><title>Open page</title></head><main><p>Hello there, this is open.</p></main></html>")),
            ]
        })
        .await;
        let s = session(&site.url, Config::default());
        let refused = read(&s, &format!("{}/private/page", site.url)).await.unwrap_err();
        assert!(refused.contains("robots.txt"), "{refused}");
        let page = read(&s, &format!("{}/open", site.url)).await.unwrap();
        assert_eq!((page.title.as_deref(), page.kind), (Some("Open page"), Kind::Page));
        assert!(page.text.contains("this is open"));
        assert_eq!(site.hits(), ["/robots.txt", "/open"], "robots.txt once, the private page never");
    }

    #[tokio::test]
    async fn an_unreachable_robots_txt_allows_nothing_and_a_missing_one_everything() {
        let broken = FakeSite::start(|_| {
            vec![
                ("/robots.txt", Reply { status: 500, ..Reply::text("text/plain", "oops") }),
                ("/a", Reply::html("<p>a</p>")),
            ]
        })
        .await;
        let s = session(&broken.url, Config::default());
        assert!(read(&s, &format!("{}/a", broken.url)).await.unwrap_err().contains("robots.txt"));
        let missing =
            FakeSite::start(|_| vec![("/a", Reply::html("<main><p>Plenty of readable text here.</p></main>"))]).await;
        let s = session(&missing.url, Config::default());
        assert!(read(&s, &format!("{}/a", missing.url)).await.is_ok(), "404 for robots.txt allows reading");
    }

    #[tokio::test]
    async fn busy_sites_logins_and_bot_checks_are_respected() {
        let busy = FakeSite::start(|_| {
            vec![(
                "/busy",
                Reply { status: 429, headers: vec![("Retry-After", "120".into())], ..Reply::html("slow down") },
            )]
        })
        .await;
        let s = session(&busy.url, Config::default());
        assert!(read(&s, &format!("{}/busy", busy.url)).await.unwrap_err().contains("busy"));
        assert!(read(&s, &format!("{}/busy", busy.url)).await.unwrap_err().contains("slower pace"), "the site is held");
        assert_eq!(busy.hits().iter().filter(|h| *h == "/busy").count(), 1, "the held site was not asked again");
        let site = FakeSite::start(|_| {
            vec![
                ("/login", Reply { status: 403, ..Reply::html("sign in") }),
                ("/check", Reply::html("<div class='cf-chl'>Are you a robot?</div>")),
            ]
        })
        .await;
        assert!(read(&s, &format!("{}/login", site.url)).await.unwrap_err().contains("login"));
        assert!(read(&s, &format!("{}/check", site.url)).await.unwrap_err().contains("bot check"));
    }

    #[tokio::test]
    async fn documents_are_read_and_oversized_ones_refused() {
        let site = FakeSite::start(|_| {
            vec![
                ("/prices.xlsx", Reply::bytes("application/octet-stream", xlsx_bytes())),
                ("/big.pdf", Reply::bytes("application/pdf", vec![b'x'; (MAX_DOC_BYTES + 1) as usize])),
                ("/notes.txt", Reply::text("text/plain", "line one\nline two")),
            ]
        })
        .await;
        let s = session(&site.url, Config::default());
        let sheet = read(&s, &format!("{}/prices.xlsx", site.url)).await.unwrap();
        assert_eq!(sheet.kind, Kind::Excel);
        assert!(sheet.text.contains("Diesel\t1.89") && sheet.text.contains("Sheet: Prices"), "{}", sheet.text);
        assert_eq!(sheet.title.as_deref(), Some("prices.xlsx"));
        let big = read(&s, &format!("{}/big.pdf", site.url)).await.unwrap_err();
        assert!(big.contains("larger than"), "{big}");
        assert_eq!(read(&s, &format!("{}/notes.txt", site.url)).await.unwrap().text, "line one\nline two");
    }

    #[tokio::test]
    async fn redirects_are_followed_with_robots_checked_on_each_hop() {
        let site = FakeSite::start(|base| {
            vec![
                ("/robots.txt", Reply::text("text/plain", "User-agent: distroclaw-olymp\nDisallow: /blocked\n")),
                ("/old", Reply { status: 301, headers: vec![("Location", format!("{base}/new"))], ..Reply::html("") }),
                ("/new", Reply::html("<main><p>The new home of the page, with enough text.</p></main>")),
                ("/sneaky", Reply { status: 302, headers: vec![("Location", "/blocked".into())], ..Reply::html("") }),
                ("/blocked", Reply::html("<p>blocked</p>")),
            ]
        })
        .await;
        let s = session(&site.url, Config::default());
        let page = read(&s, &format!("{}/old", site.url)).await.unwrap();
        assert!(page.url.ends_with("/new"));
        assert!(read(&s, &format!("{}/sneaky", site.url)).await.unwrap_err().contains("robots.txt"));
        assert!(!site.hits().contains(&"/blocked".to_string()));
    }

    #[tokio::test]
    async fn crawl4ai_only_gets_public_pages_and_its_answer_must_stay_public() {
        // A fake Crawl4AI on loopback that would render anything.
        let crawler = FakeSite::start(|_| {
            vec![(
                "/crawl",
                Reply::json(r#"{"results":[{"success":true,"url":"http://10.0.0.7/admin","markdown":{"raw_markdown":"router admin"}}]}"#),
            )]
        })
        .await;
        let site = FakeSite::start(|_| {
            vec![("/app", Reply::html("<html><body><div id=root></div><script src=/a.js></script></body></html>"))]
        })
        .await;
        let config = Config { crawl4ai: Some(crawler.url.clone()), ..Config::default() };
        let s = session(&site.url, config);
        // The page's own text is thin and the render ended on a private address: nothing is used.
        let r = read(&s, &format!("{}/app", site.url)).await;
        assert!(r.is_err(), "{r:?}");
        assert_eq!(crawler.hits(), ["/crawl"]);
        // A private address is never sent to Crawl4AI in the first place.
        let mut strict = olymp_at(&site.url);
        strict.allow_private = false;
        let s = Session::new(
            Arc::new(strict),
            Config { crawl4ai: Some(crawler.url.clone()), ..Config::default() },
            vec![],
            Arc::new(Cancel::default()),
        );
        assert!(read(&s, "http://10.0.0.7/admin").await.is_err());
        assert_eq!(crawler.hits().len(), 1, "Crawl4AI was not asked again");
    }
}
