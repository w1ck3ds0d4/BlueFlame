//! Persist open tabs across app restarts. Serialized to a small JSON file
//! at `<app_data>/session.json`; read on boot, written after every tab
//! mutation. Data URLs (the new-tab speed dial) are excluded - they'd just
//! be replaced at boot anyway.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTab {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    pub tabs: Vec<PersistedTab>,
    /// Index (0-based) of the tab that was active, if any.
    pub active_index: Option<usize>,
}

fn path(app_data: &Path) -> PathBuf {
    app_data.join("session.json")
}

pub fn load(app_data: &Path) -> Option<Session> {
    let body = std::fs::read_to_string(path(app_data)).ok()?;
    serde_json::from_str(&body).ok()
}

pub fn save(app_data: &Path, session: &Session) -> anyhow::Result<()> {
    let p = path(app_data);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(session)?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}
