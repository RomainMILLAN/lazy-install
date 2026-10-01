use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::components::Hint;
use crate::ui::keys::{merged, KeyMap};
use crate::ui::style::{styles, theme};

/// Contextual hints on the bottom row. Holds nothing: the hints are derived from
/// the bindings at render time, never written out.
pub struct StatusBar;

/// Terminal columns, not bytes.
fn cols(s: &str) -> u16 {
    s.chars().count() as u16
}

impl StatusBar {
    pub fn render(area: Rect, buf: &mut Buffer, hints: &[Hint]) {
        let bg = styles::bar_style();
        for x in area.x..area.x + area.width {
            buf.set_string(x, area.y, " ", bg);
        }
        let key_style = styles::key_style().bg(theme::color_surface());
        let right = area.x + area.width;
        let mut x = area.x + 1;
        // A hint that would be cut by the right edge is dropped whole.
        for hint in hints {
            let needed = cols(&hint.key) + 1 + cols(&hint.desc);
            if x + needed > right {
                break;
            }
            buf.set_string(x, area.y, &hint.key, key_style);
            x += cols(&hint.key) + 1;
            buf.set_string(x, area.y, &hint.desc, bg);
            x += cols(&hint.desc) + 2;
        }
    }
}

/// The list screen, by usefulness: the bar drops overflow from the right.
pub fn list_hints(km: &KeyMap) -> Vec<Hint> {
    vec![
        km.update.hint(),
        km.update_all.hint(),
        merged(&km.up, &km.down, "j/k", "move"),
        km.add.hint(),
        km.edit.hint(),
        km.delete.hint(),
        km.check.hint(),
        km.check_all.hint(),
        km.focus_terminal.hint_as("logs"),
        km.filter.hint(),
        km.help.hint(),
        km.quit.hint(),
    ]
}

/// While typing in the inline filter.
pub fn filter_hints(km: &KeyMap) -> Vec<Hint> {
    vec![
        Hint::new("type", "filter"),
        Hint::new("enter", "keep"),
        km.escape.hint_as("clear"),
        Hint::new("\u{2191}/\u{2193}", "move"),
    ]
}

/// While the terminal has focus every key goes to the script, except these.
pub fn terminal_hints(km: &KeyMap) -> Vec<Hint> {
    vec![
        km.focus_list.hint(),
        km.scroll_up.hint(),
        km.scroll_down.hint(),
        Hint::new("keys", "→ script"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::keys::default_key_map;

    #[test]
    fn every_hint_is_bound() {
        let km = default_key_map();
        for hint in list_hints(&km)
            .into_iter()
            .chain(terminal_hints(&km).into_iter().take(3))
        {
            assert!(km.names_only_bound_keys(&hint.key), "{hint:?}");
        }
    }

    #[test]
    fn hints_are_never_cut_mid_word() {
        let km = default_key_map();
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        StatusBar::render(area, &mut buf, &list_hints(&km));
        let row: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(row.contains("update all"), "{row}");
        assert!(!row.contains("quit"), "{row}");
    }

    #[test]
    fn the_whole_list_bar_fits_120_columns() {
        let km = default_key_map();
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar::render(area, &mut buf, &list_hints(&km));
        let row: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(row.contains("quit"), "{row}");
    }
}
