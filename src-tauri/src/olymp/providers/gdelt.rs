//! GDELT's DOC 2.0 API: news articles from around the world, free and without a key. It asks
//! for about one request every five seconds and answers errors as plain text.

use super::{Failure, Hit, Query, json, lang};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let mut query = q.text.clone();
    if let Some(name) = match lang(q) {
        Some("en") => Some("english"),
        Some("de") => Some("german"),
        Some("fr") => Some("french"),
        Some("it") => Some("italian"),
        _ => None,
    } {
        query.push_str(&format!(" sourcelang:{name}"));
    }
    let req = o.apis.get(&o.endpoints.gdelt).query(&[
        ("query", query.as_str()),
        ("mode", "artlist"),
        ("format", "json"),
        ("maxrecords", "10"),
        ("sort", "hybridrel"),
    ]);
    let body = super::send(req).await?;
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Hit> {
    let v = json(body);
    let Some(articles) = v["articles"].as_array() else { return vec![] };
    articles
        .iter()
        .filter_map(|a| {
            let url = a["url"].as_str()?;
            let title = a["title"].as_str().unwrap_or(url);
            // "20261109T120000Z" -> "2026-11-09"
            let date = a["seendate"]
                .as_str()
                .filter(|d| d.len() >= 8)
                .map(|d| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8]));
            let snippet = format!("{} · {}", a["domain"].as_str().unwrap_or(""), a["language"].as_str().unwrap_or(""))
                .trim_matches([' ', '·'])
                .to_string();
            Some(Hit::new(title, url, snippet, "GDELT").dated(date))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn articles_are_read_and_text_errors_mean_none() {
        let body = r#"{"articles":[{"url":"https://news.example/a","title":"Rates cut","seendate":"20261109T120000Z","domain":"news.example","language":"English"}]}"#;
        let hits = parse(body);
        assert_eq!(hits[0].date.as_deref(), Some("2026-11-09"));
        assert_eq!(hits[0].snippet, "news.example · English");
        assert!(parse("Please limit requests to one every 5 seconds").is_empty());
    }
}
