//! Olymp's settings. Each user switches sources on or off for themselves (`olymp_<provider>`;
//! unset means the source's default, which is on for all but GDELT). The addresses of a local
//! SearXNG and Crawl4AI and the search API keys belong to the installation: only an admin changes
//! them, the addresses must be loopback, and keys are never read back (asking for one only tells
//! whether it is set).

use super::Config;
use super::providers::ALL;
use crate::db::Db;
use crate::error::AppResult;

/// The address Olymp tries for SearXNG when none is set; Crawl4AI is off until set.
pub const DEFAULT_SEARXNG: &str = "http://127.0.0.1:8888";

pub const SEARXNG_URL: &str = "olymp_searxng_url";
pub const CRAWL4AI_URL: &str = "olymp_crawl4ai_url";
pub const GOOGLE_CX: &str = "olymp_google_cx";
/// Admin-only settings anyone may read.
pub const APP_KEYS: &[&str] = &[SEARXNG_URL, CRAWL4AI_URL, GOOGLE_CX];
/// Admin-only settings nobody reads back.
pub const SECRET_KEYS: &[&str] = &["olymp_brave_key", "olymp_google_key"];
/// Settings that hold an address of a service on this computer.
pub const LOCAL_URL_KEYS: &[&str] = &[SEARXNG_URL, CRAWL4AI_URL];

/// The per-user switch of a source: `olymp_duckduckgo`, `olymp_gdelt`…
pub fn is_source_key(key: &str) -> bool {
    key.strip_prefix("olymp_").is_some_and(|k| ALL.iter().any(|p| p.key == k))
}

/// An address setting: empty or "off" switches the service off; anything else must be a
/// loopback address.
pub fn check_local_url(value: &str) -> Result<(), String> {
    let v = value.trim();
    if v.is_empty() || v == "off" {
        return Ok(());
    }
    super::local::validate(v).map(|_| ())
}

/// Olymp's settings for `user_id`'s reply.
pub fn load(db: &Db, user_id: &str) -> AppResult<Config> {
    let mut off = Vec::new();
    for p in ALL {
        let on = match db.get_user_setting(user_id, &format!("olymp_{}", p.key))?.as_deref() {
            Some("off") => false,
            Some("on") => true,
            _ => p.default_on,
        };
        if !on {
            off.push(p.key.to_string());
        }
    }
    let local = |key: &str, default: Option<&str>| -> AppResult<Option<String>> {
        let v = db.get_setting(key)?;
        let v = match v.as_deref().map(str::trim) {
            None => default.map(str::to_string),
            Some("") | Some("off") => None,
            Some(v) => Some(v.to_string()),
        };
        // A value that is no longer valid counts as off.
        Ok(v.filter(|v| super::local::validate(v).is_ok()))
    };
    let secret = |key: &str| -> AppResult<Option<String>> {
        Ok(db.get_setting(key)?.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()))
    };
    Ok(Config {
        off,
        searxng: local(SEARXNG_URL, Some(DEFAULT_SEARXNG))?,
        crawl4ai: local(CRAWL4AI_URL, None)?,
        brave_key: secret("olymp_brave_key")?,
        google_key: secret("olymp_google_key")?,
        google_cx: secret(GOOGLE_CX)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_and_local_addresses_are_checked() {
        assert!(is_source_key("olymp_gdelt") && is_source_key("olymp_searxng"));
        assert!(!is_source_key("olymp_nope") && !is_source_key("gdelt"));
        assert!(check_local_url("").is_ok() && check_local_url("off").is_ok());
        assert!(check_local_url("http://127.0.0.1:8888").is_ok());
        assert!(check_local_url("http://192.168.1.4:8888").is_err());
    }
}
