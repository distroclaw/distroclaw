//! robots.txt, as RFC 9309 describes it: the group for Olymp's product token, else the `*`
//! group; the longest matching rule decides, Allow wins a tie; `*` and a final `$` in rules;
//! Crawl-delay is honoured as a pace. A robots.txt that answers 4xx allows everything; one that
//! cannot be read (5xx, no answer) allows nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Largest robots.txt read (RFC 9309 asks crawlers to read at least 500 KiB).
pub const MAX_BYTES: usize = 500 * 1024;
/// How long a robots.txt is kept; an unreadable one is asked for again sooner.
const KEEP: Duration = Duration::from_secs(24 * 3600);
const KEEP_FAILED: Duration = Duration::from_secs(3600);
/// A Crawl-delay above this is treated as this (the fetcher then skips the host).
const MAX_DELAY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Robots {
    /// (allow, pattern)
    rules: Vec<(bool, String)>,
    pub delay: Option<Duration>,
    /// Set when the file could not be read: nothing is allowed.
    deny_all: bool,
}

impl Robots {
    pub fn allow_all() -> Self {
        Self::default()
    }

    pub fn deny_all() -> Self {
        Self { deny_all: true, ..Self::default() }
    }

    /// Whether the path (with its query) may be read.
    pub fn allows(&self, path: &str) -> bool {
        if path == "/robots.txt" {
            return true;
        }
        if self.deny_all {
            return false;
        }
        let mut best: Option<(usize, bool)> = None;
        for (allow, pattern) in &self.rules {
            if !matches(pattern, path) {
                continue;
            }
            let len = pattern.len();
            best = match best {
                Some((l, a)) if l > len || (l == len && a) => Some((l, a)),
                _ => Some((len, *allow)),
            };
        }
        best.is_none_or(|(_, allow)| allow)
    }
}

/// A rule pattern against a path: `*` matches any run of characters, a final `$` anchors the end.
fn matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let parts: Vec<&str> = pattern.split('*').collect();
    let Some(first) = parts.first() else { return true };
    if !path.starts_with(first) {
        return false;
    }
    let mut pos = first.len();
    for (i, part) in parts.iter().enumerate().skip(1) {
        let last = i == parts.len() - 1;
        if last && anchored {
            return path.len() >= pos + part.len() && path.ends_with(part);
        }
        match path[pos..].find(part) {
            Some(at) => pos += at + part.len(),
            None => return false,
        }
    }
    !anchored || pos == path.len()
}

/// The rules that apply to `token` (lower case), else to `*`.
pub fn parse(text: &str, token: &str) -> Robots {
    struct Group {
        agents: Vec<String>,
        rules: Vec<(bool, String)>,
        delay: Option<Duration>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut in_rules = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                if in_rules || groups.is_empty() {
                    groups.push(Group { agents: vec![], rules: vec![], delay: None });
                    in_rules = false;
                }
                if let Some(g) = groups.last_mut() {
                    g.agents.push(value.to_ascii_lowercase());
                }
            }
            "allow" | "disallow" => {
                in_rules = true;
                if let Some(g) = groups.last_mut()
                    && !value.is_empty()
                {
                    g.rules.push((key == "allow", value.to_string()));
                }
            }
            "crawl-delay" => {
                in_rules = true;
                if let (Some(g), Ok(secs)) = (groups.last_mut(), value.parse::<f64>())
                    && secs.is_finite()
                    && secs >= 0.0
                {
                    g.delay = Some(Duration::from_secs_f64(secs).min(MAX_DELAY));
                }
            }
            _ => {}
        }
    }
    let ours = |a: &str| a == token || token.split('-').next() == Some(a);
    let mut chosen: Vec<&Group> = groups.iter().filter(|g| g.agents.iter().any(|a| ours(a))).collect();
    if chosen.is_empty() {
        chosen = groups.iter().filter(|g| g.agents.iter().any(|a| a == "*")).collect();
    }
    Robots {
        rules: chosen.iter().flat_map(|g| g.rules.iter().cloned()).collect(),
        delay: chosen.iter().filter_map(|g| g.delay).max(),
        deny_all: false,
    }
}

/// robots.txt files by origin (`scheme://host:port`), for the life of the app.
#[derive(Default)]
pub struct Cache(Mutex<HashMap<String, (Instant, Arc<Robots>)>>);

impl Cache {
    pub fn get(&self, origin: &str) -> Option<Arc<Robots>> {
        let map = self.0.lock().unwrap();
        let (until, robots) = map.get(origin)?;
        (Instant::now() < *until).then(|| robots.clone())
    }

    pub fn put(&self, origin: &str, robots: Robots, readable: bool) -> Arc<Robots> {
        let robots = Arc::new(robots);
        let keep = if readable { KEEP } else { KEEP_FAILED };
        self.0.lock().unwrap().insert(origin.to_string(), (Instant::now() + keep, robots.clone()));
        robots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TOKEN: &str = "distroclaw-olymp";

    #[test]
    fn the_longest_rule_wins_and_allow_wins_a_tie() {
        let r = parse(
            "User-agent: *\nDisallow: /private\nAllow: /private/open\nDisallow: /*.json$\nAllow: /p\nDisallow: /p\n",
            TOKEN,
        );
        assert!(!r.allows("/private/x") && r.allows("/private/open/page"));
        assert!(!r.allows("/data/list.json") && r.allows("/data/list.json?x=1"), "$ anchors the end");
        assert!(r.allows("/p"), "a tie goes to Allow");
        assert!(r.allows("/public") && r.allows("/"));
        assert!(r.allows("/robots.txt"));
    }

    #[test]
    fn our_group_beats_the_star_group_and_delays_are_kept() {
        let text = "User-agent: *\nDisallow: /\n\n# us\nUser-agent: DistroClaw\nUser-agent: other\nDisallow: /admin\nCrawl-delay: 2.5\n";
        let r = parse(text, TOKEN);
        assert!(r.allows("/news") && !r.allows("/admin/x"));
        assert_eq!(r.delay, Some(Duration::from_millis(2500)));
        let star = parse("User-agent: *\nDisallow: /\n", TOKEN);
        assert!(!star.allows("/anything"));
        assert!(parse("", TOKEN).allows("/x"), "an empty file allows everything");
        assert!(parse("User-agent: *\nDisallow:\n", TOKEN).allows("/x"), "an empty rule allows everything");
        assert!(parse("User-agent: googlebot\nDisallow: /\n", TOKEN).allows("/x"), "other robots' groups do not apply");
    }

    #[test]
    fn wildcards_match_runs_of_characters() {
        assert!(matches("/*/private", "/a/b/private/c"));
        assert!(matches("/search*q=", "/search?lang=de&q=x"));
        assert!(!matches("/a*b$", "/a-b-c"));
        assert!(matches("/a*b$", "/a-c-b"));
        assert!(Robots::deny_all().allows("/robots.txt") && !Robots::deny_all().allows("/"));
    }

    #[test]
    fn the_cache_keeps_files_per_origin() {
        let c = Cache::default();
        assert!(c.get("https://a.org").is_none());
        c.put("https://a.org", Robots::deny_all(), false);
        assert!(!c.get("https://a.org").unwrap().allows("/x"));
    }
}
