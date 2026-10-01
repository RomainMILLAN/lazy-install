use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::ui::keys::KeyMap;
use crate::ui::style::styles;
use crate::ui::text::truncate_chars;

/// The keys, from the bindings themselves, and the contract in a few lines.
pub struct HelpPopup {
    visible: bool,
}

impl Default for HelpPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpPopup {
    pub fn new() -> Self {
        HelpPopup { visible: false }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, km: &KeyMap) {
        if !self.visible {
            return;
        }
        Clear.render(area, buf);
        let block = Block::default()
            .title(" Help ")
            .title_style(styles::block_title_style(true))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles::border_style(true));
        let inner = block.inner(area);
        block.render(area, buf);

        let mut lines: Vec<(String, String)> = km
            .all_named()
            .into_iter()
            .map(|(_, b)| {
                let h = b.hint();
                (h.key, h.desc)
            })
            .collect();
        lines.push((String::new(), String::new()));
        for text in [
            "A script defines needs_update() and update(), with the",
            "marker line \"# lazy-install: v1\". needs_update must end",
            "with li_update_available or li_up_to_date [inst] [latest].",
            "Template: lazy-install --template",
        ] {
            lines.push((String::new(), text.to_string()));
        }
        let w = inner.width.saturating_sub(2) as usize;
        for (i, (k, d)) in lines.iter().enumerate().take(inner.height as usize) {
            let y = inner.y + i as u16;
            if k.is_empty() {
                buf.set_string(inner.x + 1, y, truncate_chars(d, w), styles::muted_style());
            } else {
                buf.set_string(inner.x + 1, y, format!("{k:>8}"), styles::key_style());
                buf.set_string(
                    inner.x + 11,
                    y,
                    truncate_chars(d, w.saturating_sub(10)),
                    styles::description_style(),
                );
            }
        }
    }
}
