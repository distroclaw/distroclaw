//! Hacker News through Algolia's free search API: technology news and discussions.

use super::{Failure, Hit, Query, json};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let req =
        o.apis.get(&o.endpoints.hn).query(&[("query", q.text.as_str()), ("tags", "story"), ("hitsPerPage", "10")]);
    let body = super::send(req).await?;
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Hit> {
    let v = json(body);
    let Some(hits) = v["hits"].as_array() else { return vec![] };
    hits.iter()
        .filter_map(|h| {
            let title = h["title"].as_str()?;
            let id = h["objectID"].as_str().unwrap_or("");
            let discussion = format!("https://news.ycombinator.com/item?id={id}");
            let url = h["url"].as_str().filter(|u| u.starts_with("http")).map(str::to_string).unwrap_or(discussion);
            let snippet = format!(
                "{} points, {} comments on Hacker News",
                h["points"].as_u64().unwrap_or(0),
                h["num_comments"].as_u64().unwrap_or(0)
            );
            let date = h["created_at"].as_str().map(|d| d.chars().take(10).collect());
            Some(Hit::new(title, url, snippet, "Hacker News").dated(date))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stories_link_to_the_article_or_the_discussion() {
        let body = r#"{"hits":[
            {"title":"Rust 2.0","url":"https://blog.example/rust","objectID":"1","points":300,"num_comments":120,"created_at":"2026-11-01T08:00:00Z"},
            {"title":"Ask HN: tools?","url":null,"objectID":"2"}]}"#;
        let hits = parse(body);
        assert_eq!(hits[0].url, "https://blog.example/rust");
        assert_eq!(hits[0].date.as_deref(), Some("2026-11-01"));
        assert_eq!(hits[1].url, "https://news.ycombinator.com/item?id=2");
    }
}
