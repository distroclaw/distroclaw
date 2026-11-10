//! SearXNG and Crawl4AI running on this computer. Their addresses come from Settings and must be
//! written as a loopback address (127.0.0.1 or [::1]); they are checked when saved and again
//! before every request, and this client never follows a redirect. So a changed setting can
//! only ever reach a service on this computer, never another machine.

use reqwest::Url;
use std::net::IpAddr;
use std::time::Duration;

/// Crawl4AI renders pages in its own browser, which takes a while.
pub const LOCAL_TIMEOUT: Duration = Duration::from_secs(40);

pub struct LocalClient {
    pub(crate) http: reqwest::Client,
}

impl LocalClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(LOCAL_TIMEOUT)
            .connect_timeout(Duration::from_secs(2))
            .user_agent(super::USER_AGENT)
            .build()
            .expect("the local client builds");
        Self { http }
    }
}

impl Default for LocalClient {
    fn default() -> Self {
        Self::new()
    }
}

/// `raw` as the base address of a local service, ending in `/`; refused unless it is http(s) on
/// 127.0.0.1 or [::1], without a user name, password, query or fragment.
pub fn validate(raw: &str) -> Result<Url, String> {
    let mut url = Url::parse(raw.trim()).map_err(|_| "Not a valid address.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Use an http address.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("The address cannot contain a user name or password.".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("The address cannot contain ? or #.".into());
    }
    // Written as an address, never a name: a name could be made to point elsewhere.
    let host = url.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']');
    let loopback = match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
        Ok(IpAddr::V6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
        Err(_) => false,
    };
    if !loopback {
        return Err("Only a service on this computer can be used: write the address as 127.0.0.1.".into());
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// Whether the local services answer, for the Test button in Settings.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub searxng: String,
    pub crawl4ai: String,
}

/// "ok", "off" (no address set), "not_running", "no_json" (SearXNG without its JSON format) or
/// "error".
pub async fn check(client: &LocalClient, cfg: &super::Config) -> Check {
    let searxng = match cfg.searxng.as_deref().map(validate) {
        None => "off".to_string(),
        Some(Err(_)) => "error".into(),
        Some(Ok(base)) => {
            let url = base.join("search").expect("a relative path joins");
            match client.http.get(url).query(&[("q", "test"), ("format", "json")]).send().await {
                Ok(r) if r.status().as_u16() == 403 => "no_json".into(),
                Ok(r) if r.status().is_success() => {
                    let json = r.json::<serde_json::Value>().await.is_ok_and(|v| v["results"].is_array());
                    if json { "ok".into() } else { "no_json".into() }
                }
                Ok(_) => "error".into(),
                Err(e) if e.is_connect() || e.is_timeout() => "not_running".into(),
                Err(_) => "error".into(),
            }
        }
    };
    let crawl4ai = match cfg.crawl4ai.as_deref().map(validate) {
        None => "off".to_string(),
        Some(Err(_)) => "error".into(),
        Some(Ok(base)) => {
            let url = base.join("health").expect("a relative path joins");
            match client.http.get(url).send().await {
                Ok(r) if r.status().is_success() => "ok".into(),
                Ok(_) => "error".into(),
                Err(e) if e.is_connect() || e.is_timeout() => "not_running".into(),
                Err(_) => "error".into(),
            }
        }
    };
    Check { searxng, crawl4ai }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeSite, Reply};

    #[tokio::test]
    async fn the_check_tells_running_missing_and_misconfigured_services_apart() {
        let site = FakeSite::start(|_| {
            vec![("/search", Reply::json(r#"{"results":[]}"#)), ("/health", Reply::json(r#"{"status":"ok"}"#))]
        })
        .await;
        let client = LocalClient::new();
        let cfg = super::super::Config {
            searxng: Some(site.url.clone()),
            crawl4ai: Some(site.url.clone()),
            ..Default::default()
        };
        assert_eq!(check(&client, &cfg).await, Check { searxng: "ok".into(), crawl4ai: "ok".into() });
        let html = FakeSite::start(|_| vec![("/search", Reply::html("<html>search page</html>"))]).await;
        let cfg = super::super::Config { searxng: Some(html.url.clone()), ..Default::default() };
        assert_eq!(check(&client, &cfg).await, Check { searxng: "no_json".into(), crawl4ai: "off".into() });
        // A port nothing listens on.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let cfg = super::super::Config { searxng: Some(format!("http://127.0.0.1:{free}")), ..Default::default() };
        assert_eq!(check(&client, &cfg).await.searxng, "not_running");
    }

    #[test]
    fn only_loopback_addresses_are_accepted() {
        assert_eq!(validate("http://127.0.0.1:8888").unwrap().as_str(), "http://127.0.0.1:8888/");
        assert_eq!(validate("http://[::1]:11235/base").unwrap().as_str(), "http://[::1]:11235/base/");
        for bad in [
            "http://10.0.0.5:8888",
            "http://192.168.1.2",
            "http://localhost:8888",
            "http://searx.example.org",
            "http://user@127.0.0.1:8888",
            "http://127.0.0.1:8888/?q=x",
            "file:///etc/passwd",
            "http://127.0.0.2:8888",
            "not a url",
        ] {
            assert!(validate(bad).is_err(), "{bad}");
        }
    }
}
