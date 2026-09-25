//! Action log for the Claude control channel.
//!
//! Every tool call the pipe dispatches is appended here: an in-memory ring
//! buffer for the Debug panel, and a JSON-lines file on disk Daniel can open
//! directly (`<app_data>/control/action-log.jsonl`) if he wants the full
//! history rather than just the last 200 entries.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

const RING_CAPACITY: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct ActionLogEntry {
    pub ts_ms: u64,
    pub tab_id: Option<u64>,
    pub tool: String,
    pub detail: String,
    pub outcome: String,
}

impl ActionLogEntry {
    pub fn new(tab_id: Option<u64>, tool: &str, detail: impl Into<String>, outcome: &str) -> Self {
        Self {
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            tab_id,
            tool: tool.to_string(),
            detail: detail.into(),
            outcome: outcome.to_string(),
        }
    }
}

pub struct ActionLog {
    path: Option<PathBuf>,
    recent: Mutex<VecDeque<ActionLogEntry>>,
}

impl ActionLog {
    pub fn open(app_data: &Path) -> Self {
        Self {
            path: Some(app_data.join("control").join("action-log.jsonl")),
            recent: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
        }
    }

    /// For tests and the non-Windows stub control channel: never touches disk.
    #[cfg(any(test, not(windows)))]
    pub fn in_memory() -> Self {
        Self {
            path: None,
            recent: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
        }
    }

    pub fn record(&self, entry: ActionLogEntry) {
        self.append_to_file(&entry);
        let Ok(mut ring) = self.recent.lock() else {
            return;
        };
        if ring.len() == RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(entry);
    }

    pub fn recent(&self, limit: usize) -> Vec<ActionLogEntry> {
        let Ok(ring) = self.recent.lock() else {
            return Vec::new();
        };
        ring.iter().rev().take(limit).cloned().collect()
    }

    fn append_to_file(&self, entry: &ActionLogEntry) {
        let Some(path) = &self.path else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(mut line) = serde_json::to_string(entry) else {
            return;
        };
        line.push('\n');
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_returns_newest_first() {
        let log = ActionLog::in_memory();
        log.record(ActionLogEntry::new(Some(1), "navigate", "a", "ok"));
        log.record(ActionLogEntry::new(Some(1), "click", "b", "ok"));
        let recent = log.recent(10);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].tool, "click");
        assert_eq!(recent[1].tool, "navigate");
    }

    #[test]
    fn ring_buffer_caps_at_capacity() {
        let log = ActionLog::in_memory();
        for i in 0..(RING_CAPACITY + 10) {
            log.record(ActionLogEntry::new(None, "tool", i.to_string(), "ok"));
        }
        assert_eq!(log.recent(usize::MAX).len(), RING_CAPACITY);
    }

    #[test]
    fn writes_jsonl_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let log = ActionLog::open(dir.path());
        log.record(ActionLogEntry::new(Some(7), "navigate", "https://x", "ok"));
        let body =
            std::fs::read_to_string(dir.path().join("control").join("action-log.jsonl")).unwrap();
        assert!(body.contains("\"tool\":\"navigate\""));
        assert!(body.trim_end().lines().count() == 1);
    }
}
