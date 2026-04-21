//! Fetch easylist-format filter lists, cache them in SQLite, and compile
//! them into a merged `RegexSet` for the proxy.
//!
//! Callers wire this into the proxy state at boot and can hot-swap the
//! active regex set when a refresh completes.

use std::path::Path;
use std::time::SystemTime;

use anyhow::Context;
use regex::{RegexSet, RegexSetBuilder};

use crate::filter_parser;

/// A single filter list subscription (name + URL).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FilterList {
    pub name: String,
    pub url: String,
}

/// Default subscriptions shipped with the app. Conservative choices so the
/// out-of-box experience is useful without surprising block behavior.
pub fn default_lists() -> Vec<FilterList> {
    vec![
        FilterList {
            name: "EasyPrivacy".to_string(),
            url: "https://easylist.to/easylist/easyprivacy.txt".to_string(),
        },
        FilterList {
            name: "EasyList".to_string(),
            url: "https://easylist.to/easylist/easylist.txt".to_string(),
        },
    ]
}

/// Compile the built-in safety patterns plus each filter-list document into
/// a collection of `RegexSet`s. Splitting avoids the regex crate's compiled
/// DFA size blowing past its per-set limit: EasyList + EasyPrivacy merged
/// into one set exceeds 128 MiB compiled, which trips the default guard.
/// With one set per list (and large lists chunked further), each set stays
/// small and the proxy iterates them - still O(log) on match thanks to
/// RegexSet, and cheap to build incrementally.
pub fn compile_patterns(
    builtin: &[&str],
    raw_documents: &[String],
) -> anyhow::Result<Vec<RegexSet>> {
    let mut sets: Vec<RegexSet> = Vec::with_capacity(raw_documents.len() + 1);

    // Built-in patterns always live in their own set, kept small.
    let (builtin_patterns, dropped) =
        validate_each(builtin.iter().map(|s| s.to_string()).collect());
    if dropped > 0 {
        tracing::warn!(
            dropped,
            "built-in filter patterns dropped during validation"
        );
    }
    if !builtin_patterns.is_empty() {
        sets.push(build_set(builtin_patterns).context("building built-in regex set")?);
    }

    // Each document becomes one or more sets. Large lists are chunked so a
    // single list can't blow past the compile size limit on its own.
    const CHUNK: usize = 5_000;
    for doc in raw_documents {
        let parsed = filter_parser::parse_document(doc);
        let (valid, dropped) = validate_each(parsed);
        if dropped > 0 {
            tracing::warn!(dropped, "dropped invalid regex patterns from filter list");
        }
        for chunk in valid.chunks(CHUNK) {
            sets.push(
                build_set(chunk.to_vec()).context("building regex set for filter-list chunk")?,
            );
        }
    }

    Ok(sets)
}

fn build_set(patterns: Vec<String>) -> Result<RegexSet, regex::Error> {
    // 128 MiB is plenty for a single chunk; chunks are bounded above.
    const SIZE_LIMIT: usize = 128 * 1024 * 1024;
    RegexSetBuilder::new(patterns)
        .size_limit(SIZE_LIMIT)
        .dfa_size_limit(SIZE_LIMIT)
        .build()
}

fn validate_each(candidates: Vec<String>) -> (Vec<String>, usize) {
    let mut ok = Vec::with_capacity(candidates.len());
    let mut dropped = 0usize;
    for p in candidates {
        if regex::Regex::new(&p).is_ok() {
            ok.push(p);
        } else {
            dropped += 1;
        }
    }
    (ok, dropped)
}

/// Fetch a list from its URL and return the raw body.
pub async fn fetch(url: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("BlueFlame/0.1 (+https://github.com/w1ck3ds0d4/BlueFlame)")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("building HTTP client")?;

    let resp = client.get(url).send().await.context("sending request")?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("unexpected status {status} from {url}");
    }
    resp.text().await.context("reading response body")
}

/// Cache directory for downloaded filter lists. We store one file per list
/// keyed by a safe filename derived from the URL.
pub fn cache_path(cache_dir: &Path, url: &str) -> std::path::PathBuf {
    let safe = url
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    cache_dir.join(format!("{safe}.txt"))
}

/// Load a list's contents from the cache, returning both the body and the
/// cache file's last-modified timestamp.
pub fn read_cache(path: &Path) -> anyhow::Result<(String, SystemTime)> {
    let body =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let meta = std::fs::metadata(path)?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Ok((body, modified))
}

/// Write the fetched body to disk atomically-ish (write-then-rename).
pub fn write_cache(path: &Path, body: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_merges_builtin_and_docs() {
        let builtin: &[&str] = &[r"^https?://[^/]*hardcoded\.example/"];
        let doc = "||added.example^\n||skipped-cosmetic##.foo\n".to_string();
        let sets = compile_patterns(builtin, &[doc]).unwrap();
        assert!(any_match(&sets, "https://hardcoded.example/x"));
        assert!(any_match(&sets, "https://added.example/tracker"));
        assert!(!any_match(&sets, "https://unrelated.example/"));
    }

    fn any_match(sets: &[RegexSet], url: &str) -> bool {
        sets.iter().any(|s| s.is_match(url))
    }

    #[test]
    fn cache_path_is_stable() {
        let dir = std::path::Path::new("/tmp/blueflame");
        let a = cache_path(dir, "https://easylist.to/easylist/easyprivacy.txt");
        let b = cache_path(dir, "https://easylist.to/easylist/easyprivacy.txt");
        assert_eq!(a, b);
    }

    #[test]
    fn write_then_read_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("list.txt");
        write_cache(&p, "||a.example^\n").unwrap();
        let (body, _) = read_cache(&p).unwrap();
        assert_eq!(body, "||a.example^\n");
    }
}
