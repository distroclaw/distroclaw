//! arXiv's API (Atom XML): preprints in physics, mathematics, computer science and more. It asks
//! for three seconds between requests, which the registry keeps.

use super::{Failure, FileType, Hit, Query};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    // Every meaningful word must appear somewhere in the paper's record.
    let words: Vec<String> = crate::service::web::terms(std::slice::from_ref(&q.text)).into_iter().take(6).collect();
    if words.is_empty() {
        return Ok(vec![]);
    }
    let search = words.iter().map(|w| format!("all:{w}")).collect::<Vec<_>>().join(" AND ");
    let req = o.apis.get(&o.endpoints.arxiv).query(&[
        ("search_query", search.as_str()),
        ("start", "0"),
        ("max_results", "8"),
        ("sortBy", "relevance"),
    ]);
    let body = super::send(req).await?;
    Ok(parse(&body, q.filetype == Some(FileType::Pdf)))
}

/// The text of the first `<tag>` in `xml`.
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let start = xml.find(&open)?;
    let body = start + xml[start..].find('>')? + 1;
    let end = body + xml[body..].find(&format!("</{name}>"))?;
    Some(&xml[body..end])
}

fn parse(xml: &str, pdf: bool) -> Vec<Hit> {
    xml.split("<entry>")
        .skip(1)
        .filter_map(|entry| {
            let abs = tag(entry, "id")?.trim().replacen("http://", "https://", 1);
            let title = crate::web::decode_entities(tag(entry, "title")?);
            let summary = crate::web::decode_entities(tag(entry, "summary").unwrap_or(""));
            let date = tag(entry, "published").map(|d| d.trim().chars().take(10).collect());
            let url = if pdf { abs.replacen("/abs/", "/pdf/", 1) } else { abs };
            let mut hit = Hit::new(title, url, summary, "arXiv").dated(date);
            hit.doc = pdf;
            Some(hit)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_read_from_the_atom_feed() {
        let xml = r#"<feed><entry><id>http://arxiv.org/abs/2411.00001v1</id><published>2026-11-01T00:00:00Z</published>
            <title>Error correction &amp; qubits</title><summary>We show a new code.</summary></entry></feed>"#;
        let hits = parse(xml, false);
        assert_eq!(hits[0].url, "https://arxiv.org/abs/2411.00001v1");
        assert_eq!(hits[0].title, "Error correction & qubits");
        assert_eq!(hits[0].date.as_deref(), Some("2026-11-01"));
        assert_eq!(parse(xml, true)[0].url, "https://arxiv.org/pdf/2411.00001v1");
    }
}
