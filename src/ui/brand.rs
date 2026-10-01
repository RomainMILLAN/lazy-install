//! The product name, as it appears on top of the screen. The logo itself is the
//! real PNG (`docs/assets/logo-mark*.png`), drawn by [`crate::ui::logo`] in the
//! area this module reserves for it — never redrawn with characters.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::ui::style::theme;

/// Canonical spelling: lowercase, hyphenated. Also the repository name.
pub const NAME: &str = "lazy-install";

pub const TAGLINE: &str = "what to update · one key to install";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The logo's cells. 8×4 is square on a usual 1:2 terminal cell.
pub const LOGO_W: u16 = 8;
pub const LOGO_H: u16 = 4;

/// Columns between the logo and the wordmark.
const GAP: u16 = 3;

/// Rows the header takes: a blank row, the logo, a blank row.
pub const HEADER_H: u16 = LOGO_H + 2;

fn cols(s: &str) -> u16 {
    s.chars().count() as u16
}

/// Columns the logo and the text need.
pub fn block_w() -> u16 {
    let name = cols(NAME) + 1 + cols(VERSION) + 1;
    LOGO_W + GAP + name.max(cols(TAGLINE))
}

/// Where the logo goes inside the header.
pub fn logo_area(header: Rect) -> Rect {
    Rect::new(header.x + 2, header.y + 1, LOGO_W, LOGO_H)
}

/// The counts shown at the right of the header, already projected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub total: usize,
    pub updates: usize,
    pub problems: usize,
    pub busy: usize,
}

impl Summary {
    /// `15 apps · 3 to update · 1 error · 2 running`, zero counts omitted.
    fn parts(&self) -> Vec<(String, Style)> {
        let muted = Style::default().fg(theme::color_muted());
        let mut out = vec![(
            format!(
                "{} app{}",
                self.total,
                if self.total == 1 { "" } else { "s" }
            ),
            Style::default().fg(theme::color_text()),
        )];
        let mut push = |n: usize, label: &str, color: Color| {
            if n > 0 {
                out.push((" · ".to_string(), muted));
                out.push((
                    format!("{n} {label}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            }
        };
        push(self.updates, "to update", theme::color_warning());
        push(
            self.problems,
            if self.problems == 1 {
                "error"
            } else {
                "errors"
            },
            theme::color_danger(),
        );
        push(self.busy, "running", theme::color_info());
        out
    }
}

/// The header above the panels, as in lazy-transfer: the logo (drawn by the
/// caller in [`logo_area`]), the wordmark and the tagline beside it, the counts
/// on the right — dropped first when the terminal is narrow.
pub fn render_header(area: Rect, buf: &mut Buffer, summary: &Summary) {
    let logo = logo_area(area);
    let text_x = logo.x + LOGO_W + GAP;
    // Wordmark and tagline on the logo's two middle rows.
    let name_y = logo.y + 1;
    buf.set_string(
        text_x,
        name_y,
        NAME,
        Style::default()
            .fg(theme::color_bright())
            .add_modifier(Modifier::BOLD),
    );
    buf.set_string(
        text_x + cols(NAME) + 1,
        name_y,
        format!("v{VERSION}"),
        Style::default().fg(theme::color_primary()),
    );
    buf.set_string(
        text_x,
        name_y + 1,
        TAGLINE,
        Style::default().fg(theme::color_muted()),
    );

    let parts = summary.parts();
    let width: u16 = parts.iter().map(|(t, _)| cols(t)).sum();
    let left_end = logo.x + block_w();
    let right = area.x + area.width;
    if right >= left_end + 4 + width + 2 {
        let mut cx = right - 2 - width;
        for (text, style) in parts {
            buf.set_string(cx, name_y, &text, style);
            cx += cols(&text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn header_shows_name_tagline_and_counts() {
        let summary = Summary {
            total: 15,
            updates: 3,
            problems: 1,
            busy: 0,
        };
        let area = Rect::new(0, 0, 120, HEADER_H);
        let mut buf = Buffer::empty(area);
        render_header(area, &mut buf, &summary);
        let name = row(&buf, logo_area(area).y + 1);
        assert!(name.contains(&format!("lazy-install v{VERSION}")), "{name}");
        assert!(
            name.trim_end().ends_with("15 apps · 3 to update · 1 error"),
            "{name}"
        );
        assert!(row(&buf, logo_area(area).y + 2).contains(TAGLINE));
    }

    #[test]
    fn the_logo_area_is_left_free_for_the_image() {
        let area = Rect::new(0, 0, 120, HEADER_H);
        let mut buf = Buffer::empty(area);
        render_header(area, &mut buf, &Summary::default());
        let logo = logo_area(area);
        for y in logo.y..logo.y + logo.height {
            for x in logo.x..logo.x + logo.width {
                assert_eq!(buf[(x, y)].symbol(), " ", "({x},{y}) is drawn over");
            }
        }
    }

    #[test]
    fn counts_are_dropped_when_narrow() {
        let summary = Summary {
            total: 15,
            ..Summary::default()
        };
        let area = Rect::new(0, 0, block_w() + 6, HEADER_H);
        let mut buf = Buffer::empty(area);
        render_header(area, &mut buf, &summary);
        assert!(!row(&buf, logo_area(area).y + 1).contains("apps"));
    }
}
