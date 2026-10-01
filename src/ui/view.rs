//! What the screen looks like right now: no domain state here.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Terminal,
}

/// After focus comes back to the list on its own, keys are ignored this long: a
/// password being typed for sudo must not land in the filter or press `u`.
const FOCUS_GUARD: Duration = Duration::from_millis(300);

pub struct ViewState {
    pub focus: Focus,
    pub filter: String,
    pub selected: usize,
    ignore_until: Option<Instant>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewState {
    pub fn new() -> Self {
        ViewState {
            focus: Focus::List,
            filter: String::new(),
            selected: 0,
            ignore_until: None,
        }
    }

    /// An update has started: its terminal takes the keys.
    pub fn update_started(&mut self) {
        self.focus = Focus::Terminal;
    }

    /// An update is over: back to the list, with a short guard.
    pub fn update_ended(&mut self) {
        if self.focus == Focus::Terminal {
            self.focus = Focus::List;
            self.ignore_until = Some(Instant::now() + FOCUS_GUARD);
        }
    }

    pub fn ignoring_keys(&self) -> bool {
        self.ignore_until.is_some_and(|t| Instant::now() < t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_returns_with_a_guard() {
        let mut v = ViewState::new();
        v.update_started();
        assert_eq!(v.focus, Focus::Terminal);
        v.update_ended();
        assert_eq!(v.focus, Focus::List);
        assert!(v.ignoring_keys());
    }

    #[test]
    fn no_guard_when_focus_was_already_on_the_list() {
        let mut v = ViewState::new();
        v.update_ended();
        assert!(!v.ignoring_keys());
    }
}
