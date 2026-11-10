//! Crossref's REST API: metadata of published articles, books and reports, by DOI.

use super::{Failure, Hit, Query, json};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let req = o.apis.get(&o.endpoints.crossref).query(&[
        ("query", q.text.as_str()),
        ("rows", "8"),
        ("select", "DOI,title,URL,abstract,issued,container-title"),
    ]);
    let body = super::send(req).await?;
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Hit> {
    let v = json(body);
    let Some(items) = v["message"]["items"].as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|it| {
            let title = it["title"][0].as_str()?;
            let url = it["URL"].as_str()?;
            let year = it["issued"]["date-parts"][0][0].as_u64().map(|y| y.to_string());
            let venue = it["container-title"][0].as_str().unwrap_or("");
            // Abstracts come as JATS XML.
            let abstract_ = crate::web::inline_text(it["abstract"].as_str().unwrap_or(""));
            let snippet = [year.clone().unwrap_or_default(), venue.to_string(), abstract_]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            Some(Hit::new(title, url, snippet, "Crossref").dated(year))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_are_read_with_year_and_journal() {
        let body = r#"{"message":{"items":[{"DOI":"10.1/x","title":["Heat pumps"],"URL":"https://doi.org/10.1/x",
            "issued":{"date-parts":[[2024,5]]},"container-title":["Applied Energy"],"abstract":"<jats:p>Efficient.</jats:p>"}]}}"#;
        let hits = parse(body);
        assert_eq!(hits[0].snippet, "2024 · Applied Energy · Efficient.");
        assert_eq!(hits[0].date.as_deref(), Some("2024"));
    }
}
