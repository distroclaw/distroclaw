//! OpenAlex: an open index of scholarly works, with links to open-access PDFs. Abstracts come
//! as an inverted index (word -> positions) and are put back together here.

use super::{Failure, FileType, Hit, Query, json};
use crate::olymp::Olymp;
use serde_json::Value;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let mut params = vec![("search", q.text.clone()), ("per-page", "8".to_string())];
    let pdf = q.filetype == Some(FileType::Pdf);
    if pdf {
        params.push(("filter", "is_oa:true".to_string()));
    }
    let body = super::send(o.apis.get(&o.endpoints.openalex).query(&params)).await?;
    Ok(parse(&body, pdf))
}

fn parse(body: &str, pdf: bool) -> Vec<Hit> {
    let v = json(body);
    let Some(works) = v["results"].as_array() else { return vec![] };
    works
        .iter()
        .filter_map(|w| {
            let title = w["display_name"].as_str().or(w["title"].as_str())?;
            let pdf_url = w["best_oa_location"]["pdf_url"].as_str().or(w["primary_location"]["pdf_url"].as_str());
            let page = w["doi"]
                .as_str()
                .or(w["primary_location"]["landing_page_url"].as_str())
                .or(w["open_access"]["oa_url"].as_str())
                .or(w["id"].as_str())?;
            let url = if pdf { pdf_url? } else { page };
            let year = w["publication_year"].as_u64().map(|y| y.to_string());
            let venue = w["primary_location"]["source"]["display_name"].as_str().unwrap_or("");
            let abstract_ = abstract_text(&w["abstract_inverted_index"]);
            let snippet = [year.clone().unwrap_or_default(), venue.to_string(), abstract_]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let mut hit = Hit::new(title, url, snippet, "OpenAlex").dated(year);
            hit.doc = pdf;
            Some(hit)
        })
        .collect()
}

/// The abstract from OpenAlex's inverted index, cut to a snippet.
fn abstract_text(index: &Value) -> String {
    let Some(map) = index.as_object() else { return String::new() };
    let mut words: Vec<(u64, &str)> = Vec::new();
    for (word, positions) in map {
        for p in positions.as_array().into_iter().flatten().filter_map(Value::as_u64) {
            if p < 2_000 {
                words.push((p, word));
            }
        }
    }
    words.sort();
    words.iter().take(60).map(|(_, w)| *w).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_and_their_abstracts_are_read() {
        let body = r#"{"results":[{"display_name":"Grid storage at scale","doi":"https://doi.org/10.1/x",
            "publication_year":2025,"primary_location":{"source":{"display_name":"Energy Journal"}},
            "best_oa_location":{"pdf_url":"https://repo.example/x.pdf"},
            "abstract_inverted_index":{"storage":[1],"Grid":[0],"matters.":[2]}},
            {"display_name":"No PDF","doi":"https://doi.org/10.1/y"}]}"#;
        let hits = parse(body, false);
        assert_eq!(hits[0].url, "https://doi.org/10.1/x");
        assert_eq!(hits[0].snippet, "2025 · Energy Journal · Grid storage matters.");
        let pdfs = parse(body, true);
        assert_eq!((pdfs.len(), pdfs[0].url.as_str(), pdfs[0].doc), (1, "https://repo.example/x.pdf", true));
    }
}
