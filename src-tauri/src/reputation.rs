//! Host-level reputation feed - augments the pattern-based malware
//! signals with known-bad hosts pulled from a refreshable list.
//!
//! Loaders use the same `fetch` / cache helpers as `list_loader` so feed
//! subscriptions and filter-list subscriptions share a disk layout. The
//! in-memory shape is a hash-set of lowercase hosts; lookups from
//! `trust::evaluate` are O(1). Feed parsing is URLHaus-format (one URL
//! per line, `#` comments) - we extract the host from each line and
//! throw the path away.
//!
//! The store always starts with a tiny bundled list so even first-run
//! (no network) has some defense, then the background hydrator adds
//! every cached feed and finally every fresh-fetched feed. Writes are
//! incremental - each feed's hosts union into the existing set, not a
//! full replacement, so a partial fetch (one feed succeeds, one fails)
//! still leaves something useful loaded.

use std::collections::HashSet;
use std::path::Path;
use std::sync::RwLock;
use std::time::SystemTime;

use anyhow::Context;

use crate::list_loader;

/// A single reputation feed subscription.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReputationFeed {
    pub name: String,
    pub url: String,
}

/// Default feeds shipped with the app. URLHaus is abuse.ch's free,
/// redistributable malware-URL feed (no API key required). Add more
/// here only if they're similarly permissive.
pub fn default_feeds() -> Vec<ReputationFeed> {
    vec![ReputationFeed {
        name: "URLHaus".to_string(),
        url: "https://urlhaus.abuse.ch/downloads/text/".to_string(),
    }]
}

/// Hand-curated baseline so a fresh install with no network still
/// catches the most obvious red flags. Kept tiny on purpose - the
/// network feeds carry the real volume.
const BUNDLED_BAD_HOSTS: &[&str] = &[
    // .test placeholders drive the tests in trust.rs + below. Not a
    // real blocklist - the real volume comes from URLHaus at refresh.
    "phishing.example.test",
    "malware.example.test",
];

#[derive(Debug, Default)]
pub struct ReputationStore {
    bad_hosts: RwLock<HashSet<String>>,
}

impl ReputationStore {
    pub fn with_bundled() -> Self {
        let mut set = HashSet::new();
        for h in BUNDLED_BAD_HOSTS {
            set.insert((*h).to_string());
        }
        Self {
            bad_hosts: RwLock::new(set),
        }
    }

    /// `true` if `host` - or any parent domain - is on the reputation
    /// list. Case-insensitive.
    pub fn is_known_bad(&self, host: &str) -> bool {
        let needle = host.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return false;
        }
        let guard = self.bad_hosts.read().expect("reputation rwlock poisoned");
        if guard.contains(&needle) {
            return true;
        }
        // Match subdomains of a listed host too: `login.phishing.example`
        // matches a listed `phishing.example`. Walk up label-by-label.
        let mut rest = needle.as_str();
        while let Some(idx) = rest.find('.') {
            rest = &rest[idx + 1..];
            if guard.contains(rest) {
                return true;
            }
        }
        false
    }

    /// Merge a new batch of hosts into the store. Lowercased + trimmed.
    pub fn extend(&self, hosts: impl IntoIterator<Item = String>) {
        let mut guard = self.bad_hosts.write().expect("reputation rwlock poisoned");
        for h in hosts {
            let h = h.trim().to_ascii_lowercase();
            if !h.is_empty() {
                guard.insert(h);
            }
        }
    }

    /// Current host-count. Only consumed by tests today; left in for
    /// the obvious future surface (Settings panel showing "N hosts
    /// loaded"). `#[allow(dead_code)]` keeps clippy quiet until then.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.bad_hosts.read().map(|g| g.len()).unwrap_or_default()
    }
}

/// Parse a URLHaus-style plain-text feed. Format is one URL per line;
/// blank lines and lines starting with `#` are comments. We keep only
/// the host component - we score by host, not by full URL.
pub fn parse_feed(body: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Ok(u) = url::Url::parse(line) {
            if let Some(h) = u.host_str() {
                out.insert(h.to_ascii_lowercase());
            }
        }
    }
    out
}

/// Cache directory under app_data for reputation feeds. Distinct from
/// the filter-list cache so the two never accidentally stomp each
/// other's files.
pub fn cache_dir(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("reputation-cache")
}

/// Status entry shown in the Settings UI - one row per subscription,
/// with whether we have a cached copy and how old it is.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FeedStatus {
    pub name: String,
    pub url: String,
    pub cached: bool,
    pub cached_at: Option<u64>,
}

pub fn status_for(feed: &ReputationFeed, cache_dir: &Path) -> FeedStatus {
    let path = list_loader::cache_path(cache_dir, &feed.url);
    let (cached, cached_at) = match std::fs::metadata(&path) {
        Ok(m) => {
            let ts = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            (true, ts)
        }
        Err(_) => (false, None),
    };
    FeedStatus {
        name: feed.name.clone(),
        url: feed.url.clone(),
        cached,
        cached_at,
    }
}

/// Fetch `feed` fresh from the network, persist its body to cache, and
/// return the parsed host set. Uses the same `fetch` helper as filter
/// lists so upstream routing (Tor etc.) is consistent.
pub async fn fetch_and_cache(
    feed: &ReputationFeed,
    cache_dir: &Path,
) -> anyhow::Result<HashSet<String>> {
    let body = list_loader::fetch(&feed.url)
        .await
        .with_context(|| format!("fetching reputation feed {}", feed.url))?;
    let path = list_loader::cache_path(cache_dir, &feed.url);
    list_loader::write_cache(&path, &body).with_context(|| format!("caching {}", feed.url))?;
    Ok(parse_feed(&body))
}

/// Load a feed from its cache file without touching the network.
/// Returns `None` if the cache file doesn't exist yet.
pub fn load_cached(feed: &ReputationFeed, cache_dir: &Path) -> Option<HashSet<String>> {
    let path = list_loader::cache_path(cache_dir, &feed.url);
    list_loader::read_cache(&path)
        .ok()
        .map(|(body, _)| parse_feed(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_is_matched_out_of_the_box() {
        let s = ReputationStore::with_bundled();
        assert!(s.is_known_bad("phishing.example.test"));
        assert!(s.is_known_bad("malware.example.test"));
        assert!(!s.is_known_bad("example.com"));
    }

    #[test]
    fn subdomain_of_listed_host_matches() {
        let s = ReputationStore::with_bundled();
        assert!(s.is_known_bad("login.phishing.example.test"));
        assert!(s.is_known_bad("deep.nested.malware.example.test"));
    }

    #[test]
    fn extend_merges_new_hosts() {
        let s = ReputationStore::with_bundled();
        let before = s.len();
        s.extend(["EVIL.example".to_string(), "  ".to_string()]);
        assert_eq!(s.len(), before + 1);
        assert!(s.is_known_bad("evil.example"));
    }

    #[test]
    fn parse_feed_extracts_hosts_and_skips_comments() {
        let body = "\
# URLHaus feed
https://bad.example/malware.exe
http://PHISH.example/login?a=1

# blank line above
not-a-url
https://bad.example/other
";
        let set = parse_feed(body);
        assert!(set.contains("bad.example"));
        assert!(set.contains("phish.example"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn empty_host_never_matches() {
        let s = ReputationStore::with_bundled();
        assert!(!s.is_known_bad(""));
        assert!(!s.is_known_bad("   "));
    }
}
