//! The list of applications.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Widget};

use crate::catalog::Tag;
use crate::ui::style::{styles, theme};
use crate::ui::text::{fit, truncate_chars};

/// One line of the list, already projected from the session.
pub struct Row {
    pub name: String,
    pub tag: Tag,
    pub versions: String,
    pub detail: Option<String>,
}

pub struct AppsPanel<'a> {
    pub rows: &'a [Row],
    pub selected: usize,
    pub focused: bool,
    pub filter: &'a str,
    /// Typing goes to the filter right now (cursor shown, warning border).
    pub filtering: bool,
    pub total: usize,
    pub spinner: char,
}

impl AppsPanel<'_> {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        // As in lazy-transfer: the filter lives in the title, `/text█` while typing.
        let count = if self.filter.is_empty() {
            format!("{}", self.total)
        } else {
            format!("{}/{}", self.rows.len(), self.total)
        };
        let filter_text = match (self.filtering, self.filter.is_empty()) {
            (true, _) => format!(" /{}█", self.filter),
            (false, false) => format!(" /{}", self.filter),
            (false, true) => String::new(),
        };
        let border = if self.filtering {
            Style::default().fg(theme::color_warning())
        } else {
            styles::border_style(self.focused)
        };
        let block = Block::default()
            .title(format!(" Apps ({count}){filter_text} "))
            .title_style(styles::block_title_style(self.focused))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border);
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height == 0 || inner.width < 10 {
            return;
        }
        if self.rows.is_empty() {
            let msg = if self.total == 0 {
                "No application yet. Press a to add one."
            } else {
                "Nothing matches the filter."
            };
            buf.set_string(
                inner.x + 1,
                inner.y,
                truncate_chars(msg, inner.width as usize - 2),
                styles::muted_style(),
            );
            return;
        }

        let width = inner.width as usize - 1;
        let name_w = (width / 3).clamp(8, 24);
        let tag_w = 12;
        let rest_w = width.saturating_sub(name_w + tag_w + 2);
        let height = inner.height as usize;
        let offset = self.selected.saturating_sub(height.saturating_sub(1));

        for (i, row) in self.rows.iter().enumerate().skip(offset).take(height) {
            let y = inner.y + (i - offset) as u16;
            let selected = i == self.selected;
            let base = if selected {
                styles::selected_style(self.focused)
            } else {
                styles::description_style()
            };
            buf.set_string(inner.x, y, " ".repeat(inner.width as usize), base);
            buf.set_string(inner.x + 1, y, fit(&row.name, name_w), base);

            let label = match row.tag {
                Tag::Checking | Tag::Updating => {
                    format!("{} {}", self.spinner, theme::tag_label(row.tag))
                }
                t => theme::tag_label(t).to_string(),
            };
            let tag_style = if selected {
                base
            } else {
                Style::default()
                    .fg(theme::tag_color(row.tag))
                    .add_modifier(Modifier::BOLD)
            };
            let tag_x = inner.x + 1 + name_w as u16 + 1;
            buf.set_string(tag_x, y, fit(&label, tag_w), tag_style);

            let tail = match (&row.versions, &row.detail) {
                (v, Some(d)) if !v.is_empty() => format!("{v}  {d}"),
                (_, Some(d)) => d.clone(),
                (v, None) => v.clone(),
            };
            let tail_style = if selected {
                base
            } else {
                styles::muted_style()
            };
            buf.set_string(
                tag_x + tag_w as u16 + 1,
                y,
                truncate_chars(&tail, rest_w),
                tail_style,
            );
        }
    }
}
