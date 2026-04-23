//! Search engine picker. Stored in SQLite as a single `search_engine` setting.

use std::path::Path;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Supported privacy-aligned search engines. All of these honor
/// Do-Not-Track and do not require accounts. Ahmia indexes `.onion`
/// sites; result pages load on clearnet but the links require Tor
/// turned on to follow.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    #[default]
    DuckDuckGo,
    Brave,
    Startpage,
    Mojeek,
    Kagi,
    Ecosia,
    Ahmia,
}

impl SearchEngine {
    pub fn id(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Brave => "brave",
            Self::Startpage => "startpage",
            Self::Mojeek => "mojeek",
            Self::Kagi => "kagi",
            Self::Ecosia => "ecosia",
            Self::Ahmia => "ahmia",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "DuckDuckGo",
            Self::Brave => "Brave Search",
            Self::Startpage => "Startpage",
            Self::Mojeek => "Mojeek",
            Self::Kagi => "Kagi",
            Self::Ecosia => "Ecosia",
            Self::Ahmia => "Ahmia (.onion)",
        }
    }

    /// Build a search URL for the given query. Query is URL-encoded here.
    pub fn search_url(self, query: &str) -> String {
        let q = url_encode(query);
        match self {
            Self::DuckDuckGo => format!("https://duckduckgo.com/?q={q}"),
            Self::Brave => format!("https://search.brave.com/search?q={q}"),
            Self::Startpage => format!("https://www.startpage.com/do/search?q={q}"),
            Self::Mojeek => format!("https://www.mojeek.com/search?q={q}"),
            Self::Kagi => format!("https://kagi.com/search?q={q}"),
            Self::Ecosia => format!("https://www.ecosia.org/search?q={q}"),
            Self::Ahmia => format!("https://ahmia.fi/search/?q={q}"),
        }
    }

    /// Home page URL suitable for opening in a new tab.
    #[allow(dead_code)] // consumed by a later new-tab-page PR
    pub fn home_url(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "https://duckduckgo.com",
            Self::Brave => "https://search.brave.com",
            Self::Startpage => "https://www.startpage.com",
            Self::Mojeek => "https://www.mojeek.com",
            Self::Kagi => "https://kagi.com",
            Self::Ecosia => "https://www.ecosia.org",
            Self::Ahmia => "https://ahmia.fi",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "duckduckgo" => Some(Self::DuckDuckGo),
            "brave" => Some(Self::Brave),
            "startpage" => Some(Self::Startpage),
            "mojeek" => Some(Self::Mojeek),
            "kagi" => Some(Self::Kagi),
            "ecosia" => Some(Self::Ecosia),
            "ahmia" => Some(Self::Ahmia),
            _ => None,
        }
    }

    pub fn all() -> &'static [SearchEngine] {
        &[
            Self::DuckDuckGo,
            Self::Brave,
            Self::Startpage,
            Self::Mojeek,
            Self::Kagi,
            Self::Ecosia,
            Self::Ahmia,
        ]
    }

    /// Host for this search engine's main search URL. Used by the proxy to
    /// carve out an always-allow rule - a contextual EasyList line without
    /// the `$third-party` guard otherwise blocks the user's own search.
    pub fn host(self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo.com",
            Self::Brave => "search.brave.com",
            Self::Startpage => "www.startpage.com",
            Self::Mojeek => "www.mojeek.com",
            Self::Kagi => "kagi.com",
            Self::Ecosia => "www.ecosia.org",
            Self::Ahmia => "ahmia.fi",
        }
    }
}

/// Hosts that are always allowed through the filter regardless of blocklist
/// rules. Prevents the classic overblock where a `?q=` rule meant for
/// trackers blocks the user's legitimate search.
pub fn always_allowed_hosts() -> Vec<&'static str> {
    SearchEngine::all().iter().map(|e| e.host()).collect()
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Persist / load the chosen engine in a tiny SQLite file under the app data dir.
pub struct SearchSettings {
    db_path: std::path::PathBuf,
}

impl SearchSettings {
    pub fn open<P: AsRef<Path>>(dir: P) -> anyhow::Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let db_path = dir.join("search.sqlite");
        let this = Self { db_path };
        this.ensure_schema()?;
        Ok(this)
    }

    fn conn(&self) -> anyhow::Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        self.conn()?.execute_batch(
            "create table if not exists settings (k text primary key, v text not null);",
        )?;
        Ok(())
    }

    pub fn get_engine(&self) -> anyhow::Result<SearchEngine> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select v from settings where k = 'search_engine'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            return Ok(SearchEngine::from_id(&id).unwrap_or_default());
        }
        Ok(SearchEngine::default())
    }

    pub fn set_engine(&self, engine: SearchEngine) -> anyhow::Result<()> {
        self.conn()?.execute(
            "insert into settings (k, v) values ('search_engine', ?1)
             on conflict(k) do update set v = excluded.v",
            params![engine.id()],
        )?;
        Ok(())
    }

    pub fn get_metasearch(&self) -> anyhow::Result<bool> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select v from settings where k = 'metasearch'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let v: String = row.get(0)?;
            return Ok(v == "1" || v.eq_ignore_ascii_case("true"));
        }
        Ok(false)
    }

    pub fn set_metasearch(&self, enabled: bool) -> anyhow::Result<()> {
        self.conn()?.execute(
            "insert into settings (k, v) values ('metasearch', ?1)
             on conflict(k) do update set v = excluded.v",
            params![if enabled { "1" } else { "0" }],
        )?;
        Ok(())
    }

    /// Whether new tabs should spoof a mobile user-agent so that
    /// UA-sniffing sites serve their mobile layout (YouTube, Reddit,
    /// Twitter etc. pick mobile vs desktop from the UA, not from the
    /// viewport width, so pure window-resize doesn't help). Stored as
    /// "mobile" / "desktop"; missing or any other value means desktop.
    pub fn get_mobile_ua(&self) -> anyhow::Result<bool> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select v from settings where k = 'mobile_ua'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let v: String = row.get(0)?;
            return Ok(v == "mobile");
        }
        Ok(false)
    }

    pub fn set_mobile_ua(&self, mobile: bool) -> anyhow::Result<()> {
        self.conn()?.execute(
            "insert into settings (k, v) values ('mobile_ua', ?1)
             on conflict(k) do update set v = excluded.v",
            params![if mobile { "mobile" } else { "desktop" }],
        )?;
        Ok(())
    }

    /// Last-known desktop-mode window size, snapshotted right before
    /// flipping to mobile so we can restore whatever the user was
    /// using when they flip back. Returns `None` if no snapshot
    /// exists (fresh install, or user has only ever been on desktop).
    /// Stored as two separate keys, logical pixels, serialized via
    /// `{:.0}` to keep the sqlite text compact.
    pub fn get_desktop_window_size(&self) -> anyhow::Result<Option<(f64, f64)>> {
        let c = self.conn()?;
        let mut stmt = c.prepare(
            "select (select v from settings where k = 'desktop_window_width'),
                    (select v from settings where k = 'desktop_window_height')",
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let w: Option<String> = row.get(0)?;
            let h: Option<String> = row.get(1)?;
            if let (Some(w), Some(h)) = (w, h) {
                if let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>()) {
                    return Ok(Some((w, h)));
                }
            }
        }
        Ok(None)
    }

    pub fn set_desktop_window_size(&self, width: f64, height: f64) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "insert into settings (k, v) values ('desktop_window_width', ?1)
             on conflict(k) do update set v = excluded.v",
            params![format!("{width:.0}")],
        )?;
        conn.execute(
            "insert into settings (k, v) values ('desktop_window_height', ?1)
             on conflict(k) do update set v = excluded.v",
            params![format!("{height:.0}")],
        )?;
        Ok(())
    }

    pub fn get_tor_enabled(&self) -> anyhow::Result<bool> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select v from settings where k = 'tor_enabled'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let v: String = row.get(0)?;
            return Ok(v == "1" || v.eq_ignore_ascii_case("true"));
        }
        Ok(false)
    }

    pub fn set_tor_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        self.conn()?.execute(
            "insert into settings (k, v) values ('tor_enabled', ?1)
             on conflict(k) do update set v = excluded.v",
            params![if enabled { "1" } else { "0" }],
        )?;
        Ok(())
    }

    pub fn get_tor_proxy_addr(&self) -> anyhow::Result<String> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select v from settings where k = 'tor_proxy_addr'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let v: String = row.get(0)?;
            if !v.is_empty() {
                return Ok(v);
            }
        }
        Ok(DEFAULT_TOR_SOCKS5.to_string())
    }

    pub fn set_tor_proxy_addr(&self, addr: &str) -> anyhow::Result<()> {
        self.conn()?.execute(
            "insert into settings (k, v) values ('tor_proxy_addr', ?1)
             on conflict(k) do update set v = excluded.v",
            params![addr],
        )?;
        Ok(())
    }

    pub fn get_tor_builtin(&self) -> anyhow::Result<bool> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select v from settings where k = 'tor_builtin'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let v: String = row.get(0)?;
            return Ok(v == "1" || v.eq_ignore_ascii_case("true"));
        }
        Ok(false)
    }

    pub fn set_tor_builtin(&self, enabled: bool) -> anyhow::Result<()> {
        self.conn()?.execute(
            "insert into settings (k, v) values ('tor_builtin', ?1)
             on conflict(k) do update set v = excluded.v",
            params![if enabled { "1" } else { "0" }],
        )?;
        Ok(())
    }

    /// Dump every persisted setting as `(key, value)` pairs for export.
    /// Includes every row in the settings table, so new keys added later
    /// are picked up automatically.
    pub fn dump_all(&self) -> anyhow::Result<Vec<(String, String)>> {
        let c = self.conn()?;
        let mut stmt = c.prepare("select k, v from settings order by k")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Restore settings from an export. Only keys in the allowlist are
    /// written, so a tampered backup can't inject arbitrary keys. Returns
    /// the count of keys actually applied.
    pub fn restore_all(&self, pairs: &[(String, String)]) -> anyhow::Result<usize> {
        const ALLOWED: &[&str] = &[
            "search_engine",
            "metasearch",
            "mobile_ua",
            "desktop_window_width",
            "desktop_window_height",
            "tor_enabled",
            "tor_proxy_addr",
            "tor_builtin",
        ];
        let c = self.conn()?;
        let mut applied = 0;
        for (k, v) in pairs {
            if !ALLOWED.contains(&k.as_str()) {
                continue;
            }
            c.execute(
                "insert into settings (k, v) values (?1, ?2)
                 on conflict(k) do update set v = excluded.v",
                params![k, v],
            )?;
            applied += 1;
        }
        Ok(applied)
    }
}

/// Stock Tor daemon SOCKS5 port. Tor Browser uses 9150.
pub const DEFAULT_TOR_SOCKS5: &str = "127.0.0.1:9050";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_engine_id() {
        for engine in SearchEngine::all() {
            assert_eq!(SearchEngine::from_id(engine.id()), Some(*engine));
        }
    }

    #[test]
    fn unknown_id_is_none() {
        assert!(SearchEngine::from_id("lycos").is_none());
    }

    #[test]
    fn search_urls_include_query() {
        let u = SearchEngine::Brave.search_url("rust async");
        assert!(u.starts_with("https://search.brave.com/"));
        assert!(u.contains("rust+async"));
    }

    #[test]
    fn settings_persistence() {
        let tmp = tempfile::tempdir().unwrap();
        let s = SearchSettings::open(tmp.path()).unwrap();
        assert_eq!(s.get_engine().unwrap(), SearchEngine::DuckDuckGo);
        s.set_engine(SearchEngine::Brave).unwrap();
        assert_eq!(s.get_engine().unwrap(), SearchEngine::Brave);
    }

    #[test]
    fn metasearch_toggle_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let s = SearchSettings::open(tmp.path()).unwrap();
        assert!(!s.get_metasearch().unwrap());
        s.set_metasearch(true).unwrap();
        assert!(s.get_metasearch().unwrap());
        s.set_metasearch(false).unwrap();
        assert!(!s.get_metasearch().unwrap());
    }
}
