//! Google's Programmable Search (Custom Search JSON API), with the user's own key and search
//! engine id (Settings › Olymp). Google limits free use per day.

use super::{Failure, Hit, Query, json, lang};
use crate::olymp::{Config, Olymp};

pub(super) async fn search(o: &Olymp, cfg: &Config, q: &Query) -> Result<Vec<Hit>, Failure> {
    let (Some(key), Some(cx)) =
        (cfg.google_key.as_deref().filter(|k| !k.is_empty()), cfg.google_cx.as_deref().filter(|c| !c.is_empty()))
    else {
        return Err(Failure::Skipped);
    };
    let mut params =
        vec![("key", key.to_string()), ("cx", cx.to_string()), ("q", q.text.clone()), ("num", "10".into())];
    if let Some(ft) = q.filetype {
        params.push(("fileType", ft.ext().into()));
    }
    if let Some(l) = lang(q) {
        params.push(("lr", format!("lang_{l}")));
    }
    let body = super::send(o.apis.get(&o.endpoints.google).query(&params)).await?;
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Hit> {
    let v = json(body);
    let Some(items) = v["items"].as_array() else { return vec![] };
    items
        .iter()
        .filter_map(|it| {
            let url = it["link"].as_str()?;
            Some(Hit::new(it["title"].as_str().unwrap_or(url), url, it["snippet"].as_str().unwrap_or(""), "Google"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_read() {
        let body = r#"{"items":[{"title":"Annual report","link":"https://x.example/r.pdf","snippet":"2025 figures"}]}"#;
        assert_eq!(parse(body)[0].url, "https://x.example/r.pdf");
        assert!(parse("{}").is_empty());
    }
}
