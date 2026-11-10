//! Search providers. Each lives in a file of its own and answers a `Query` with `Hit`s; the
//! registry (`ALL`) says which kinds of search it serves, which document types it can filter by,
//! how often it may be asked, and under which key the user can switch it off. A new source is a
//! new file plus one registry entry.
//!
//! Each provider follows its own usage policy: rate limits are kept as a pace between requests,
//! a Retry-After or rate-limit answer pauses the provider, and a bot check pauses it too and is
//! never worked around.

mod arxiv;
mod brave;
mod crossref;
mod ddg;
mod gdelt;
mod github;
mod google;
mod hn;
mod openalex;
mod searxng;
mod stackexchange;
mod wikipedia;

use super::{Config, Olymp, PROVIDER_TIMEOUT};
use futures_util::future::join_all;
use reqwest::header::HeaderMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// What kind of sources a search is after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Web,
    News,
    Papers,
    Code,
}

impl Focus {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "web" | "general" => Some(Self::Web),
            "news" => Some(Self::News),
            "papers" | "paper" | "science" | "research" | "academic" => Some(Self::Papers),
            "code" | "dev" | "developer" | "programming" | "docs" => Some(Self::Code),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Web => "the web",
            Self::News => "news",
            Self::Papers => "papers",
            Self::Code => "developer sources",
        }
    }
}

/// Public documents a search can be limited to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Csv,
}

impl FileType {
    /// A filetype value that asks for no particular type.
    pub fn is_any(s: &str) -> bool {
        matches!(
            s.trim().trim_start_matches('.').to_ascii_lowercase().as_str(),
            "" | "any" | "all" | "none" | "null" | "web" | "html" | "htm" | "page" | "webpage"
        )
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" | "doc" | "word" => Some(Self::Docx),
            "xlsx" | "xls" | "excel" | "spreadsheet" => Some(Self::Xlsx),
            "pptx" | "ppt" | "powerpoint" | "slides" => Some(Self::Pptx),
            "csv" => Some(Self::Csv),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Csv => "csv",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx => "Word",
            Self::Xlsx => "Excel",
            Self::Pptx => "PowerPoint",
            Self::Csv => "CSV",
        }
    }

    /// Whether `url`'s path names a file of this type (xls and doc count for xlsx and docx).
    pub fn matches_url(self, url: &str) -> bool {
        let path = reqwest::Url::parse(url).map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
        let exts: &[&str] = match self {
            Self::Pdf => &[".pdf"],
            Self::Docx => &[".docx", ".doc"],
            Self::Xlsx => &[".xlsx", ".xls", ".xlsm", ".ods"],
            Self::Pptx => &[".pptx", ".ppt"],
            Self::Csv => &[".csv"],
        };
        exts.iter().any(|e| path.ends_with(e))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub text: String,
    pub focus: Focus,
    pub filetype: Option<FileType>,
    /// Two-letter language code, when the user wants results in one language.
    pub lang: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// When the page or work was published, as YYYY-MM-DD or YYYY.
    pub date: Option<String>,
    /// The providers that found it.
    pub from: Vec<&'static str>,
    /// The address is a document of the requested type (a PDF link from a paper index).
    pub doc: bool,
}

impl Hit {
    pub fn new(
        title: impl Into<String>,
        url: impl Into<String>,
        snippet: impl Into<String>,
        from: &'static str,
    ) -> Self {
        let clip = |s: String| -> String {
            let s = crate::web::collapse(&s).replace('\n', " ");
            match s.char_indices().nth(400) {
                Some((i, _)) => format!("{}…", &s[..i]),
                None => s,
            }
        };
        Self {
            title: clip(title.into()),
            url: url.into(),
            snippet: clip(snippet.into()),
            date: None,
            from: vec![from],
            doc: false,
        }
    }

    pub fn dated(mut self, date: Option<String>) -> Self {
        self.date = date.filter(|d| !d.is_empty());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// Switched off, or no key or address set.
    Skipped,
    /// The local service does not answer.
    NotRunning,
    Paused,
    BotCheck,
    /// Rate-limited; wait that long before asking again.
    Limited(Option<Duration>),
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id {
    SearXng,
    DuckDuckGo,
    Wikipedia,
    Gdelt,
    HackerNews,
    OpenAlex,
    Arxiv,
    Crossref,
    StackExchange,
    GitHub,
    Brave,
    Google,
}

pub struct Info {
    pub id: Id,
    /// Settings key suffix and short id: `olymp_<key>` switches it.
    pub key: &'static str,
    pub label: &'static str,
    pub focus: &'static [Focus],
    /// The document types it can filter by.
    pub filetypes: &'static [FileType],
    /// The pace its usage policy asks for.
    pub gap: Duration,
    /// Asked unless the user switches it off; when false, only once the user switches it on.
    pub default_on: bool,
}

const ANY_DOC: &[FileType] = &[FileType::Pdf, FileType::Docx, FileType::Xlsx, FileType::Pptx, FileType::Csv];

pub const ALL: &[Info] = &[
    Info {
        id: Id::SearXng,
        key: "searxng",
        label: "SearXNG",
        focus: &[Focus::Web, Focus::News, Focus::Papers, Focus::Code],
        filetypes: ANY_DOC,
        gap: Duration::from_millis(300),
        default_on: true,
    },
    Info {
        id: Id::DuckDuckGo,
        key: "duckduckgo",
        label: "DuckDuckGo",
        focus: &[Focus::Web, Focus::Code],
        filetypes: ANY_DOC,
        gap: Duration::from_secs(2),
        default_on: true,
    },
    Info {
        id: Id::Wikipedia,
        key: "wikipedia",
        label: "Wikipedia",
        focus: &[Focus::Web],
        filetypes: &[],
        gap: Duration::from_millis(500),
        default_on: true,
    },
    Info {
        id: Id::Gdelt,
        key: "gdelt",
        label: "GDELT",
        focus: &[Focus::News],
        filetypes: &[],
        gap: Duration::from_secs(5),
        default_on: false,
    },
    Info {
        id: Id::HackerNews,
        key: "hackernews",
        label: "Hacker News",
        focus: &[Focus::News, Focus::Code],
        filetypes: &[],
        gap: Duration::from_secs(1),
        default_on: true,
    },
    Info {
        id: Id::OpenAlex,
        key: "openalex",
        label: "OpenAlex",
        focus: &[Focus::Papers],
        filetypes: &[FileType::Pdf],
        gap: Duration::from_secs(1),
        default_on: true,
    },
    Info {
        id: Id::Arxiv,
        key: "arxiv",
        label: "arXiv",
        focus: &[Focus::Papers],
        filetypes: &[FileType::Pdf],
        gap: Duration::from_secs(3),
        default_on: true,
    },
    Info {
        id: Id::Crossref,
        key: "crossref",
        label: "Crossref",
        focus: &[Focus::Papers],
        filetypes: &[],
        gap: Duration::from_secs(1),
        default_on: true,
    },
    Info {
        id: Id::StackExchange,
        key: "stackexchange",
        label: "Stack Exchange",
        focus: &[Focus::Code],
        filetypes: &[],
        gap: Duration::from_secs(1),
        default_on: true,
    },
    Info {
        id: Id::GitHub,
        key: "github",
        label: "GitHub",
        focus: &[Focus::Code],
        filetypes: &[],
        gap: Duration::from_secs(6),
        default_on: true,
    },
    Info {
        id: Id::Brave,
        key: "brave",
        label: "Brave Search",
        focus: &[Focus::Web, Focus::News],
        filetypes: ANY_DOC,
        gap: Duration::from_secs(1),
        default_on: true,
    },
    Info {
        id: Id::Google,
        key: "google",
        label: "Google",
        focus: &[Focus::Web],
        filetypes: ANY_DOC,
        gap: Duration::from_secs(1),
        default_on: true,
    },
];

pub fn info(id: Id) -> &'static Info {
    ALL.iter().find(|p| p.id == id).expect("every provider is registered")
}

/// The providers that serve `q`, in registry order.
pub fn chosen(q: &Query, cfg: &Config) -> Vec<&'static Info> {
    ALL.iter()
        .filter(|p| p.focus.contains(&q.focus))
        .filter(|p| q.filetype.is_none_or(|ft| p.filetypes.contains(&ft)))
        .filter(|p| !cfg.is_off(p.key))
        .collect()
}

async fn search_one(o: &Olymp, cfg: &Config, id: Id, q: &Query) -> Result<Vec<Hit>, Failure> {
    match id {
        Id::SearXng => searxng::search(o, cfg, q).await,
        Id::DuckDuckGo => ddg::search(o, q).await,
        Id::Wikipedia => wikipedia::search(o, q).await,
        Id::Gdelt => gdelt::search(o, q).await,
        Id::HackerNews => hn::search(o, q).await,
        Id::OpenAlex => openalex::search(o, q).await,
        Id::Arxiv => arxiv::search(o, q).await,
        Id::Crossref => crossref::search(o, q).await,
        Id::StackExchange => stackexchange::search(o, q).await,
        Id::GitHub => github::search(o, q).await,
        Id::Brave => brave::search(o, cfg, q).await,
        Id::Google => google::search(o, cfg, q).await,
    }
}

/// What one search got from each provider.
pub struct Outcome {
    /// One ranked list per provider that answered.
    pub lists: Vec<Vec<Hit>>,
    pub report: Vec<(Id, Result<usize, Failure>)>,
}

/// Asks every chosen provider at once, each in its turn and within `PROVIDER_TIMEOUT`.
pub async fn search_all(o: &Olymp, cfg: &Config, q: &Query) -> Outcome {
    // Sources that don't understand operators like site: would search for them as words.
    let plain = Query { text: without_operators(&q.text), ..q.clone() };
    let asks = chosen(q, cfg).into_iter().map(|p| {
        let q = if takes_operators(p.id) { q } else { &plain };
        async move {
            if o.paused(p.key) {
                return (p.id, Err(Failure::Paused));
            }
            if !o.take_turn(&format!("provider:{}", p.key), p.gap).await {
                return (p.id, Err(Failure::Limited(None)));
            }
            let r = match tokio::time::timeout(PROVIDER_TIMEOUT, search_one(o, cfg, p.id, q)).await {
                Ok(r) => r,
                Err(_) => Err(Failure::Other("no answer in time".into())),
            };
            match &r {
                Err(Failure::BotCheck) => o.pause(p.key, crate::web::PROVIDER_PAUSE),
                Err(Failure::Limited(d)) => {
                    o.pause(p.key, d.unwrap_or(Duration::from_secs(60)).min(Duration::from_secs(3600)))
                }
                Err(Failure::Other(e)) => log::info!("olymp: {} failed: {e}", p.label),
                _ => {}
            }
            (p.id, r)
        }
    });
    let answers = join_all(asks).await;
    let mut lists = Vec::new();
    let mut report = Vec::new();
    for (id, r) in answers {
        match r {
            Ok(hits) => {
                report.push((id, Ok(hits.len())));
                if !hits.is_empty() {
                    lists.push(hits);
                }
            }
            Err(f) => report.push((id, Err(f))),
        }
    }
    Outcome { lists, report }
}

/// "DuckDuckGo 6, Wikipedia 3 · SearXNG not running", for the tool row and the model.
pub fn describe(report: &[(Id, Result<usize, Failure>)]) -> String {
    let answered: Vec<String> =
        report.iter().filter_map(|(id, r)| r.as_ref().ok().map(|n| format!("{} {n}", info(*id).label))).collect();
    let trouble: Vec<String> = report
        .iter()
        .filter_map(|(id, r)| {
            let why = match r.as_ref().err()? {
                Failure::Skipped => return None,
                Failure::NotRunning => "not running",
                Failure::Paused => "paused",
                Failure::BotCheck => "asked for a bot check",
                Failure::Limited(_) => "busy",
                Failure::Other(_) => "did not answer",
            };
            Some(format!("{} {why}", info(*id).label))
        })
        .collect();
    let mut s = if answered.is_empty() { "no source answered".to_string() } else { answered.join(", ") };
    if !trouble.is_empty() {
        s.push_str(" · ");
        s.push_str(&trouble.join(", "));
    }
    s
}

/// Sends a provider request and reads the body; rate limits, bot checks and errors become a
/// `Failure`.
pub(crate) async fn send(req: reqwest::RequestBuilder) -> Result<String, Failure> {
    let resp = req.send().await.map_err(|e| Failure::Other(e.to_string()))?;
    let status = resp.status();
    let headers = resp.headers().clone();
    if status.as_u16() == 429 || (status.as_u16() == 403 && rate_limited(&headers)) {
        return Err(Failure::Limited(retry_after(&headers)));
    }
    if status.as_u16() == 202 {
        return Err(Failure::BotCheck);
    }
    if !status.is_success() {
        return Err(Failure::Other(format!("status {}", status.as_u16())));
    }
    let body = resp.text().await.map_err(|e| Failure::Other(e.to_string()))?;
    Ok(body)
}

fn rate_limited(h: &HeaderMap) -> bool {
    h.get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()) == Some("0") || h.contains_key("retry-after")
}

/// How long a provider asked to be left alone: Retry-After seconds, or a rate-limit reset time.
pub(crate) fn retry_after(h: &HeaderMap) -> Option<Duration> {
    let secs = |name: &str| h.get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(s) = secs("retry-after") {
        return Some(Duration::from_secs(s));
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    secs("x-ratelimit-reset").map(|reset| Duration::from_secs(reset.saturating_sub(now)))
}

/// Web search engines that understand operators like `site:` and `filetype:`.
fn takes_operators(id: Id) -> bool {
    matches!(id, Id::SearXng | Id::DuckDuckGo | Id::Brave | Id::Google)
}

/// The words of a query without search operators (`site:`, `filetype:`, `intitle:`…). A query
/// that is nothing but operators keeps the words after the colons.
pub(crate) fn without_operators(text: &str) -> String {
    const OPERATORS: &[&str] = &[
        "site:",
        "-site:",
        "filetype:",
        "ext:",
        "inurl:",
        "intitle:",
        "allintitle:",
        "allinurl:",
        "before:",
        "after:",
    ];
    let is_op = |w: &str| {
        let l = w.to_ascii_lowercase();
        OPERATORS.iter().any(|op| l.starts_with(op))
    };
    let kept: Vec<&str> = text.split_whitespace().filter(|w| !is_op(w)).collect();
    if !kept.is_empty() {
        return kept.join(" ");
    }
    text.split_whitespace()
        .filter_map(|w| w.split_once(':').map(|(_, v)| v.trim_matches('"')))
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The query with a `filetype:` operator, for providers whose engines understand it.
pub(crate) fn with_filetype(q: &Query) -> String {
    match q.filetype {
        Some(ft) => format!("{} filetype:{}", q.text, ft.ext()),
        None => q.text.clone(),
    }
}

/// A language code Olymp passes on: en, de, fr, it and the like.
pub(crate) fn lang(q: &Query) -> Option<&str> {
    q.lang.as_deref().filter(|l| l.len() == 2 && l.chars().all(|c| c.is_ascii_lowercase()))
}

/// Parses JSON leniently: a body that is not JSON counts as no results.
pub(crate) fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {

    #[test]
    fn operators_are_left_out_for_sources_that_take_them_as_words() {
        assert_eq!(without_operators("latest stable rust site:rust-lang.org release"), "latest stable rust release");
        assert_eq!(without_operators("heat pump FILETYPE:pdf intitle:guide"), "heat pump");
        assert_eq!(without_operators("site:rust-lang.org"), "rust-lang.org");
        assert_eq!(without_operators("rust 1.80: what changed"), "rust 1.80: what changed");
        assert!(takes_operators(Id::DuckDuckGo) && !takes_operators(Id::Wikipedia));
    }
    use super::*;

    pub fn q(text: &str, focus: Focus) -> Query {
        Query { text: text.into(), focus, filetype: None, lang: None }
    }

    #[test]
    fn providers_are_chosen_by_focus_type_and_switches() {
        let ids = |q: &Query, cfg: &Config| chosen(q, cfg).iter().map(|p| p.key).collect::<Vec<_>>();
        let cfg = Config::default();
        assert_eq!(ids(&q("x", Focus::Web), &cfg), ["searxng", "duckduckgo", "wikipedia", "brave", "google"]);
        assert_eq!(ids(&q("x", Focus::News), &cfg), ["searxng", "gdelt", "hackernews", "brave"]);
        // The settings decide which are off; GDELT starts off (see settings::load).
        assert!(!info(Id::Gdelt).default_on && info(Id::HackerNews).default_on);
        assert_eq!(ids(&q("x", Focus::Papers), &cfg), ["searxng", "openalex", "arxiv", "crossref"]);
        assert_eq!(ids(&q("x", Focus::Code), &cfg), ["searxng", "duckduckgo", "hackernews", "stackexchange", "github"]);
        let pdf = Query { filetype: Some(FileType::Pdf), ..q("x", Focus::Papers) };
        assert_eq!(ids(&pdf, &cfg), ["searxng", "openalex", "arxiv"]);
        let xlsx = Query { filetype: Some(FileType::Xlsx), ..q("x", Focus::Web) };
        assert_eq!(ids(&xlsx, &cfg), ["searxng", "duckduckgo", "brave", "google"]);
        let off = Config { off: vec!["duckduckgo".into(), "brave".into()], ..Config::default() };
        assert_eq!(ids(&q("x", Focus::Web), &off), ["searxng", "wikipedia", "google"]);
    }

    #[test]
    fn file_types_and_focus_are_read_leniently() {
        assert_eq!(FileType::parse(".PDF"), Some(FileType::Pdf));
        assert_eq!(FileType::parse("excel"), Some(FileType::Xlsx));
        assert_eq!(FileType::parse("exe"), None);
        assert!(FileType::Xlsx.matches_url("https://x.org/data/2025.XLS?dl=1"));
        assert!(!FileType::Pdf.matches_url("https://x.org/pdf-guide"));
        assert_eq!(Focus::parse("Science"), Some(Focus::Papers));
        assert_eq!(Focus::parse(""), Some(Focus::Web));
        assert_eq!(Focus::parse("images"), None);
    }

    #[test]
    fn the_report_names_what_answered_and_what_did_not() {
        let r = vec![
            (Id::DuckDuckGo, Ok(6)),
            (Id::Wikipedia, Ok(0)),
            (Id::SearXng, Err(Failure::NotRunning)),
            (Id::Brave, Err(Failure::Skipped)),
            (Id::Gdelt, Err(Failure::Limited(None))),
        ];
        assert_eq!(describe(&r), "DuckDuckGo 6, Wikipedia 0 · SearXNG not running, GDELT busy");
        assert_eq!(describe(&[(Id::Google, Err(Failure::Skipped))]), "no source answered");
    }
}
