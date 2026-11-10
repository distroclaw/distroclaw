//! A SearXNG on this computer: one query reaches the engines it has enabled (Google, Bing,
//! Yahoo, Startpage, Brave, Qwant, Mojeek, DuckDuckGo, Wikipedia…). Needs the JSON format
//! switched on in its settings.yml (see docs/OLYMP.md).

use super::{Failure, Focus, Hit, Query, json, lang, with_filetype};
use crate::olymp::{Config, Olymp, local};

pub(super) async fn search(o: &Olymp, cfg: &Config, q: &Query) -> Result<Vec<Hit>, Failure> {
    let Some(base) = cfg.searxng.as_deref() else { return Err(Failure::Skipped) };
    let base = local::validate(base).map_err(Failure::Other)?;
    let url = base.join("search").map_err(|e| Failure::Other(e.to_string()))?;
    let category = match q.focus {
        Focus::Web => "general",
        Focus::News => "news",
        Focus::Papers => "science",
        Focus::Code => "it",
    };
    let text = with_filetype(q);
    let req = o.local.http.get(url).query(&[
        ("q", text.as_str()),
        ("format", "json"),
        ("categories", category),
        ("language", lang(q).unwrap_or("all")),
        ("safesearch", "1"),
    ]);
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) if e.is_connect() || e.is_timeout() => return Err(Failure::NotRunning),
        Err(e) => return Err(Failure::Other(e.to_string())),
    };
    let status = resp.status().as_u16();
    if status == 403 {
        return Err(Failure::Other("SearXNG refuses JSON: add json to search.formats in its settings.yml".into()));
    }
    if status == 429 {
        return Err(Failure::Limited(None));
    }
    if !resp.status().is_success() {
        return Err(Failure::Other(format!("status {status}")));
    }
    let body = resp.text().await.map_err(|e| Failure::Other(e.to_string()))?;
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Hit> {
    let v = json(body);
    let Some(results) = v["results"].as_array() else { return vec![] };
    results
        .iter()
        .filter_map(|r| {
            let url = r["url"].as_str()?;
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return None;
            }
            let title = r["title"].as_str().unwrap_or(url);
            let date = r["publishedDate"].as_str().map(|d| d.chars().take(10).collect());
            Some(Hit::new(title, url, r["content"].as_str().unwrap_or(""), "SearXNG").dated(date))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_are_read_from_searxng_json() {
        let body = r#"{"query":"x","results":[
            {"url":"https://example.org/a","title":"A page","content":"About <b>a</b>.","engine":"google","publishedDate":"2026-11-02T10:00:00"},
            {"url":"javascript:alert(1)","title":"bad"},
            {"url":"https://example.org/b.pdf","title":"B report","engines":["bing"]}]}"#;
        let hits = parse(body);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].title.as_str(), hits[0].date.as_deref()), ("A page", Some("2026-11-02")));
        assert_eq!(hits[1].url, "https://example.org/b.pdf");
        assert!(parse("not json").is_empty());
    }
}
