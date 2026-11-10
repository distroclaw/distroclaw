//! Stack Exchange's API (Stack Overflow): questions and whether they were answered. Its answers
//! are always gzip-compressed; a `backoff` in the answer pauses the provider as asked.

use super::{Failure, Hit, Query, json};
use crate::olymp::Olymp;
use std::time::Duration;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let req = o.apis.get(&o.endpoints.stackexchange).query(&[
        ("order", "desc"),
        ("sort", "relevance"),
        ("q", q.text.as_str()),
        ("site", "stackoverflow"),
        ("pagesize", "8"),
    ]);
    let body = super::send(req).await?;
    let v = json(&body);
    if let Some(secs) = v["backoff"].as_u64() {
        // The results still count; the next request waits.
        o.pause("stackexchange", Duration::from_secs(secs));
    }
    if v["quota_remaining"].as_u64() == Some(0) {
        o.pause("stackexchange", Duration::from_secs(3600));
    }
    Ok(parse(&v))
}

fn parse(v: &serde_json::Value) -> Vec<Hit> {
    let Some(items) = v["items"].as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|it| {
            let title = crate::web::decode_entities(it["title"].as_str()?);
            let url = it["link"].as_str()?;
            let tags: Vec<&str> = it["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str()).collect();
            let answered = if it["is_answered"].as_bool() == Some(true) { "answered" } else { "not answered yet" };
            let snippet = format!("{answered}, score {} · {}", it["score"].as_i64().unwrap_or(0), tags.join(", "));
            Some(Hit::new(title, url, snippet, "Stack Exchange"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_are_read_with_their_state() {
        let v = json(
            r#"{"items":[{"title":"How to read &quot;xlsx&quot; in Rust?","link":"https://stackoverflow.com/q/1",
               "is_answered":true,"score":12,"tags":["rust","excel"]}],"quota_remaining":290}"#,
        );
        let hits = parse(&v);
        assert_eq!(hits[0].title, "How to read \"xlsx\" in Rust?");
        assert_eq!(hits[0].snippet, "answered, score 12 · rust, excel");
    }
}
