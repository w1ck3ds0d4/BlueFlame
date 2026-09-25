//! Per-origin approval gate.
//!
//! The first tool call that touches a given origin (scheme + host + port)
//! blocks until Daniel answers an in-app prompt. His answer is remembered
//! for the rest of the session (and persisted across restarts) so he is
//! not asked twice for the same site.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ApprovalFile {
    #[serde(default)]
    origins: HashMap<String, Decision>,
}

pub struct ApprovalGate {
    path: Option<PathBuf>,
    state: Mutex<HashMap<String, Decision>>,
}

impl ApprovalGate {
    /// Load persisted decisions from `<app_data>/control/approvals.json`,
    /// if any. A missing or unreadable file just starts empty; nothing is
    /// pre-approved by default.
    pub fn open(app_data: &Path) -> Self {
        let path = app_data.join("control").join("approvals.json");
        let origins = std::fs::read_to_string(&path)
            .ok()
            .and_then(|body| serde_json::from_str::<ApprovalFile>(&body).ok())
            .map(|f| f.origins)
            .unwrap_or_default();
        Self {
            path: Some(path),
            state: Mutex::new(origins),
        }
    }

    /// An in-memory-only gate, for tests and for the non-Windows stub
    /// control channel (there's no pipe to serve there yet, so nothing to
    /// persist). Never touches disk.
    #[cfg(any(test, not(windows)))]
    pub fn in_memory() -> Self {
        Self {
            path: None,
            state: Mutex::new(HashMap::new()),
        }
    }

    pub fn decision(&self, origin: &str) -> Option<Decision> {
        self.state.lock().ok()?.get(origin).copied()
    }

    /// Record Daniel's answer for `origin` and persist it. A poisoned lock
    /// or an unwritable disk both fail soft: the in-memory decision still
    /// takes effect for the rest of this session.
    pub fn remember(&self, origin: &str, decision: Decision) {
        let snapshot = {
            let Ok(mut guard) = self.state.lock() else {
                return;
            };
            guard.insert(origin.to_string(), decision);
            guard.clone()
        };
        self.persist(&snapshot);
    }

    fn persist(&self, origins: &HashMap<String, Decision>) {
        let Some(path) = &self.path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(body) = serde_json::to_string_pretty(&ApprovalFile {
            origins: origins.clone(),
        }) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Best-effort `scheme://host[:port]` extraction used as the approval key.
/// Anything that fails to parse as a URL is treated as its own unique
/// origin string so it still gates rather than silently passing through.
pub fn origin_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let scheme = parsed.scheme();
            let host = parsed.host_str().unwrap_or("");
            match parsed.port() {
                Some(port) => format!("{scheme}://{host}:{port}"),
                None => format!("{scheme}://{host}"),
            }
        }
        Err(_) => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_origin_has_no_decision() {
        let gate = ApprovalGate::in_memory();
        assert_eq!(gate.decision("https://example.com"), None);
    }

    #[test]
    fn remembers_allow() {
        let gate = ApprovalGate::in_memory();
        gate.remember("https://example.com", Decision::Allow);
        assert_eq!(gate.decision("https://example.com"), Some(Decision::Allow));
    }

    #[test]
    fn remembers_deny_independently_per_origin() {
        let gate = ApprovalGate::in_memory();
        gate.remember("https://a.example", Decision::Deny);
        assert_eq!(gate.decision("https://a.example"), Some(Decision::Deny));
        assert_eq!(gate.decision("https://b.example"), None);
    }

    #[test]
    fn persists_and_reloads_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        {
            let gate = ApprovalGate::open(dir.path());
            gate.remember("https://example.com", Decision::Allow);
        }
        let reopened = ApprovalGate::open(dir.path());
        assert_eq!(
            reopened.decision("https://example.com"),
            Some(Decision::Allow)
        );
    }

    #[test]
    fn origin_of_normalizes_scheme_host_port() {
        assert_eq!(
            origin_of("https://example.com/a/b?x=1"),
            "https://example.com"
        );
        assert_eq!(
            origin_of("http://example.com:8080/x"),
            "http://example.com:8080"
        );
    }

    #[test]
    fn origin_of_unparseable_url_falls_back_to_itself() {
        assert_eq!(origin_of("not a url"), "not a url");
    }
}
