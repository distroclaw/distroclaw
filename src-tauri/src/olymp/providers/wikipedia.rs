//! Wikipedia's search API, in the user's language when it is English, German, French or Italian.

use super::{Failure, Hit, Query, lang};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let code = lang(q).filter(|l| matches!(*l, "en" | "de" | "fr" | "it")).unwrap_or("en");
    let api = o.endpoints.wikipedia.replace("{lang}", code);
    let req = o.apis.get(&api).query(&[
        ("action", "query"),
        ("list", "search"),
        ("srsearch", q.text.as_str()),
        ("format", "json"),
        ("srlimit", "6"),
        ("utf8", "1"),
    ]);
    let body = super::send(req).await?;
    Ok(crate::web::parse_wikipedia(&body, &api)
        .into_iter()
        .map(|h| Hit::new(h.title, h.url, h.snippet, "Wikipedia"))
        .collect())
}
