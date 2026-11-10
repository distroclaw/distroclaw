//! Turns the result lists of many providers into one: the same page found twice counts once
//! (addresses are compared after cleanup), pages several providers found rise (reciprocal-rank
//! fusion), and no single site takes more than two places.

use super::providers::Hit;
use reqwest::Url;
use std::collections::HashMap;

/// The usual constant of reciprocal-rank fusion: a high rank in one list matters, but agreement
/// between lists matters more.
const RRF_K: f64 = 60.0;
/// Places one site may take in one result list.
const PER_SITE: usize = 2;

/// Query parameters that only track the click.
fn tracking(param: &str) -> bool {
    let p = param.to_ascii_lowercase();
    p.starts_with("utm_")
        || matches!(p.as_str(), "fbclid" | "gclid" | "dclid" | "msclkid" | "mc_cid" | "mc_eid" | "yclid" | "igshid")
}

/// The address of a page as compared: lower-case host without `www.`, no fragment, no tracking
/// parameters, sorted query, no trailing slash; arXiv's abstract and PDF pages and DOI links are
/// the same work.
pub fn canonical(raw: &str) -> String {
    let Ok(mut url) = Url::parse(raw.trim()) else { return raw.trim().to_string() };
    url.set_fragment(None);
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host == "doi.org" || host == "dx.doi.org" {
        return format!("doi:{}", url.path().trim_start_matches('/').to_ascii_lowercase());
    }
    if host.ends_with("arxiv.org") {
        let path = url.path();
        if let Some(id) = path.strip_prefix("/abs/").or_else(|| path.strip_prefix("/pdf/")) {
            return format!("arxiv:{}", id.trim_end_matches(".pdf").trim_end_matches('/'));
        }
    }
    let mut pairs: Vec<(String, String)> =
        url.query_pairs().filter(|(k, _)| !tracking(k)).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    pairs.sort();
    let query = if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&"))
    };
    let path = url.path().trim_end_matches('/');
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{host}{port}{path}{query}")
}

/// The site a page belongs to: the last two labels of its host, or three for country domains
/// such as `co.uk` or `com.au`. Close enough to tell sites apart without a suffix list.
pub fn site(raw: &str) -> String {
    let host = Url::parse(raw).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    if n <= 2 {
        return host;
    }
    let second = labels[n - 2];
    let country = labels[n - 1].len() == 2 && matches!(second, "co" | "com" | "org" | "net" | "gov" | "edu" | "ac");
    labels[n - if country { 3 } else { 2 }..].join(".")
}

/// One list from many, best first, at most `max` results and `PER_SITE` per site.
pub fn fuse(lists: Vec<Vec<Hit>>, max: usize) -> Vec<Hit> {
    let mut merged: Vec<(f64, Hit)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for list in lists {
        for (rank, hit) in list.into_iter().enumerate() {
            let score = 1.0 / (RRF_K + rank as f64 + 1.0);
            let key = canonical(&hit.url);
            match index.get(&key) {
                Some(&i) => {
                    let (s, existing) = &mut merged[i];
                    *s += score;
                    for f in hit.from {
                        if !existing.from.contains(&f) {
                            existing.from.push(f);
                        }
                    }
                    if hit.snippet.len() > existing.snippet.len() {
                        existing.snippet = hit.snippet;
                    }
                    if existing.date.is_none() {
                        existing.date = hit.date;
                    }
                    existing.doc |= hit.doc;
                }
                None => {
                    index.insert(key, merged.len());
                    merged.push((score, hit));
                }
            }
        }
    }
    merged.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut per_site: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for (_, hit) in merged {
        let count = per_site.entry(site(&hit.url)).or_default();
        if *count >= PER_SITE {
            continue;
        }
        *count += 1;
        out.push(hit);
        if out.len() >= max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str, from: &'static str) -> Hit {
        Hit { title: url.into(), url: url.into(), snippet: String::new(), date: None, from: vec![from], doc: false }
    }

    #[test]
    fn addresses_are_compared_after_cleanup() {
        assert_eq!(
            canonical("https://WWW.Example.org/a/?utm_source=x&b=2&a=1#top"),
            canonical("http://example.org/a?a=1&b=2&fbclid=z")
        );
        assert_ne!(canonical("https://example.org/a?id=1"), canonical("https://example.org/a?id=2"));
        assert_eq!(canonical("https://arxiv.org/pdf/2401.00001v2.pdf"), canonical("http://arxiv.org/abs/2401.00001v2"));
        assert_eq!(canonical("https://doi.org/10.1000/ABC"), canonical("https://dx.doi.org/10.1000/abc"));
    }

    #[test]
    fn sites_are_told_apart() {
        assert_eq!(site("https://news.bbc.co.uk/x"), "bbc.co.uk");
        assert_eq!(site("https://docs.python.org/3/"), "python.org");
        assert_eq!(site("https://www.admin.ch/gov/de"), "admin.ch");
        assert_eq!(site("https://example.com"), "example.com");
    }

    #[test]
    fn agreement_ranks_first_and_one_site_takes_two_places() {
        let a = vec![hit("https://one.org/x", "A"), hit("https://two.org/y", "A"), hit("https://big.com/1", "A")];
        let b = vec![
            hit("https://big.com/2", "B"),
            hit("https://big.com/3", "B"),
            hit("https://www.two.org/y/", "B"),
            hit("https://big.com/4", "B"),
        ];
        let fused = fuse(vec![a, b], 10);
        assert_eq!(fused[0].url, "https://two.org/y", "found by both lists");
        assert_eq!(fused[0].from, vec!["A", "B"]);
        assert_eq!(fused.iter().filter(|h| site(&h.url) == "big.com").count(), 2);
        assert_eq!(fused.len(), 4);
        assert_eq!(fuse(vec![], 5).len(), 0);
    }
}
