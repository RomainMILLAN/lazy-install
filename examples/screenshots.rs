//! Renders the real screens into a ratatui buffer and writes each one out as an
//! HTML page, one `<span>` per run of same-styled cells.
//!
//! Not a mockup: the colours and glyphs come from the same widgets and the same
//! `theme::color_*` functions the application draws with.
//!
//! ```text
//! cargo run --example screenshots
//! bash docs/assets/src/render.sh
//! ```

use std::fmt::Write as _;
use std::fs;

use lazy_install::catalog::{ExitOutcome, Tag};
use lazy_install::jobs::RunRecord;
use lazy_install::pty::Screen;
use lazy_install::ui::brand;
use lazy_install::ui::components::statusbar::{list_hints, terminal_hints};
use lazy_install::ui::components::StatusBar;
use lazy_install::ui::keys::default_key_map;
use lazy_install::ui::layout::compute_layout;
use lazy_install::ui::panels::apps::{AppsPanel, Row};
use lazy_install::ui::panels::terminal::TerminalPanel;
use lazy_install::ui::style::theme::{self, ThemeMode};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

const FONT_PX: f32 = 15.0;
const CELL_W: f32 = 9.0;
const CELL_H: f32 = 18.4;

fn row(name: &str, tag: Tag, versions: &str, detail: Option<&str>) -> Row {
    Row {
        name: name.to_string(),
        tag,
        versions: versions.to_string(),
        detail: detail.map(str::to_string),
    }
}

fn rows(updating: bool) -> Vec<Row> {
    vec![
        row("kitty", Tag::Ok, "0.49.2", None),
        row("firefox", Tag::Ok, "157.0", None),
        row("vscode", Tag::Update, "1.139.1 → 1.140.0", None),
        row(
            "dbeaver",
            if updating { Tag::Updating } else { Tag::Update },
            "26.2.0 → 26.2.1",
            None,
        ),
        row("rdm", Tag::Ok, "2026.3.0.5", None),
        row(
            "lazygit",
            if updating { Tag::Queued } else { Tag::Update },
            "0.64.1 → 0.65.1",
            None,
        ),
        row("delta", Tag::Ok, "0.19.2", None),
        row("lazy-aws", Tag::Ok, "0.3.0", None),
        row("lazy-transfer", Tag::Ok, "0.3.0", None),
        row("temporary-tab", Tag::Ok, "1.0.1", None),
        row(
            "appsec-references",
            Tag::Error,
            "",
            Some("GitHub API rate-limited"),
        ),
    ]
}

/// A run, fed through the same vt100 parser a real one goes through.
fn run(rows: u16, cols: u16, text: &str, outcome: Option<ExitOutcome>) -> RunRecord {
    let screen = Screen::new(rows, cols);
    screen.process(text.replace('\n', "\r\n").as_bytes());
    RunRecord { screen, outcome }
}

const DBEAVER_RUN: &str = "Résolution de la dernière version de DBeaver CE...\n\
Version installée : 26.2.0 — dernière version : 26.2.1\n\
Téléchargement de dbeaver-ce 26.2.1 (121 Mo)...\n\
\x1b[32m✓\x1b[0m sha256 vérifié\n\
Installation (sudo)...\n\
[sudo] password for romain: ";

const LAZYGIT_RUN: &str = "Fetching latest lazygit version...\n\
Downloading lazygit 0.65.1...\n\
lazygit installed to /home/romain/.local/bin/lazygit\n\
commit=8e1b3f2, build date=2026-09-28T10:12:44Z, version=0.65.1\n\
\n\
\x1b[90m[lazy-install] exit 0\x1b[0m";

fn screen(width: u16, height: u16, updating: bool) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, Style::default().bg(theme::color_background()));
    let km = default_key_map();
    let l = compute_layout(width, height, false);
    let rows = rows(updating);
    let selected = if updating { 3 } else { 5 };

    if let Some(header) = l.header {
        let count = |pred: &dyn Fn(Tag) -> bool| rows.iter().filter(|r| pred(r.tag)).count();
        brand::render_header(
            header,
            &mut buf,
            &brand::Summary {
                total: rows.len(),
                updates: count(&|t| t == Tag::Update),
                problems: count(&|t| matches!(t, Tag::Error | Tag::Failed | Tag::Invalid)),
                busy: count(&|t| t == Tag::Updating),
            },
        );
    }

    AppsPanel {
        rows: &rows,
        selected,
        focused: !updating,
        filter: "",
        filtering: false,
        total: rows.len(),
        spinner: '⠹',
    }
    .render(l.list, &mut buf);

    let (pty_rows, pty_cols) = l.pty_size();
    let (record, name) = if updating {
        (run(pty_rows, pty_cols, DBEAVER_RUN, None), "dbeaver")
    } else {
        (
            run(pty_rows, pty_cols, LAZYGIT_RUN, Some(ExitOutcome::Code(0))),
            "lazygit",
        )
    };
    TerminalPanel {
        app_name: Some(name),
        record: Some(&record),
        running: updating,
        focused: updating,
        detail: None,
    }
    .render(l.terminal, &mut buf);

    let hints = if updating {
        terminal_hints(&km, true)
    } else {
        list_hints(&km)
    };
    StatusBar::render(l.status, &mut buf, &hints);
    buf
}

fn hex(c: Color, fallback: &str) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02X}{g:02X}{b:02X}"),
        _ => fallback.to_string(),
    }
}

/// One `<span>` per run of cells sharing a style, so the output stays small.
/// `logo`: the cells where the TUI draws the real logo with the terminal's image
/// protocol. A buffer cannot carry that image, so the HTML places the same file
/// at the same cells.
fn to_html(buf: &Buffer, title: &str, logo: Option<(Rect, &str)>) -> String {
    let area = *buf.area();
    let page_bg = hex(theme::color_background(), "#000000");
    let page_fg = hex(theme::color_text(), "#FFFFFF");

    let mut body = String::new();
    for y in 0..area.height {
        let mut runs: Vec<(String, String)> = Vec::new();
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let (fg, bg) = if reversed {
                (hex(cell.bg, &page_bg), hex(cell.fg, &page_fg))
            } else {
                (hex(cell.fg, &page_fg), hex(cell.bg, &page_bg))
            };
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "700"
            } else {
                "400"
            };
            let key = format!("color:{fg};background:{bg};font-weight:{weight}");
            let symbol = match cell.symbol() {
                "" => " ".to_string(),
                s => s
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            };
            match runs.last_mut() {
                Some((k, text)) if *k == key => text.push_str(&symbol),
                _ => runs.push((key, symbol)),
            }
        }
        let _ = write!(body, "<div class=\"row\">");
        for (style, text) in runs {
            let _ = write!(body, "<span style=\"{style}\">{text}</span>");
        }
        let _ = writeln!(body, "</div>");
    }

    let logo_html = logo
        .map(|(r, file)| {
            format!(
                "<img class=\"logo\" src=\"../{file}\" style=\"left:{}px;top:{}px;width:{}px;height:{}px\">",
                r.x as f32 * CELL_W,
                r.y as f32 * CELL_H,
                r.width as f32 * CELL_W,
                r.height as f32 * CELL_H,
            )
        })
        .unwrap_or_default();
    let w = (area.width as f32 * CELL_W).round() as u32;
    let h = (area.height as f32 * CELL_H).round() as u32;

    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<title>{title}</title>
<!-- Generated by `cargo run --example screenshots`. Do not edit by hand. -->
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;700&display=swap">
<style>
  *, *::before, *::after {{ box-sizing: border-box; }}
  html, body {{ margin: 0; padding: 0; }}
  body {{
    width: {w}px; height: {h}px; overflow: hidden;
    background: {page_bg}; color: {page_fg};
    font-family: "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: {FONT_PX}px; line-height: {CELL_H}px;
    -webkit-font-smoothing: antialiased;
  }}
  body {{ position: relative; }}
  .row {{ white-space: pre; height: {CELL_H}px; }}
  .logo {{ position: absolute; object-fit: contain; }}
</style>
{body}{logo_html}"#
    )
}

fn main() -> std::io::Result<()> {
    fs::create_dir_all("docs/assets/src")?;
    for (mode, suffix) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        theme::set_mode(mode);
        let logo_file = match mode {
            ThemeMode::Dark => "logo-mark.svg",
            ThemeMode::Light => "logo-mark-light.svg",
        };
        for (kind, updating) in [("list", false), ("update", true)] {
            let name = format!("screenshot-{kind}-{suffix}");
            fs::write(
                format!("docs/assets/src/{name}.html"),
                to_html(
                    &screen(120, 24, updating),
                    &name,
                    compute_layout(120, 24, false)
                        .header
                        .map(|h| (brand::logo_area(h), logo_file)),
                ),
            )?;
            println!("wrote docs/assets/src/{name}.html");
        }
    }
    Ok(())
}
