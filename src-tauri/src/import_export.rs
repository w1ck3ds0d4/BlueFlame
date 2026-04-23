//! Backup / restore settings + bookmarks, and import of Netscape-format
//! browser bookmark exports (Chrome, Firefox, Edge, Safari all produce
//! the same HTML format when you use their "Export bookmarks" feature).
//!
//! Wire format for the native backup:
//! ```json
//! {
//!   "version": 1,
//!   "exported_at": 1713456000,
//!   "settings": [["search_engine", "duckduckgo"], ...],
//!   "bookmarks": [{"url": "...", "title": "...", "created_at": 1713...}]
//! }
//! ```
//!
//! Imports never overwrite existing bookmark rows: bookmarks are keyed
//! on url and conflicting rows are skipped so `created_at` is preserved.
//! Settings use overwrite semantics since the intent of restoring a
//! backup is "make this device match that one."

use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::search::SearchSettings;
use crate::storage::SharedStore;

const BACKUP_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct BackupFile {
    pub version: u32,
    pub exported_at: i64,
    pub settings: Vec<(String, String)>,
    pub bookmarks: Vec<BookmarkExport>,
}

#[derive(Serialize, Deserialize)]
pub struct BookmarkExport {
    pub url: String,
    pub title: String,
    pub created_at: i64,
}

#[derive(Serialize)]
pub struct ImportResult {
    pub settings_imported: usize,
    pub bookmarks_imported: usize,
    pub bookmarks_skipped: usize,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub async fn export_data(
    app: tauri::AppHandle,
    store: tauri::State<'_, SharedStore>,
) -> Result<String, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings = SearchSettings::open(&data_dir).map_err(|e| format!("open settings: {e}"))?;
    let all_settings = settings
        .dump_all()
        .map_err(|e| format!("dump settings: {e}"))?;

    let bookmarks = {
        let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
        s.list_bookmarks()
            .map_err(|e| format!("list bookmarks: {e}"))?
            .into_iter()
            .map(|b| BookmarkExport {
                url: b.url,
                title: b.title,
                created_at: b.created_at,
            })
            .collect()
    };

    let backup = BackupFile {
        version: BACKUP_VERSION,
        exported_at: now_secs(),
        settings: all_settings,
        bookmarks,
    };
    serde_json::to_string_pretty(&backup).map_err(|e| format!("serialize: {e}"))
}

#[tauri::command]
pub async fn import_data(
    app: tauri::AppHandle,
    store: tauri::State<'_, SharedStore>,
    json: String,
) -> Result<ImportResult, String> {
    let backup: BackupFile =
        serde_json::from_str(&json).map_err(|e| format!("not a valid backup file: {e}"))?;
    if backup.version > BACKUP_VERSION {
        return Err(format!(
            "backup is from a newer version ({}) than this build supports ({}).",
            backup.version, BACKUP_VERSION
        ));
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let settings = SearchSettings::open(&data_dir).map_err(|e| format!("open settings: {e}"))?;
    let settings_imported = settings
        .restore_all(&backup.settings)
        .map_err(|e| format!("restore settings: {e}"))?;

    let (bookmarks_imported, bookmarks_skipped) = {
        let s = store.lock().map_err(|e| format!("lock store: {e}"))?;
        let mut imported = 0usize;
        let mut skipped = 0usize;
        for b in &backup.bookmarks {
            if b.url.is_empty() {
                skipped += 1;
                continue;
            }
            match s.insert_bookmark_if_new(&b.url, &b.title, b.created_at) {
                Ok(true) => imported += 1,
                Ok(false) => skipped += 1,
                Err(_) => skipped += 1,
            }
        }
        (imported, skipped)
    };

    Ok(ImportResult {
        settings_imported,
        bookmarks_imported,
        bookmarks_skipped,
    })
}

#[tauri::command]
pub async fn import_bookmarks_html(
    store: tauri::State<'_, SharedStore>,
    html: String,
) -> Result<ImportResult, String> {
    let bookmarks = parse_netscape_bookmarks(&html);
    let now = now_secs();
    let s = store.lock().map_err(|e| format!("lock store: {e}"))?;

    let mut imported = 0usize;
    let mut skipped = 0usize;
    for (url, title, add_date) in bookmarks {
        let ts = if add_date > 0 { add_date } else { now };
        match s.insert_bookmark_if_new(&url, &title, ts) {
            Ok(true) => imported += 1,
            Ok(false) => skipped += 1,
            Err(_) => skipped += 1,
        }
    }

    Ok(ImportResult {
        settings_imported: 0,
        bookmarks_imported: imported,
        bookmarks_skipped: skipped,
    })
}

/// Pull `(url, title, add_date)` triples out of a Netscape-format
/// bookmark export. All mainstream browsers write this format when the
/// user picks "Export bookmarks": `<DT><A HREF="..." ADD_DATE="...">Title</A>`.
/// Attribute order, case, and surrounding whitespace vary across
/// browsers, so the parser is lenient. `add_date` is epoch seconds; 0
/// when the attribute is missing.
fn parse_netscape_bookmarks(html: &str) -> Vec<(String, String, i64)> {
    let anchor_re = Regex::new(r#"(?is)<a\s+([^>]*?)>(.*?)</a>"#).expect("static regex");
    let href_re = Regex::new(r#"(?i)href\s*=\s*"([^"]*)""#).expect("static regex");
    let date_re = Regex::new(r#"(?i)add_date\s*=\s*"(\d+)""#).expect("static regex");

    let mut out = Vec::new();
    for cap in anchor_re.captures_iter(html) {
        let attrs = &cap[1];
        let inner = &cap[2];
        let Some(href) = href_re.captures(attrs).and_then(|c| c.get(1)) else {
            continue;
        };
        let url = href.as_str().trim();
        if url.is_empty() {
            continue;
        }
        let lower = url.to_ascii_lowercase();
        // Skip feed:// and place: pseudo-urls that Firefox writes for folders.
        if !(lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("ftp://")
            || lower.starts_with("file://"))
        {
            continue;
        }
        let title = strip_tags(inner).trim().to_string();
        let add_date = date_re
            .captures(attrs)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0);
        out.push((url.to_string(), title, add_date));
    }
    out
}

/// Drop any nested tags from a bookmark title; browsers sometimes
/// wrap emoji or `<b>` inside `<a>...</a>`.
fn strip_tags(s: &str) -> String {
    let tag_re = Regex::new(r#"<[^>]*>"#).expect("static regex");
    let without = tag_re.replace_all(s, "");
    // Minimal HTML entity decode for the handful of entities browsers emit.
    without
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chrome_export() {
        let html = r#"
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<DL><p>
    <DT><A HREF="https://rust-lang.org/" ADD_DATE="1700000000">Rust</A>
    <DT><A HREF="https://docs.rs/" ADD_DATE="1700000100">docs.rs</A>
</DL>"#;
        let got = parse_netscape_bookmarks(html);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "https://rust-lang.org/");
        assert_eq!(got[0].1, "Rust");
        assert_eq!(got[0].2, 1700000000);
    }

    #[test]
    fn skips_folder_placeholders_and_non_http() {
        let html = r#"
<DT><H3>My Folder</H3>
<DT><A HREF="place:type=6">Folder view</A>
<DT><A HREF="">empty</A>
<DT><A HREF="https://ok.example">ok</A>
"#;
        let got = parse_netscape_bookmarks(html);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "https://ok.example");
    }

    #[test]
    fn decodes_common_entities_and_nested_tags() {
        let html = r#"<DT><A HREF="https://a.example">AT&amp;T <b>home</b></A>"#;
        let got = parse_netscape_bookmarks(html);
        assert_eq!(got[0].1, "AT&T home");
    }

    #[test]
    fn missing_add_date_yields_zero() {
        let html = r#"<DT><A HREF="https://a.example">x</A>"#;
        let got = parse_netscape_bookmarks(html);
        assert_eq!(got[0].2, 0);
    }
}
