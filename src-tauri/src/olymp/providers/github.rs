//! GitHub's repository search, without a token: ten searches a minute, which the registry keeps.

use super::{Failure, Hit, Query, json};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let req = o
        .apis
        .get(&o.endpoints.github)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .query(&[("q", q.text.as_str()), ("per_page", "8")]);
    let body = super::send(req).await?;
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Hit> {
    let v = json(body);
    let Some(items) = v["items"].as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|r| {
            let name = r["full_name"].as_str()?;
            let url = r["html_url"].as_str()?;
            let snippet = format!(
                "{} · {} stars",
                r["description"].as_str().unwrap_or("No description"),
                r["stargazers_count"].as_u64().unwrap_or(0)
            );
            let date = r["updated_at"].as_str().map(|d| d.chars().take(10).collect());
            Some(Hit::new(name, url, snippet, "GitHub").dated(date))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_are_read() {
        let body = r#"{"items":[{"full_name":"tafia/calamine","html_url":"https://github.com/tafia/calamine",
            "description":"Excel reader","stargazers_count":1900,"updated_at":"2026-10-30T00:00:00Z"}]}"#;
        let hits = parse(body);
        assert_eq!(hits[0].snippet, "Excel reader · 1900 stars");
        assert_eq!(hits[0].date.as_deref(), Some("2026-10-30"));
    }
}
