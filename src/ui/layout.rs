//! Where things go, computed as values so tests check the real geometry.

use ratatui::layout::Rect;

use super::brand;

/// Below this width the two panels stack.
pub const SIDE_BY_SIDE_MIN_WIDTH: u16 = 100;

/// Below this height the header folds away: a short terminal needs every row
/// for the list and the logs.
pub const HEADER_MIN_HEIGHT: u16 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Layout {
    /// The brand header, when the terminal is tall and wide enough.
    pub header: Option<Rect>,
    pub list: Rect,
    pub terminal: Rect,
    /// One row above the status bar, only when asked for.
    pub banner: Option<Rect>,
    pub status: Rect,
}

impl Layout {
    /// The size of the terminal panel without its border: the PTY size.
    pub fn pty_size(&self) -> (u16, u16) {
        (
            self.terminal.height.saturating_sub(2).max(1),
            self.terminal.width.saturating_sub(2).max(1),
        )
    }
}

pub fn compute_layout(width: u16, height: u16, banner: bool) -> Layout {
    let status = Rect::new(0, height.saturating_sub(1), width, 1.min(height));
    let banner_h = u16::from(banner && height > 4);
    let body_h = height.saturating_sub(1 + banner_h);
    let banner = (banner_h == 1).then(|| Rect::new(0, body_h, width, 1));

    let header_fits = height >= HEADER_MIN_HEIGHT && width >= brand::block_w() + 4;
    let top = if header_fits { brand::HEADER_H } else { 0 };
    let header = header_fits.then(|| Rect::new(0, 0, width, top));
    let body_h = body_h.saturating_sub(top);

    let (list, terminal) = if width >= SIDE_BY_SIDE_MIN_WIDTH {
        let list_w = (width * 2 / 5).max(36).min(width);
        (
            Rect::new(0, top, list_w, body_h),
            Rect::new(list_w, top, width - list_w, body_h),
        )
    } else {
        let list_h = (body_h * 2 / 5).max(3).min(body_h);
        (
            Rect::new(0, top, width, list_h),
            Rect::new(0, top + list_h, width, body_h - list_h),
        )
    };
    Layout {
        header,
        list,
        terminal,
        banner,
        status,
    }
}

/// A rectangle of `w` × `h` centred in `area`, shrunk to fit.
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_by_side_when_wide() {
        let l = compute_layout(120, 40, false);
        assert_eq!(l.list.y, l.terminal.y);
        assert_eq!(l.list.width + l.terminal.width, 120);
        assert_eq!(l.status.y, 39);
        assert!(l.banner.is_none());
    }

    #[test]
    fn stacked_when_narrow() {
        let l = compute_layout(80, 40, true);
        assert_eq!(l.list.x, l.terminal.x);
        assert!(l.terminal.y > l.list.y);
        assert_eq!(l.banner.unwrap().y, 38);
        assert_eq!(l.terminal.y + l.terminal.height, 38);
    }

    #[test]
    fn the_header_sits_on_top_and_folds_away_when_short() {
        let l = compute_layout(120, 40, false);
        let h = l.header.expect("a 40-row terminal has a header");
        assert_eq!((h.y, h.height), (0, brand::HEADER_H));
        assert_eq!(l.list.y, brand::HEADER_H);
        assert_eq!(l.terminal.y + l.terminal.height, 39);

        let short = compute_layout(120, HEADER_MIN_HEIGHT - 1, false);
        assert!(short.header.is_none());
        assert_eq!(short.list.y, 0);
    }

    #[test]
    fn tiny_terminals_do_not_underflow() {
        let l = compute_layout(5, 2, true);
        let _ = l.pty_size();
    }
}
