//! DuckDuckGo's HTML results, the same provider the app's online search uses. A bot check
//! pauses it; it is never worked around.

use super::{Failure, Hit, Query, lang, with_filetype};
use crate::olymp::Olymp;

pub(super) async fn search(o: &Olymp, q: &Query) -> Result<Vec<Hit>, Failure> {
    let text = with_filetype(q);
    let region = match lang(q) {
        Some("de") => "de-de",
        Some("fr") => "fr-fr",
        Some("it") => "it-it",
        Some("en") => "us-en",
        _ => "wt-wt",
    };
    let req = o
        .apis
        .get(&o.endpoints.ddg)
        .header(reqwest::header::USER_AGENT, crate::web::BROWSER_UA)
        .query(&[("q", text.as_str()), ("kl", region)]);
    let body = super::send(req).await?;
    if crate::web::is_bot_check(&body) {
        return Err(Failure::BotCheck);
    }
    let hits = crate::web::parse_ddg(&body, "result__a", "result__snippet")
        .into_iter()
        .map(|h| Hit::new(h.title, h.url, h.snippet, "DuckDuckGo"))
        .collect();
    Ok(hits)
}
