//! Brave Search API, with the user's own key (Settings › Olymp). Web and news results.

use super::{Failure, Focus, Hit, Query, json, lang, with_filetype};
use crate::olymp::{Config, Olymp};

pub(super) async fn search(o: &Olymp, cfg: &Config, q: &Query) -> Result<Vec<Hit>, Failure> {
    let Some(key) = cfg.brave_key.as_deref().filter(|k| !k.is_empty()) else { return Err(Failure::Skipped) };
    let news = q.focus == Focus::News;
    let url = format!("{}/{}/search", o.endpoints.brave.trim_end_matches('/'), if news { "news" } else { "web" });
    let text = with_filetype(q);
    let mut params = vec![("q", text.as_str()), ("count", "10")];
    if let Some(l) = lang(q) {
        params.push(("search_lang", l));
    }
    let req = o
        .apis
        .get(url)
        .header("X-Subscription-Token", key)
        .header(reqwest::header::ACCEPT, "application/json")
        .query(&params);
    let body = super::send(req).await?;
    Ok(parse(&body, news))
}

fn parse(body: &str, news: bool) -> Vec<Hit> {
    let v = json(body);
    let list = if news { &v["results"] } else { &v["web"]["results"] };
    let Some(results) = list.as_array() else { return vec![] };
    results
        .iter()
        .filter_map(|r| {
            let url = r["url"].as_str()?;
            let title = r["title"].as_str().unwrap_or(url);
            let snippet = crate::web::inline_text(r["description"].as_str().unwrap_or(""));
            let date = r["page_age"].as_str().map(|d| d.chars().take(10).collect());
            Some(Hit::new(title, url, snippet, "Brave Search").dated(date))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_news_results_are_read() {
        let web =
            r#"{"web":{"results":[{"title":"A","url":"https://a.example","description":"<strong>A</strong> page"}]}}"#;
        assert_eq!(parse(web, false)[0].snippet, "A page");
        let news = r#"{"results":[{"title":"N","url":"https://n.example","page_age":"2026-11-08T09:00:00"}]}"#;
        assert_eq!(parse(news, true)[0].date.as_deref(), Some("2026-11-08"));
    }
}
