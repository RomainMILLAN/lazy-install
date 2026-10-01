//! In-TUI file picker for a script path (from lazy-aws). It knows nothing about
//! scripts: it navigates directories and yields a path; validation is elsewhere.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};

use crate::ui::style::{styles, theme};
use crate::ui::text::{fuzzy_match, truncate_chars};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Parent,
    Dir,
    File,
}

struct Entry {
    label: String,
    path: PathBuf,
    kind: Kind,
}

pub enum BrowserOutcome {
    None,
    PickFile(PathBuf),
    Cancelled,
}

pub struct FileBrowser {
    visible: bool,
    cwd: PathBuf,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    filter: String,
    cursor: usize,
}

impl Default for FileBrowser {
    fn default() -> Self {
        Self::new()
    }
}

impl FileBrowser {
    pub fn new() -> Self {
        FileBrowser {
            visible: false,
            cwd: PathBuf::from("."),
            entries: Vec::new(),
            filtered: Vec::new(),
            filter: String::new(),
            cursor: 0,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn open(&mut self, start: &Path) {
        self.cwd = start.to_path_buf();
        self.visible = true;
        self.refresh();
    }

    fn refresh(&mut self) {
        self.entries = build_entries(&self.cwd);
        self.filter.clear();
        self.cursor = 0;
        self.rebuild_filter();
    }

    fn rebuild_filter(&mut self) {
        let mut scored: Vec<(usize, i32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| fuzzy_match(&e.label, &self.filter).map(|s| (i, s)))
            .collect();
        if !self.filter.is_empty() {
            scored.sort_by_key(|b| std::cmp::Reverse(b.1));
        }
        self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        self.cursor = self.cursor.min(self.filtered.len().saturating_sub(1));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> BrowserOutcome {
        match key.code {
            KeyCode::Esc => {
                self.visible = false;
                BrowserOutcome::Cancelled
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                BrowserOutcome::None
            }
            KeyCode::Down => {
                if self.cursor + 1 < self.filtered.len() {
                    self.cursor += 1;
                }
                BrowserOutcome::None
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Backspace => {
                if self.filter.pop().is_none() {
                    if let Some(parent) = self.cwd.parent().map(Path::to_path_buf) {
                        self.cwd = parent;
                        self.refresh();
                    }
                } else {
                    self.rebuild_filter();
                }
                BrowserOutcome::None
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                self.cursor = 0;
                self.rebuild_filter();
                BrowserOutcome::None
            }
            _ => BrowserOutcome::None,
        }
    }

    fn activate(&mut self) -> BrowserOutcome {
        let Some(entry) = self
            .filtered
            .get(self.cursor)
            .and_then(|&i| self.entries.get(i))
        else {
            return BrowserOutcome::None;
        };
        match entry.kind {
            Kind::Parent | Kind::Dir => {
                self.cwd = entry.path.clone();
                self.refresh();
                BrowserOutcome::None
            }
            Kind::File => {
                let p = entry.path.clone();
                self.visible = false;
                BrowserOutcome::PickFile(p)
            }
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if !self.visible {
            return;
        }
        Clear.render(area, buf);
        let title = format!(" Choose a script — {} ", self.cwd.display());
        let block = Block::default()
            .title(truncate_chars(
                &title,
                area.width.saturating_sub(2) as usize,
            ))
            .title_style(styles::block_title_style(true))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(styles::border_style(true));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height < 3 {
            return;
        }
        let list_h = inner.height.saturating_sub(2) as usize;
        let offset = self.cursor.saturating_sub(list_h.saturating_sub(1));
        for (vis, &ei) in self.filtered.iter().skip(offset).take(list_h).enumerate() {
            let entry = &self.entries[ei];
            let y = inner.y + vis as u16;
            let icon = match entry.kind {
                Kind::Parent => "↑ ",
                Kind::Dir => "▸ ",
                Kind::File => "  ",
            };
            let style = if offset + vis == self.cursor {
                styles::selected_style(true)
            } else if entry.kind == Kind::File {
                Style::default().fg(theme::color_text())
            } else {
                styles::directory_style()
            };
            let line = truncate_chars(
                &format!("{icon}{}", entry.label),
                inner.width.saturating_sub(2) as usize,
            );
            buf.set_string(inner.x + 1, y, line, style);
        }
        let footer = if self.filter.is_empty() {
            " ↑↓ move · enter open/pick · type to filter · esc cancel ".to_string()
        } else {
            format!(" filter: {} ", self.filter)
        };
        buf.set_string(
            inner.x + 1,
            inner.y + inner.height - 1,
            truncate_chars(&footer, inner.width.saturating_sub(2) as usize),
            styles::muted_style(),
        );
    }
}

fn build_entries(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    if let Some(parent) = dir.parent() {
        out.push(Entry {
            label: "..".to_string(),
            path: parent.to_path_buf(),
            kind: Kind::Parent,
        });
    }
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                dirs.push(Entry {
                    label: name,
                    path,
                    kind: Kind::Dir,
                });
            } else {
                files.push(Entry {
                    label: name,
                    path,
                    kind: Kind::File,
                });
            }
        }
    }
    dirs.sort_by_key(|e| e.label.to_lowercase());
    // Scripts first, then the rest.
    files.sort_by_key(|e| (!e.label.ends_with(".sh"), e.label.to_lowercase()));
    out.extend(dirs);
    out.extend(files);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn navigates_and_picks() {
        let tmp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub").join("k.sh"), "").unwrap();
        let mut fb = FileBrowser::new();
        fb.open(tmp.path());
        for c in "sub".chars() {
            fb.handle_key(k(KeyCode::Char(c)));
        }
        assert!(matches!(
            fb.handle_key(k(KeyCode::Enter)),
            BrowserOutcome::None
        ));
        fb.handle_key(k(KeyCode::Char('k')));
        match fb.handle_key(k(KeyCode::Enter)) {
            BrowserOutcome::PickFile(p) => assert!(p.ends_with("sub/k.sh")),
            _ => panic!("no pick"),
        }
    }
}
