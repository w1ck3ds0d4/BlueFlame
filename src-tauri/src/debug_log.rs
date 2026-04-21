//! In-memory debug log exposed to the UI.
//!
//! Captures `tracing` events (info/warn/error) into a bounded ring buffer so
//! the frontend Debug view can render them live. Also receives events pushed
//! explicitly from the frontend via a `log_from_frontend` command so JS errors
//! end up in the same feed.

use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

const CAPACITY: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct DebugEntry {
    /// Unix epoch seconds of when we observed the event.
    pub ts: u64,
    /// Lowercase level: `error` / `warn` / `info` / `debug` / `trace` / `frontend`.
    pub level: String,
    /// `tracing` target (typically a module path) or "frontend" for UI events.
    pub target: String,
    /// Human-readable message.
    pub message: String,
}

#[derive(Default)]
pub struct DebugLog {
    inner: RwLock<VecDeque<DebugEntry>>,
}

impl DebugLog {
    pub fn push(&self, entry: DebugEntry) {
        let mut q = self.inner.write().expect("debug log rwlock poisoned");
        if q.len() >= CAPACITY {
            q.pop_front();
        }
        q.push_back(entry);
    }

    pub fn recent(&self, limit: usize) -> Vec<DebugEntry> {
        let q = self.inner.read().expect("debug log rwlock poisoned");
        let take = limit.min(q.len());
        q.iter().skip(q.len() - take).cloned().collect()
    }

    pub fn clear(&self) {
        let mut q = self.inner.write().expect("debug log rwlock poisoned");
        q.clear();
    }
}

pub type SharedDebugLog = Arc<DebugLog>;

/// `tracing_subscriber::Layer` that fans events out into our ring buffer in
/// addition to whatever other layers are installed (fmt prints to stderr).
pub struct DebugLogLayer {
    log: SharedDebugLog,
}

impl DebugLogLayer {
    pub fn new(log: SharedDebugLog) -> Self {
        Self { log }
    }
}

impl<S> Layer<S> for DebugLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = match *meta.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            tracing::Level::DEBUG => "debug",
            tracing::Level::TRACE => "trace",
        };
        let mut visitor = StringVisitor::default();
        event.record(&mut visitor);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.log.push(DebugEntry {
            ts: now,
            level: level.to_string(),
            target: meta.target().to_string(),
            message: visitor.finish(),
        });
    }
}

/// Flattens a tracing event's fields into a single string. The primary field
/// is `message`; other fields are appended as `key=value` pairs.
#[derive(Default)]
struct StringVisitor {
    message: String,
    fields: String,
}

impl StringVisitor {
    fn finish(self) -> String {
        if self.fields.is_empty() {
            self.message
        } else if self.message.is_empty() {
            self.fields
        } else {
            format!("{} {}", self.message, self.fields)
        }
    }
}

impl Visit for StringVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            if !self.fields.is_empty() {
                self.fields.push(' ');
            }
            let _ = write!(self.fields, "{}={value}", field.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_respects_capacity() {
        let log = DebugLog::default();
        for i in 0..CAPACITY + 10 {
            log.push(DebugEntry {
                ts: i as u64,
                level: "info".into(),
                target: "t".into(),
                message: format!("m{i}"),
            });
        }
        let all = log.recent(1000);
        assert_eq!(all.len(), CAPACITY);
        // Oldest entries were evicted, newest are at the end.
        assert_eq!(all.last().unwrap().message, format!("m{}", CAPACITY + 9));
    }

    #[test]
    fn recent_returns_tail_in_order() {
        let log = DebugLog::default();
        for i in 0..5 {
            log.push(DebugEntry {
                ts: i,
                level: "info".into(),
                target: "t".into(),
                message: format!("m{i}"),
            });
        }
        let tail = log.recent(3);
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].message, "m2");
        assert_eq!(tail[2].message, "m4");
    }

    #[test]
    fn clear_wipes_all() {
        let log = DebugLog::default();
        log.push(DebugEntry {
            ts: 0,
            level: "info".into(),
            target: "t".into(),
            message: "m".into(),
        });
        log.clear();
        assert!(log.recent(10).is_empty());
    }
}
