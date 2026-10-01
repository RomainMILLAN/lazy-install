//! The real logo in the terminal: `docs/assets/logo-mark*.png`, embedded in the
//! binary, drawn with whatever image protocol the terminal speaks (kitty,
//! sixel, iTerm2) and with unicode half-blocks where it speaks none.

use image::DynamicImage;
use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::widgets::Widget;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{FilterType, FontSize, Image, Resize};

use crate::ui::style::theme::{self, ThemeMode};

const DARK_PNG: &[u8] = include_bytes!("../../docs/assets/logo-mark.png");
const LIGHT_PNG: &[u8] = include_bytes!("../../docs/assets/logo-mark-light.png");

/// Kitty's graphics protocol where the terminal is known to speak it, outside
/// tmux; half-blocks (plain cells, works everywhere) otherwise.
fn picker_from_env(var: &dyn Fn(&str) -> String) -> Picker {
    let term = var("TERM");
    let in_tmux = !var("TMUX").is_empty() || term.starts_with("tmux") || term.starts_with("screen");
    let kitty_like = !var("KITTY_WINDOW_ID").is_empty()
        || term == "xterm-kitty"
        || term == "xterm-ghostty"
        || var("TERM_PROGRAM") == "ghostty";
    let font = crossterm::terminal::window_size()
        .ok()
        .filter(|w| w.width > 0 && w.height > 0 && w.columns > 0 && w.rows > 0)
        .map(|w| FontSize::new(w.width / w.columns, w.height / w.rows));
    match (kitty_like && !in_tmux, font) {
        (true, Some(font)) => {
            // Deprecated in favour of the stdin query this module avoids.
            #[allow(deprecated)]
            let mut picker = Picker::from_fontsize(font);
            picker.set_protocol_type(ProtocolType::Kitty);
            picker
        }
        _ => Picker::halfblocks(),
    }
}

/// One encoded image per theme and size, built on first use.
pub struct Logo {
    picker: Picker,
    cache: Vec<(ThemeMode, Rect, Protocol)>,
}

impl Logo {
    /// Chooses how to draw, without asking the terminal anything.
    ///
    /// `Picker::from_query_stdio` writes a query and reads the answer on stdin
    /// from a thread. A terminal that never answers (tmux, VS Code's console…)
    /// leaves that thread blocked on stdin after the timeout: it then steals
    /// keystrokes and, when it finally returns, turns raw mode off under our
    /// feet — the TUI hung on a blank screen. So the protocol comes from the
    /// environment and the cell size from the window-size ioctl: nothing reads
    /// stdin but the event loop.
    pub fn detect() -> Self {
        let picker = picker_from_env(&|k| std::env::var(k).unwrap_or_default());
        log::info!("logo drawn with {:?}", picker.protocol_type());
        Logo::with_picker(picker)
    }

    pub fn with_picker(picker: Picker) -> Self {
        Logo {
            picker,
            cache: Vec::new(),
        }
    }

    fn source(mode: ThemeMode) -> Option<DynamicImage> {
        let bytes = match mode {
            ThemeMode::Dark => DARK_PNG,
            ThemeMode::Light => LIGHT_PNG,
        };
        image::load_from_memory(bytes).ok()
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let mode = theme::mode();
        if !self.cache.iter().any(|(m, a, _)| *m == mode && *a == area) {
            let Some(img) = Self::source(mode) else {
                return;
            };
            // Half-blocks paint the transparent pixels: give them the ground.
            let bg = match theme::color_background() {
                ratatui::style::Color::Rgb(r, g, b) => [r, g, b, 255],
                _ => [0, 0, 0, 255],
            };
            self.picker.set_background_color(Some(bg));
            match self.picker.new_protocol(
                img,
                Size::new(area.width, area.height),
                Resize::Fit(Some(FilterType::Lanczos3)),
            ) {
                Ok(p) => {
                    self.cache.retain(|(m, a, _)| !(*m == mode && *a != area));
                    self.cache.push((mode, area, p));
                }
                Err(e) => {
                    log::warn!("cannot encode the logo: {e}");
                    return;
                }
            }
        }
        if let Some((_, _, p)) = self.cache.iter().find(|(m, a, _)| *m == mode && *a == area) {
            Image::new(p).render(area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_logos_decode() {
        assert!(Logo::source(ThemeMode::Dark).is_some());
        assert!(Logo::source(ThemeMode::Light).is_some());
    }

    #[test]
    fn tmux_and_unknown_terminals_get_half_blocks() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            }
        };
        let tmux = env(&[("KITTY_WINDOW_ID", "1"), ("TMUX", "/tmp/tmux-1/default")]);
        assert_eq!(
            picker_from_env(&tmux).protocol_type(),
            ProtocolType::Halfblocks
        );
        let unknown = env(&[("TERM", "xterm-256color")]);
        assert_eq!(
            picker_from_env(&unknown).protocol_type(),
            ProtocolType::Halfblocks
        );
    }

    #[test]
    fn half_blocks_paint_the_logo_area() {
        let mut logo = Logo::with_picker(Picker::halfblocks());
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);
        logo.render(area, &mut buf);
        let painted = (0..4)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|(x, y)| buf[(*x, *y)].symbol() != " ")
            .count();
        assert!(painted > 8, "only {painted} cells painted");
    }
}
