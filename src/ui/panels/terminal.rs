//! The embedded terminal of the selected application's last run.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, BorderType, Borders, Widget};
use tui_term::widget::{Cursor, PseudoTerminal};

use crate::jobs::RunRecord;
use crate::ui::style::styles;
use crate::ui::text::truncate_chars;

pub struct TerminalPanel<'a> {
    pub app_name: Option<&'a str>,
    pub record: Option<&'a RunRecord>,
    pub running: bool,
    pub focused: bool,
    pub detail: Option<&'a str>,
}

impl TerminalPanel<'_> {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let state = match (self.running, self.record.and_then(|r| r.outcome)) {
            (true, _) => " · running".to_string(),
            (false, Some(o)) => format!(" · {}", o.label()),
            _ => String::new(),
        };
        let title = match self.app_name {
            Some(n) => format!(" Logs · {n}{state} "),
            None => " Logs ".to_string(),
        };
        let block = Block::default()
            .title(truncate_chars(
                &title,
                area.width.saturating_sub(2) as usize,
            ))
            .title_style(styles::block_title_style(self.focused))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles::border_style(self.focused));

        match self.record {
            // Only cells are drawn: the title, OSC 52 and every other sequence the
            // script sent stay inside the parser and never reach the real terminal.
            Some(r) => r.screen.with_screen(|s| {
                // A cursor only where typing goes: a live run with the keys.
                let cursor = Cursor::default().visibility(self.running && self.focused);
                PseudoTerminal::new(s)
                    .block(block)
                    .cursor(cursor)
                    .render(area, buf);
            }),
            None => {
                let inner = block.inner(area);
                block.render(area, buf);
                let w = inner.width.saturating_sub(2) as usize;
                let mut lines = vec!["No run yet. Press u to update.".to_string()];
                if let Some(d) = self.detail {
                    lines.push(String::new());
                    lines.push(format!("Last check: {d}"));
                }
                for (i, l) in lines.iter().enumerate().take(inner.height as usize) {
                    buf.set_string(
                        inner.x + 1,
                        inner.y + i as u16,
                        truncate_chars(l, w),
                        styles::muted_style(),
                    );
                }
            }
        }
    }
}
