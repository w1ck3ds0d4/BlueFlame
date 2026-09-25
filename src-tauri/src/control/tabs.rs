//! Tracks which open tabs Claude is allowed to touch.
//!
//! Claude only ever sees and acts on tabs it opened itself (always InPrivate,
//! see `control::dispatch::open_tab`). `list_tabs` only returns members of
//! this set, and every other tool call is rejected for a tab id that is not
//! in it, so a `navigate`/`click`/`type` call can never reach one of
//! Daniel's own tabs even if it guessed a valid id.

use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Default)]
pub struct ClaudeTabs(Mutex<HashSet<u64>>);

impl ClaudeTabs {
    pub fn mark(&self, id: u64) {
        if let Ok(mut set) = self.0.lock() {
            set.insert(id);
        }
    }

    pub fn unmark(&self, id: u64) {
        if let Ok(mut set) = self.0.lock() {
            set.remove(&id);
        }
    }

    pub fn is_claude_tab(&self, id: u64) -> bool {
        self.0.lock().map(|set| set.contains(&id)).unwrap_or(false)
    }

    pub fn list(&self) -> Vec<u64> {
        self.0
            .lock()
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmarked_tab_is_not_a_claude_tab() {
        let tabs = ClaudeTabs::default();
        assert!(!tabs.is_claude_tab(1));
    }

    #[test]
    fn marking_makes_it_visible() {
        let tabs = ClaudeTabs::default();
        tabs.mark(5);
        assert!(tabs.is_claude_tab(5));
        assert_eq!(tabs.list(), vec![5]);
    }

    #[test]
    fn unmarking_removes_it() {
        let tabs = ClaudeTabs::default();
        tabs.mark(5);
        tabs.unmark(5);
        assert!(!tabs.is_claude_tab(5));
        assert!(tabs.list().is_empty());
    }
}
