use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::components::Hint;

/// A keyboard binding: the keys it answers to, and how it presents itself.
///
/// `help_key`/`help_desc` are PRIVATE. Public, every surface recomposed its own
/// label from the pieces and the duplication merely moved down a level; that is
/// how the status bar came to advertise `d` for "download" while `d` deleted. Ask
/// the binding to describe itself with [`KeyBinding::hint`] instead.
pub struct KeyBinding {
    pub keys: Vec<KeyEvent>,
    help_key: String,
    help_desc: String,
}

impl KeyBinding {
    pub fn matches(&self, key: &KeyEvent) -> bool {
        self.keys
            .iter()
            .any(|k| k.code == key.code && k.modifiers == key.modifiers)
    }

    /// The binding presents itself. Nobody else has to glue its pieces together.
    pub fn hint(&self) -> Hint {
        Hint::new(&self.help_key, &self.help_desc)
    }

    /// The same, with a context-specific description. The binding still owns the
    /// key — only the wording changes, e.g. `copy_file` reads "upload" on the
    /// local pane and "download" on the remote one.
    pub fn hint_as(&self, desc: &str) -> Hint {
        Hint::new(&self.help_key, desc)
    }

    /// Whether `label` names only keys this binding answers to.
    ///
    /// A label is a human spelling of a key set (`"backspace/h"`), so it is the one
    /// place a typo cannot be caught by construction. Segments are split on `/`,
    /// and a segment naming a non-character key (`backspace`, `tab`, `enter`) is
    /// matched by name.
    pub fn spells(&self, label: &str) -> bool {
        // A one-character label IS the key, even when that character is the `/`
        // used as the separator — `search` advertises exactly "/".
        if label.chars().count() == 1 {
            return self.answers_to(label);
        }
        label.split('/').all(|seg| self.answers_to(seg.trim()))
    }

    fn answers_to(&self, seg: &str) -> bool {
        // Modifier notation: "Ctrl+L", "Shift+Tab". The last `+`-separated part
        // names the key, everything before it names modifiers.
        let mut mods = KeyModifiers::NONE;
        let mut name = seg;
        while let Some((prefix, rest)) = name.split_once('+') {
            match prefix.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
                "shift" => mods |= KeyModifiers::SHIFT,
                "alt" => mods |= KeyModifiers::ALT,
                _ => return false,
            }
            name = rest;
        }
        let name = name.trim();

        let code = match name {
            "backspace" => KeyCode::Backspace,
            "tab" => KeyCode::Tab,
            "pgup" => KeyCode::PageUp,
            "pgdn" => KeyCode::PageDown,
            "enter" => KeyCode::Enter,
            "esc" => KeyCode::Esc,
            "\u{2191}" => KeyCode::Up,
            "\u{2193}" => KeyCode::Down,
            "arrows" => {
                return self
                    .keys
                    .iter()
                    .any(|k| matches!(k.code, KeyCode::Up | KeyCode::Down))
            }
            _ => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return false,
                }
            }
        };

        self.keys.iter().any(|k| {
            let code_matches = match (k.code, code) {
                // A label spells a letter in whichever case reads best: the theme
                // toggle is bound to Ctrl+`l` and advertised as "Ctrl+L".
                (KeyCode::Char(a), KeyCode::Char(b)) => a.eq_ignore_ascii_case(&b),
                (a, b) => a == b,
            };
            code_matches && k.modifiers.contains(mods)
        })
    }
}

/// One hint from two bindings, for the rows that have no single label:
/// `up` + `down` display as "j/k".
///
/// `label` is free text, which is exactly why [`spells_merged_label`] checks it
/// against both bindings — a hand-written label is the last place drift can hide.
pub fn merged(a: &KeyBinding, b: &KeyBinding, label: &str, desc: &str) -> Hint {
    debug_assert!(
        spells_merged_label(a, b, label),
        "merged label {label:?} names a key neither binding answers to"
    );
    Hint::new(label, desc)
}

/// Whether every key named in `label` is answered by `a` or by `b`.
pub fn spells_merged_label(a: &KeyBinding, b: &KeyBinding, label: &str) -> bool {
    label
        .split('/')
        .all(|seg| a.spells(seg.trim()) || b.spells(seg.trim()))
}

/// All keybindings for lazy-install.
pub struct KeyMap {
    pub quit: KeyBinding,
    pub help: KeyBinding,
    pub up: KeyBinding,
    pub down: KeyBinding,
    pub update: KeyBinding,
    pub update_all: KeyBinding,
    pub add: KeyBinding,
    pub edit: KeyBinding,
    pub delete: KeyBinding,
    pub check: KeyBinding,
    pub check_all: KeyBinding,
    pub filter: KeyBinding,
    pub focus_terminal: KeyBinding,
    pub focus_list: KeyBinding,
    pub scroll_up: KeyBinding,
    pub scroll_down: KeyBinding,
    pub toggle_theme: KeyBinding,
    pub escape: KeyBinding,
}

impl KeyMap {
    /// Every binding with its field name. One list, so a test cannot silently
    /// cover fewer bindings than exist.
    pub fn all_named(&self) -> Vec<(&'static str, &KeyBinding)> {
        vec![
            ("quit", &self.quit),
            ("help", &self.help),
            ("up", &self.up),
            ("down", &self.down),
            ("update", &self.update),
            ("update_all", &self.update_all),
            ("add", &self.add),
            ("edit", &self.edit),
            ("delete", &self.delete),
            ("check", &self.check),
            ("check_all", &self.check_all),
            ("filter", &self.filter),
            ("focus_terminal", &self.focus_terminal),
            ("focus_list", &self.focus_list),
            ("scroll_up", &self.scroll_up),
            ("scroll_down", &self.scroll_down),
            ("toggle_theme", &self.toggle_theme),
            ("escape", &self.escape),
        ]
    }

    /// Whether every key named in `label` is answered by SOME binding.
    pub fn names_only_bound_keys(&self, label: &str) -> bool {
        let named = self.all_named();
        let reachable = |seg: &str| named.iter().any(|(_, b)| b.spells(seg));
        if label.chars().count() == 1 {
            return reachable(label);
        }
        label.split('/').all(|seg| reachable(seg.trim()))
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn key_shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

fn bind(keys: Vec<KeyEvent>, help_key: &str, help_desc: &str) -> KeyBinding {
    KeyBinding {
        keys,
        help_key: help_key.to_string(),
        help_desc: help_desc.to_string(),
    }
}

pub fn default_key_map() -> KeyMap {
    KeyMap {
        quit: bind(vec![key(KeyCode::Char('q'))], "q", "quit"),
        help: bind(vec![key(KeyCode::Char('?'))], "?", "help"),
        up: bind(
            vec![key(KeyCode::Char('k')), key(KeyCode::Up)],
            "k/\u{2191}",
            "up",
        ),
        down: bind(
            vec![key(KeyCode::Char('j')), key(KeyCode::Down)],
            "j/\u{2193}",
            "down",
        ),
        update: bind(vec![key(KeyCode::Char('u'))], "u", "update"),
        update_all: bind(vec![key_shift(KeyCode::Char('U'))], "U", "update all"),
        add: bind(vec![key(KeyCode::Char('a'))], "a", "add"),
        edit: bind(vec![key(KeyCode::Char('e'))], "e", "edit"),
        delete: bind(vec![key(KeyCode::Char('d'))], "d", "delete"),
        check: bind(vec![key(KeyCode::Char('r'))], "r", "check"),
        check_all: bind(vec![key_shift(KeyCode::Char('R'))], "R", "check all"),
        filter: bind(vec![key(KeyCode::Char('/'))], "/", "filter"),
        focus_terminal: bind(vec![key(KeyCode::Tab)], "tab", "terminal"),
        focus_list: bind(vec![key_ctrl(KeyCode::Char('o'))], "Ctrl+O", "back to list"),
        scroll_up: bind(vec![key(KeyCode::PageUp)], "pgup", "scroll up"),
        scroll_down: bind(vec![key(KeyCode::PageDown)], "pgdn", "scroll down"),
        toggle_theme: bind(vec![key_ctrl(KeyCode::Char('l'))], "Ctrl+L", "toggle theme"),
        escape: bind(vec![key(KeyCode::Esc)], "esc", "cancel"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_bindings_claim_the_same_key() {
        let km = default_key_map();
        let named = km.all_named();
        assert_eq!(named.len(), 18, "a binding is missing from all_named()");
        for (i, (name, a)) in named.iter().enumerate() {
            for (other, b) in &named[i + 1..] {
                for k in &a.keys {
                    assert!(!b.matches(k), "{name} and {other} both answer to {k:?}");
                }
            }
        }
    }

    #[test]
    fn every_label_spells_its_own_keys() {
        let km = default_key_map();
        for (name, b) in km.all_named() {
            let hint = b.hint();
            assert!(b.spells(&hint.key), "{name} advertises {:?}", hint.key);
        }
    }

    #[test]
    fn merged_labels_name_only_bound_keys() {
        let km = default_key_map();
        assert!(spells_merged_label(&km.up, &km.down, "j/k"));
        assert!(!spells_merged_label(&km.up, &km.down, "j/u"));
    }

    #[test]
    fn shift_and_ctrl_are_distinct() {
        let km = default_key_map();
        assert!(km.update_all.matches(&key_shift(KeyCode::Char('U'))));
        assert!(!km.update_all.matches(&key(KeyCode::Char('u'))));
        assert!(km.focus_list.matches(&key_ctrl(KeyCode::Char('o'))));
    }
}
