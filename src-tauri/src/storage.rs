//! SQLite-backed personal index: history + bookmarks + settings.
//!
//! The store is opened once under `<app_data>/personal.sqlite` and shared
//! through Tauri state via a mutex. Search uses plain `LIKE` for now; a
//! follow-up can swap in FTS5 without changing the command surface.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Escape `%`, `_`, and `\` in a user string so it can be used as the
/// payload of a SQL LIKE expression without the user's input being treated
/// as wildcards. Pair with an `ESCAPE '\'` clause in the query.
fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// One recorded visit with a resolved title (or host) for display.
#[derive(Debug, Clone, Serialize)]
pub struct Visit {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
    pub visit_count: i64,
}

/// One bookmark entry.
#[derive(Debug, Clone, Serialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub created_at: i64,
}

/// One trust-score sample for a host at a point in time. The UI plots
/// `{x: recorded_at, y: score}` as a sparkline.
#[derive(Debug, Clone, Serialize)]
pub struct TrustSample {
    pub score: u8,
    pub recorded_at: i64,
}

/// One URL-bar autocomplete suggestion. `source` lets the UI badge bookmarks
/// separately from plain history hits.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub source: SuggestionSource,
    pub visit_count: i64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SuggestionSource {
    Bookmark,
    History,
}

pub struct Store {
    conn: Connection,
}

pub type SharedStore = Mutex<Store>;

impl Store {
    pub fn open<P: AsRef<Path>>(dir: P) -> anyhow::Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = dir.join("personal.sqlite");
        let conn = Connection::open(path)?;
        let s = Self { conn };
        s.ensure_schema()?;
        Ok(s)
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            r#"
            create table if not exists history (
                id integer primary key autoincrement,
                url text not null,
                title text,
                visited_at integer not null,
                visit_count integer not null default 1
            );
            create unique index if not exists ux_history_url on history(url);
            create index if not exists ix_history_visited_at on history(visited_at desc);

            create table if not exists bookmarks (
                url text primary key,
                title text,
                created_at integer not null
            );

            -- Score samples over time per host so the trust panel can
            -- render a sparkline. Each `get_trust` call writes one row
            -- under a throttle: we skip writes when a sample for the
            -- same host landed within the last TRUST_THROTTLE_SECS so
            -- the table doesn't balloon on pages that re-poll every
            -- few seconds.
            create table if not exists trust_samples (
                id integer primary key autoincrement,
                host text not null,
                score integer not null,
                recorded_at integer not null
            );
            create index if not exists ix_trust_host_recorded on
                trust_samples(host, recorded_at desc);
            "#,
        )?;
        Ok(())
    }

    /// Write a single score sample for `host`. Throttled: drops the
    /// write if an entry for the same host was recorded in the last
    /// `throttle_secs`. Called from `get_trust`, which runs on every
    /// URL-bar poll (~3s), so without throttling we'd write hundreds
    /// of rows per visited page.
    pub fn record_trust_sample(
        &self,
        host: &str,
        score: u8,
        throttle_secs: u64,
    ) -> anyhow::Result<()> {
        if host.is_empty() {
            return Ok(());
        }
        let now = now_secs();
        // `throttle_secs == 0` means "always write" - skip the lookup
        // entirely. Otherwise a burst of inserts in the same wall-clock
        // second would all look "within 0 seconds of each other" to the
        // `recorded_at >= cutoff` check (clock granularity is 1s).
        if throttle_secs > 0 {
            let cutoff = now.saturating_sub(throttle_secs as i64);
            let recent: Option<i64> = self
                .conn
                .query_row(
                    "select recorded_at from trust_samples
                     where host = ?1 and recorded_at >= ?2
                     order by recorded_at desc limit 1",
                    params![host, cutoff],
                    |row| row.get(0),
                )
                .optional()?;
            if recent.is_some() {
                return Ok(());
            }
        }
        self.conn.execute(
            "insert into trust_samples (host, score, recorded_at) values (?1, ?2, ?3)",
            params![host, score as i64, now],
        )?;
        Ok(())
    }

    /// Return up to `limit` most-recent score samples for `host`, OLDEST
    /// first so the UI can render a left-to-right sparkline without
    /// reversing the result.
    pub fn trust_history(&self, host: &str, limit: usize) -> anyhow::Result<Vec<TrustSample>> {
        // Fetch newest N rows, then reverse to chronological order.
        // Order by (recorded_at desc, id desc) so multiple samples that
        // landed in the same wall-clock second still come back in strict
        // insertion order - without the id tiebreak, reversing the
        // result to chronological is non-deterministic.
        let mut stmt = self.conn.prepare(
            "select score, recorded_at from trust_samples
             where host = ?1 order by recorded_at desc, id desc limit ?2",
        )?;
        let rows = stmt
            .query_map(params![host, limit as i64], |row| {
                Ok(TrustSample {
                    score: row.get::<_, i64>(0)? as u8,
                    recorded_at: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().rev().collect())
    }

    /// Record or update a visit. If we already know this URL we just bump the
    /// `visit_count` and refresh `visited_at`; otherwise a new row is inserted.
    pub fn record_visit(&self, url: &str, title: &str) -> anyhow::Result<()> {
        if url.is_empty() || url.starts_with("data:") || url.starts_with("about:") {
            return Ok(());
        }
        let now = now_secs();
        self.conn.execute(
            "insert into history (url, title, visited_at, visit_count) values (?1, ?2, ?3, 1)
             on conflict(url) do update set
                 title = coalesce(excluded.title, history.title),
                 visited_at = excluded.visited_at,
                 visit_count = history.visit_count + 1",
            params![url, title, now],
        )?;
        Ok(())
    }

    pub fn recent_history(&self, limit: usize) -> anyhow::Result<Vec<Visit>> {
        let mut stmt = self.conn.prepare(
            "select id, url, title, visited_at, visit_count from history
             order by visited_at desc limit ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(Visit {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    visited_at: row.get(3)?,
                    visit_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Simple personal search: LIKE against title and URL, ranked by visit
    /// frequency then recency.
    pub fn search_history(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Visit>> {
        let pat = format!("%{}%", escape_like(query.trim()));
        let mut stmt = self.conn.prepare(
            "select id, url, title, visited_at, visit_count from history
             where title like ?1 escape '\\' or url like ?1 escape '\\'
             order by visit_count desc, visited_at desc
             limit ?2",
        )?;
        let rows = stmt
            .query_map(params![pat, limit as i64], |row| {
                Ok(Visit {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    visited_at: row.get(3)?,
                    visit_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn clear_history(&self) -> anyhow::Result<()> {
        self.conn.execute("delete from history", [])?;
        Ok(())
    }

    /// Most-visited pages for the speed-dial on the new-tab page.
    pub fn top_visited(&self, limit: usize) -> anyhow::Result<Vec<Visit>> {
        let mut stmt = self.conn.prepare(
            "select id, url, title, visited_at, visit_count from history
             order by visit_count desc, visited_at desc limit ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(Visit {
                    id: row.get(0)?,
                    url: row.get(1)?,
                    title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    visited_at: row.get(3)?,
                    visit_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn toggle_bookmark(&self, url: &str, title: &str) -> anyhow::Result<bool> {
        let exists: bool = self
            .conn
            .query_row(
                "select 1 from bookmarks where url = ?1",
                params![url],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            self.conn
                .execute("delete from bookmarks where url = ?1", params![url])?;
            Ok(false)
        } else {
            self.conn.execute(
                "insert into bookmarks (url, title, created_at) values (?1, ?2, ?3)",
                params![url, title, now_secs()],
            )?;
            Ok(true)
        }
    }

    /// Insert a bookmark only if its url isn't already bookmarked.
    /// Returns `true` if a row was inserted, `false` if the url already existed.
    /// Used by import flows so re-importing doesn't overwrite existing
    /// `created_at` timestamps or duplicate entries.
    pub fn insert_bookmark_if_new(
        &self,
        url: &str,
        title: &str,
        created_at: i64,
    ) -> anyhow::Result<bool> {
        let inserted = self.conn.execute(
            "insert into bookmarks (url, title, created_at) values (?1, ?2, ?3)
             on conflict(url) do nothing",
            params![url, title, created_at],
        )?;
        Ok(inserted > 0)
    }

    pub fn is_bookmarked(&self, url: &str) -> anyhow::Result<bool> {
        Ok(self
            .conn
            .query_row(
                "select 1 from bookmarks where url = ?1",
                params![url],
                |_| Ok(true),
            )
            .unwrap_or(false))
    }

    /// URL-bar autocomplete: bookmarks matching `query` ranked first, then
    /// history matches ranked by visit_count. Deduped by URL.
    pub fn suggest(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Suggestion>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }
        let pat = format!("%{}%", escape_like(trimmed));
        let mut out: Vec<Suggestion> = Vec::with_capacity(limit);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        let mut bstmt = self.conn.prepare(
            "select url, title from bookmarks
             where title like ?1 escape '\\' or url like ?1 escape '\\'
             order by created_at desc, rowid desc
             limit ?2",
        )?;
        let brows = bstmt
            .query_map(params![pat, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (url, title) in brows {
            if seen.insert(url.clone()) {
                out.push(Suggestion {
                    url,
                    title,
                    source: SuggestionSource::Bookmark,
                    visit_count: 0,
                });
            }
            if out.len() >= limit {
                return Ok(out);
            }
        }

        let hits = self.search_history(trimmed, limit * 2)?;
        for v in hits {
            if out.len() >= limit {
                break;
            }
            if seen.insert(v.url.clone()) {
                out.push(Suggestion {
                    url: v.url,
                    title: v.title,
                    source: SuggestionSource::History,
                    visit_count: v.visit_count,
                });
            }
        }
        Ok(out)
    }

    pub fn list_bookmarks(&self) -> anyhow::Result<Vec<Bookmark>> {
        let mut stmt = self.conn.prepare(
            "select url, title, created_at from bookmarks order by created_at desc, rowid desc",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Bookmark {
                    url: row.get(0)?,
                    title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    created_at: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path()).unwrap();
        (tmp, store)
    }

    #[test]
    fn record_and_search_history() {
        let (_t, s) = fresh_store();
        s.record_visit("https://github.com/rust-lang", "Rust on GitHub")
            .unwrap();
        s.record_visit("https://rust-lang.org", "The Rust Language")
            .unwrap();
        s.record_visit("https://rust-lang.org", "The Rust Language")
            .unwrap();
        s.record_visit("https://example.com", "Example").unwrap();

        let found = s.search_history("rust", 10).unwrap();
        assert_eq!(found.len(), 2);
        // The one visited twice ranks first
        assert_eq!(found[0].url, "https://rust-lang.org");
        assert_eq!(found[0].visit_count, 2);
    }

    #[test]
    fn visit_increments_not_duplicates() {
        let (_t, s) = fresh_store();
        s.record_visit("https://github.com", "GitHub").unwrap();
        s.record_visit("https://github.com", "GitHub").unwrap();
        s.record_visit("https://github.com", "GitHub").unwrap();
        let all = s.recent_history(10).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].visit_count, 3);
    }

    #[test]
    fn data_urls_are_not_recorded() {
        let (_t, s) = fresh_store();
        s.record_visit("data:text/html;base64,PGh0bWw+", "New tab")
            .unwrap();
        s.record_visit("about:blank", "Blank").unwrap();
        s.record_visit("", "Empty").unwrap();
        assert!(s.recent_history(10).unwrap().is_empty());
    }

    #[test]
    fn toggle_bookmark_idempotent_round_trip() {
        let (_t, s) = fresh_store();
        let url = "https://crates.io";
        assert!(!s.is_bookmarked(url).unwrap());
        assert!(s.toggle_bookmark(url, "crates.io").unwrap());
        assert!(s.is_bookmarked(url).unwrap());
        assert!(!s.toggle_bookmark(url, "crates.io").unwrap());
        assert!(!s.is_bookmarked(url).unwrap());
    }

    #[test]
    fn list_bookmarks_newest_first() {
        let (_t, s) = fresh_store();
        s.toggle_bookmark("https://a.example", "a").unwrap();
        s.toggle_bookmark("https://b.example", "b").unwrap();
        let list = s.list_bookmarks().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].url, "https://b.example");
    }

    #[test]
    fn top_visited_ranks_by_visit_count() {
        let (_t, s) = fresh_store();
        s.record_visit("https://a.example", "a").unwrap();
        s.record_visit("https://b.example", "b").unwrap();
        s.record_visit("https://b.example", "b").unwrap();
        s.record_visit("https://c.example", "c").unwrap();
        s.record_visit("https://c.example", "c").unwrap();
        s.record_visit("https://c.example", "c").unwrap();
        let top = s.top_visited(2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].url, "https://c.example");
        assert_eq!(top[0].visit_count, 3);
        assert_eq!(top[1].url, "https://b.example");
    }

    #[test]
    fn suggest_prefers_bookmarks_and_dedupes() {
        let (_t, s) = fresh_store();
        // Two history hits plus a bookmark that duplicates one.
        s.record_visit("https://rust-lang.org", "Rust").unwrap();
        s.record_visit("https://rust-lang.org", "Rust").unwrap();
        s.record_visit("https://docs.rs/rust", "Rust docs").unwrap();
        s.record_visit("https://example.com", "Unrelated").unwrap();
        s.toggle_bookmark("https://rust-lang.org", "Rust (bookmark)")
            .unwrap();

        let out = s.suggest("rust", 10).unwrap();

        // rust-lang appears first tagged as a bookmark (not duplicated)
        assert_eq!(out[0].url, "https://rust-lang.org");
        assert_eq!(out[0].source, SuggestionSource::Bookmark);
        assert!(
            out.iter()
                .filter(|s| s.url == "https://rust-lang.org")
                .count()
                == 1
        );
        // docs.rs comes through as history
        assert!(out
            .iter()
            .any(|s| s.url == "https://docs.rs/rust" && s.source == SuggestionSource::History));
        // unrelated is not included
        assert!(!out.iter().any(|s| s.url == "https://example.com"));
    }

    #[test]
    fn suggest_empty_query_returns_empty() {
        let (_t, s) = fresh_store();
        s.record_visit("https://a.example", "a").unwrap();
        assert!(s.suggest("", 10).unwrap().is_empty());
        assert!(s.suggest("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn suggest_respects_limit() {
        let (_t, s) = fresh_store();
        for i in 0..5 {
            s.record_visit(&format!("https://example.com/{i}"), "Example")
                .unwrap();
        }
        assert_eq!(s.suggest("example", 3).unwrap().len(), 3);
    }

    #[test]
    fn clear_history_wipes_everything() {
        let (_t, s) = fresh_store();
        s.record_visit("https://a.example", "a").unwrap();
        s.record_visit("https://b.example", "b").unwrap();
        s.clear_history().unwrap();
        assert!(s.recent_history(10).unwrap().is_empty());
    }

    #[test]
    fn trust_history_round_trips_in_chrono_order() {
        let (_t, s) = fresh_store();
        // Throttle of 0 so every insert lands without deduping.
        s.record_trust_sample("site.example", 50, 0).unwrap();
        s.record_trust_sample("site.example", 60, 0).unwrap();
        s.record_trust_sample("site.example", 70, 0).unwrap();
        let hist = s.trust_history("site.example", 10).unwrap();
        assert_eq!(hist.len(), 3);
        // Oldest first: scores should be monotone increasing.
        let scores: Vec<u8> = hist.iter().map(|r| r.score).collect();
        assert!(scores.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn trust_history_is_host_scoped() {
        let (_t, s) = fresh_store();
        s.record_trust_sample("a.example", 40, 0).unwrap();
        s.record_trust_sample("b.example", 90, 0).unwrap();
        let a = s.trust_history("a.example", 10).unwrap();
        let b = s.trust_history("b.example", 10).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].score, 40);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].score, 90);
    }

    #[test]
    fn trust_sample_throttle_dedupes_burst_writes() {
        let (_t, s) = fresh_store();
        // Generous throttle ensures the second write is dropped.
        s.record_trust_sample("site.example", 50, 3600).unwrap();
        s.record_trust_sample("site.example", 70, 3600).unwrap();
        let hist = s.trust_history("site.example", 10).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].score, 50);
    }
}
