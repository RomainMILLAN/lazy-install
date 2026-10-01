//! What the screen looks like right now: no domain state here.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    List,
    Terminal,
}

/// After focus comes back to the list on its own, keys are ignored this long: a
/// password being typed for sudo must not land in the filter or press `u`.
const FOCUS_GUARD: Duration = Duration::from_millis(300);

/// What a key did to the inline filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOutcome {
    /// The text changed: the list is re-filtered and the cursor goes to the top.
    Changed,
    /// ↑/↓ while typing: move in the filtered list.
    Navigate {
        up: bool,
    },
    /// Left the filter (enter keeps it, esc and backspace-on-empty clear it).
    Exited,
    Ignored,
}

pub struct ViewState {
    pub focus: Focus,
    pub filter: String,
    /// Typing goes to the filter, as in lazy-transfer: no modal.
    pub filtering: bool,
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
            filtering: false,
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

    pub fn start_filter(&mut self) {
        self.filtering = true;
    }

    /// The inline filter: every printable key types, the list follows live.
    pub fn filter_key(&mut self, key: KeyEvent) -> FilterOutcome {
        match key.code {
            KeyCode::Esc => {
                self.filtering = false;
                self.filter.clear();
                self.selected = 0;
                FilterOutcome::Exited
            }
            KeyCode::Enter => {
                self.filtering = false;
                FilterOutcome::Exited
            }
            KeyCode::Backspace => {
                if self.filter.pop().is_none() {
                    self.filtering = false;
                    return FilterOutcome::Exited;
                }
                self.selected = 0;
                FilterOutcome::Changed
            }
            KeyCode::Up => FilterOutcome::Navigate { up: true },
            KeyCode::Down => FilterOutcome::Navigate { up: false },
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.filter.push(c);
                self.selected = 0;
                FilterOutcome::Changed
            }
            _ => FilterOutcome::Ignored,
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

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_filters_live_and_enter_keeps_it() {
        let mut v = ViewState::new();
        v.start_filter();
        v.selected = 3;
        assert_eq!(v.filter_key(k(KeyCode::Char('k'))), FilterOutcome::Changed);
        assert_eq!(v.filter_key(k(KeyCode::Char('i'))), FilterOutcome::Changed);
        assert_eq!((v.filter.as_str(), v.selected), ("ki", 0));
        // `u`, `q`, `d`… are text while filtering, never actions
        assert_eq!(v.filter_key(k(KeyCode::Char('q'))), FilterOutcome::Changed);
        assert_eq!(
            v.filter_key(k(KeyCode::Down)),
            FilterOutcome::Navigate { up: false }
        );
        assert_eq!(v.filter_key(k(KeyCode::Enter)), FilterOutcome::Exited);
        assert!(!v.filtering);
        assert_eq!(v.filter, "kiq");
    }

    #[test]
    fn esc_clears_and_backspace_on_empty_leaves() {
        let mut v = ViewState::new();
        v.start_filter();
        v.filter_key(k(KeyCode::Char('x')));
        assert_eq!(v.filter_key(k(KeyCode::Esc)), FilterOutcome::Exited);
        assert!(!v.filtering && v.filter.is_empty());

        v.start_filter();
        v.filter_key(k(KeyCode::Char('x')));
        assert_eq!(v.filter_key(k(KeyCode::Backspace)), FilterOutcome::Changed);
        assert_eq!(v.filter_key(k(KeyCode::Backspace)), FilterOutcome::Exited);
        assert!(!v.filtering);
    }

    #[test]
    fn ctrl_keys_do_not_type() {
        let mut v = ViewState::new();
        v.start_filter();
        let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
        assert_eq!(v.filter_key(ctrl_l), FilterOutcome::Ignored);
        assert!(v.filter.is_empty());
    }

    #[test]
    fn no_guard_when_focus_was_already_on_the_list() {
        let mut v = ViewState::new();
        v.update_ended();
        assert!(!v.ignoring_keys());
    }
}
