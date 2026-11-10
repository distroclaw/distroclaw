//! Olymp's two tools. `olymp_search` asks every chosen source at once and returns one fused,
//! numbered list; `olymp_read` opens a result (a page or a public document) by its number. Only
//! results of this reply and links the user wrote can be opened, so no page or file can make the
//! model send text to an address of its choosing. After a local file was opened in the reply,
//! both refuse: the web is closed.

use super::fetch::{self, Kind};
use super::providers::{self, FileType, Focus, Query};
use super::{
    LOCAL_CLOSED, MAX_READS, MAX_SEARCHES, PART_CHARS, RESULTS_PER_SEARCH, Session, TOOL_TIMEOUT, UNTRUSTED, fuse,
};
use crate::service::web::{excerpt, relevant, terms};
use crate::tools::{MAX_RESULT_CHARS, ToolKind, ToolOutcome, ToolSpec, truncate_result};
use serde_json::{Value, json};

pub const SEARCH: &str = "olymp_search";
pub const READ: &str = "olymp_read";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: SEARCH,
            description: "Search many sources at once: the web, news, research papers or developer sources. \
                With filetype, finds public documents (PDF, Word, Excel, PowerPoint, CSV). Returns numbered \
                results; cite them as [n] and open one with olymp_read.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "A few keywords: names, places, products, titles, dates. For documents, words from the title or file name." },
                    "focus": { "type": "string", "enum": ["web", "news", "papers", "code"], "description": "web (default), news, papers (research) or code (programming)" },
                    "filetype": { "type": "string", "enum": ["any", "pdf", "docx", "xlsx", "pptx", "csv"], "description": "any (default) for web pages; a type only when the user wants documents of that type" },
                    "lang": { "type": "string", "description": "Two-letter language of the results, e.g. en, de, fr, it" }
                },
                "required": ["query"]
            }),
            feature: None,
            kind: ToolKind::Research,
        },
        ToolSpec {
            name: READ,
            description: "Open a result from olymp_search by its number (or a link the user wrote) and read it: \
                web pages and PDF, Word, Excel or PowerPoint files. Long documents come in parts.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "The result number, like 3, or the full address" },
                    "part": { "type": "integer", "description": "For long documents: which part to read, from 1" }
                },
                "required": ["source"]
            }),
            feature: None,
            kind: ToolKind::Research,
        },
    ]
}

fn ok(result: String, summary: String) -> ToolOutcome {
    ToolOutcome::Ok { result: truncate_result(result, MAX_RESULT_CHARS), summary, images: vec![] }
}

fn err(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome::Err { message: message.into() }
}

/// Runs an Olymp tool; without a session (Online search off) it says so.
pub async fn run(session: Option<&Session>, name: &str, args: &Value) -> ToolOutcome {
    let Some(s) = session else {
        return err("Online search is off. The user can turn it on with Search in the message box.");
    };
    if s.local_touched() {
        return err(LOCAL_CLOSED);
    }
    let work = async {
        match name {
            SEARCH => search(s, args).await,
            READ => read(s, args).await,
            _ => err(format!("Unknown tool {name}.")),
        }
    };
    tokio::select! {
        _ = s.cancel.cancelled() => err("Stopped."),
        r = tokio::time::timeout(TOOL_TIMEOUT, work) => r.unwrap_or_else(|_| err("The sources took too long to answer.")),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) async fn search(s: &Session, args: &Value) -> ToolOutcome {
    let Some(text) = arg_str(args, "query") else { return err("Invalid arguments: give a query.") };
    let text: String = text.chars().take(200).collect();
    let Some(focus) = Focus::parse(arg_str(args, "focus").unwrap_or("web")) else {
        return err("focus must be web, news, papers or code.");
    };
    // Small models fill every field; a value that means "no particular type" is no filter.
    let filetype = match arg_str(args, "filetype") {
        None => None,
        Some(f) if FileType::is_any(f) => None,
        Some(f) => match FileType::parse(f) {
            Some(ft) => Some(ft),
            None => return err("filetype must be any, pdf, docx, xlsx, pptx or csv."),
        },
    };
    let lang = arg_str(args, "lang").map(|l| l.to_ascii_lowercase());
    {
        let mut st = s.state.lock().unwrap();
        if st.searches >= MAX_SEARCHES {
            return err("The search limit for this reply is reached; answer with what you found.");
        }
        st.searches += 1;
        st.queries.push(text.clone());
    }
    let q = Query { text: text.clone(), focus, filetype, lang: lang.clone() };
    let (mut hits, mut report) = find(s, &q).await;
    let asked = match filetype {
        Some(ft) => format!("{} files", ft.label()),
        None => focus.label().to_string(),
    };
    let mut what = asked.clone();
    let mut filetype = filetype;
    // Nothing in documents, news, papers or code: the same words on the web, in the same step, so
    // a model that picked the wrong kind still gets something to read.
    if hits.is_empty() && (filetype.is_some() || focus != Focus::Web) && !s.cancel.is_cancelled() {
        let web = Query { text: text.clone(), focus: Focus::Web, filetype: None, lang };
        let (more, more_report) = find(s, &web).await;
        if !more.is_empty() {
            hits = more;
            report = format!("{report}; on the web: {more_report}");
            what = format!("web pages (nothing in {asked})");
            filetype = None;
        }
    }
    if hits.is_empty() {
        return ok(
            format!(
                "Nothing relevant was found in {what} for \"{text}\" ({report}). Try other words or another focus."
            ),
            format!("Nothing found in {what} for “{text}” · {report}"),
        );
    }
    let mut body = format!("Results in {what} for \"{text}\". {UNTRUSTED}\n\n");
    for h in &hits {
        let n = s.number(&h.title, &h.url, &h.snippet);
        let mut meta: Vec<String> = vec![h.from.join(", ")];
        if let Some(d) = &h.date {
            meta.push(d.clone());
        }
        if let Some(ft) = filetype {
            meta.push(ft.label().into());
        }
        body.push_str(&format!("[{n}] {} ({})\nURL: {}\n{}\n\n", h.title, meta.join(" · "), h.url, h.snippet));
    }
    body.push_str(&format!("Sources asked: {report}. Open a result with olymp_read when the snippet is not enough."));
    let summary = format!(
        "Found {} result{} in {what} for “{text}” · {report}",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" }
    );
    ok(body, summary)
}

/// One search across the sources: fused, on topic, documents of the asked type only.
async fn find(s: &Session, q: &Query) -> (Vec<providers::Hit>, String) {
    let outcome = providers::search_all(&s.olymp, &s.config, q).await;
    let report = providers::describe(&outcome.report);
    let mut hits = fuse::fuse(outcome.lists, RESULTS_PER_SEARCH * 3);
    // Off-topic results (a source guessing) are worse than none; documents must be of the type.
    let words = terms(std::slice::from_ref(&q.text));
    hits.retain(|h| {
        let as_web = crate::web::Hit { title: h.title.clone(), url: h.url.clone(), snippet: h.snippet.clone() };
        relevant(&as_web, &words) || h.doc
    });
    if let Some(ft) = q.filetype {
        hits.retain(|h| h.doc || ft.matches_url(&h.url));
    }
    hits.truncate(RESULTS_PER_SEARCH);
    (hits, report)
}

/// The number or address the model gave.
enum Target {
    Number(u32),
    Url(String),
}

fn target(args: &Value) -> Option<Target> {
    match args.get("source")? {
        Value::Number(n) => n.as_u64().map(|n| Target::Number(n as u32)),
        Value::String(s) => {
            let s = s.trim().trim_start_matches('[').trim_end_matches(']');
            match s.parse::<u32>() {
                Ok(n) => Some(Target::Number(n)),
                Err(_) if !s.is_empty() => Some(Target::Url(s.to_string())),
                Err(_) => None,
            }
        }
        _ => None,
    }
}

pub(crate) async fn read(s: &Session, args: &Value) -> ToolOutcome {
    let Some(target) = target(args) else { return err("Invalid arguments: give the result number as source.") };
    let part = args
        .get("part")
        .and_then(|p| p.as_u64().or_else(|| p.as_str()?.trim().parse().ok()))
        .map(|p| p.max(1) as usize);
    let (known, queries) = {
        let st = s.state.lock().unwrap();
        let known = match &target {
            Target::Number(n) => st.sources.iter().find(|x| x.n == *n).cloned(),
            Target::Url(u) => {
                let key = fuse::canonical(u);
                st.sources.iter().find(|x| fuse::canonical(&x.url) == key).cloned()
            }
        };
        (known, st.queries.clone())
    };
    let url = match (&known, &target) {
        (Some(k), _) => k.url.clone(),
        (None, Target::Url(u)) if s.user_links.iter().any(|l| fuse::canonical(l) == fuse::canonical(u)) => u.clone(),
        (None, Target::Number(n)) => return err(format!("There is no result [{n}] in this reply. Search first.")),
        (None, Target::Url(_)) => {
            return err(
                "Only results from olymp_search in this reply, or links the user wrote, can be opened. Search first.",
            );
        }
    };
    {
        let mut st = s.state.lock().unwrap();
        if st.reads >= MAX_READS {
            return err("The reading limit for this reply is reached; answer with what you found.");
        }
        st.reads += 1;
    }
    let page = match fetch::read(s, &url).await {
        Ok(p) => p,
        Err(e) => return err(format!("It could not be read: {e} Use the snippet or another result.")),
    };
    let title = known.as_ref().map(|k| k.title.clone()).or(page.title.clone()).unwrap_or_else(|| fuse::site(&page.url));
    let snippet =
        s.state.lock().unwrap().found.iter().find(|f| f.url == url).map(|f| f.snippet.clone()).unwrap_or_default();
    let n = known.as_ref().map(|k| k.n).unwrap_or_else(|| s.number(&title, &url, &snippet));
    let total = page.text.chars().count();
    let parts = total.div_ceil(PART_CHARS).max(1);
    let (text, which) = match part {
        Some(p) if p <= parts => {
            let chunk: String = page.text.chars().skip((p - 1) * PART_CHARS).take(PART_CHARS).collect();
            (chunk, format!("part {p} of {parts}"))
        }
        Some(p) => return err(format!("It has {parts} part{}, not {p}.", if parts == 1 { "" } else { "s" })),
        None if parts > 1 => (
            excerpt(&page.text, &terms(&queries), PART_CHARS),
            format!("the best-matching text of {parts} parts; read a part with part=1…{parts}"),
        ),
        None => (page.text.clone(), "the whole text".into()),
    };
    let kind = page.kind.label();
    let body = format!("[{n}] {title} ({kind}, {which})\nURL: {}\n{UNTRUSTED}\n\n{text}", page.url);
    let summary = if page.kind == Kind::Page {
        format!("Read {}", fuse::site(&page.url))
    } else {
        format!("Read {} ({kind})", fuse::site(&page.url))
    };
    ok(body, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::olymp::tests::olymp_at;
    use crate::olymp::{Config, Session};
    use crate::service::chat::Cancel;
    use crate::testing::{FakeSite, Reply};
    use std::sync::Arc;

    fn enc(url: &str) -> String {
        url.replace(':', "%3A").replace('/', "%2F")
    }

    fn text(o: ToolOutcome) -> String {
        match o {
            ToolOutcome::Ok { result, .. } => result,
            ToolOutcome::Err { message } => format!("ERR {message}"),
            ToolOutcome::Proposal { .. } => unreachable!(),
        }
    }

    /// A fake web with DuckDuckGo, Wikipedia and OpenAlex answering, and pages to read.
    pub(crate) async fn world() -> FakeSite {
        FakeSite::start(|base| {
            let ddg = format!(
                r#"<a class="result__a" href="//duckduckgo.com/l/?uddg={a}">Heat pump subsidies 2026</a>
                   <a class="result__snippet">Heat pump subsidies in Basel for 2026.</a>
                   <a class="result__a" href="//duckduckgo.com/l/?uddg={pdf}">Heat pump guide (PDF)</a>
                   <a class="result__snippet">A heat pump guide.</a>
                   <a class="result__a" href="//duckduckgo.com/l/?uddg={off}">Cooking pasta</a>
                   <a class="result__snippet">Boil water.</a>"#,
                a = enc(&format!("{base}/subsidies")),
                pdf = enc(&format!("{base}/guide.pdf")),
                off = enc(&format!("{base}/pasta")),
            );
            let wiki = r#"{"query":{"search":[{"title":"Heat pump","snippet":"A <b>heat pump</b> moves heat."}]}}"#;
            let openalex = format!(
                r#"{{"results":[{{"display_name":"Heat pumps in cold climates","doi":"{base}/paper","publication_year":2025}}]}}"#
            );
            vec![
                ("/ddg/html/", Reply::html(ddg)),
                ("/wiki/en/w/api.php", Reply::json(wiki)),
                ("/openalex", Reply::json(openalex)),
                ("/subsidies", Reply::html("<html><head><title>Subsidies 2026</title></head><main><p>Basel pays CHF 4'000 for a heat pump in 2026.</p></main></html>")),
                ("/guide.pdf", Reply::text("text/plain", "Heat pump guide.\nChapter 1: sizing.")),
                ("/paper", Reply::html("<main><p>Heat pumps work down to -20 degrees in this study.</p></main>")),
            ]
        })
        .await
    }

    pub(crate) fn session_for(site: &FakeSite, links: Vec<String>) -> Session {
        let cfg = Config { searxng: None, ..Config::default() };
        Session::new(Arc::new(olymp_at(&site.url)), cfg, links, Arc::new(Cancel::default()))
    }

    #[tokio::test]
    async fn search_fuses_sources_numbers_results_and_reads_them() {
        let site = world().await;
        let s = session_for(&site, vec![]);
        let found = text(run(Some(&s), SEARCH, &json!({ "query": "heat pump subsidies" })).await);
        assert!(found.contains("[1] Heat pump subsidies 2026 (DuckDuckGo)"), "{found}");
        assert!(found.contains("Wikipedia") && found.contains("Untrusted"), "{found}");
        assert!(!found.contains("pasta"), "off-topic results are dropped");
        assert!(found.contains("Sources asked: DuckDuckGo 3, Wikipedia 1"), "{found}");

        let page = text(run(Some(&s), READ, &json!({ "source": "1" })).await);
        assert!(page.starts_with("[1] Heat pump subsidies 2026 (page, the whole text)"), "{page}");
        assert!(page.contains("CHF 4'000"));
        // A made-up address is refused without a request.
        let before = site.hits().len();
        let made_up = text(run(Some(&s), READ, &json!({ "source": format!("{}/collect?d=x", site.url) })).await);
        assert!(made_up.starts_with("ERR Only results"), "{made_up}");
        assert!(text(run(Some(&s), READ, &json!({ "source": 42 })).await).contains("no result [42]"));
        assert_eq!(site.hits().len(), before);
    }

    #[tokio::test]
    async fn papers_and_documents_have_their_own_sources() {
        let site = world().await;
        let s = session_for(&site, vec![]);
        let papers = text(run(Some(&s), SEARCH, &json!({ "query": "heat pumps cold", "focus": "papers" })).await);
        assert!(papers.contains("Heat pumps in cold climates (OpenAlex · 2025)"), "{papers}");
        let pdfs = text(run(Some(&s), SEARCH, &json!({ "query": "heat pump guide", "filetype": "pdf" })).await);
        assert!(pdfs.contains("guide.pdf") && !pdfs.contains("/subsidies"), "only PDF files: {pdfs}");
        assert!(site.hits().iter().any(|h| h.contains("filetype%3Apdf")), "the operator went to DuckDuckGo");
        assert!(
            text(run(Some(&s), SEARCH, &json!({ "query": "x", "filetype": "exe" })).await).starts_with("ERR filetype")
        );
        // No spreadsheet about subsidies: the same words on the web, in the same step.
        let sheets = text(run(Some(&s), SEARCH, &json!({ "query": "heat pump subsidies", "filetype": "xlsx" })).await);
        assert!(sheets.contains("web pages (nothing in Excel files)") && sheets.contains("/subsidies"), "{sheets}");
        assert_eq!(s.state.lock().unwrap().searches, 3, "the fallback is part of the same search");
    }

    #[tokio::test]
    async fn limits_the_closed_web_and_links_the_user_wrote() {
        let site = world().await;
        let s = session_for(&site, vec![format!("{}/subsidies", site.url)]);
        let user = text(run(Some(&s), READ, &json!({ "source": format!("{}/subsidies", site.url) })).await);
        assert!(user.starts_with("[1] Subsidies 2026"), "a link the user wrote is read and numbered: {user}");
        for i in 0..MAX_SEARCHES {
            assert!(
                !text(run(Some(&s), SEARCH, &json!({ "query": format!("heat pump {i}") })).await).starts_with("ERR")
            );
        }
        assert!(text(run(Some(&s), SEARCH, &json!({ "query": "heat pump more" })).await).contains("limit"));
        s.mark_local();
        let before = site.hits().len();
        assert!(text(run(Some(&s), READ, &json!({ "source": 1 })).await).contains("Web research is closed"));
        assert_eq!(site.hits().len(), before, "nothing left the computer after a local file was opened");
        assert!(text(run(None, SEARCH, &json!({ "query": "x" })).await).contains("Online search is off"));
    }

    #[tokio::test]
    async fn long_documents_come_in_parts() {
        let long = (0..3000).map(|i| format!("Line {i} about heat pumps")).collect::<Vec<_>>().join("\n");
        let site = FakeSite::start(move |_| vec![("/long.txt", Reply::text("text/plain", long))]).await;
        let s = session_for(&site, vec![format!("{}/long.txt", site.url)]);
        let url = format!("{}/long.txt", site.url);
        let first = text(run(Some(&s), READ, &json!({ "source": url, "part": 1 })).await);
        assert!(first.contains("part 1 of") && first.contains("Line 0 about"), "{}", &first[..200]);
        let too_far = text(run(Some(&s), READ, &json!({ "source": url, "part": 999 })).await);
        assert!(too_far.starts_with("ERR It has"), "{too_far}");
    }
}
