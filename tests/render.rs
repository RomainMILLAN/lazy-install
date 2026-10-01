//! The panels, drawn into a buffer with the real layout.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use lazy_install::catalog::Tag;
use lazy_install::ui::layout::compute_layout;
use lazy_install::ui::panels::apps::{AppsPanel, Row};
use lazy_install::ui::panels::terminal::TerminalPanel;
use lazy_install::ui::style::theme;

fn text(buf: &Buffer, area: Rect) -> String {
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

fn rows() -> Vec<Row> {
    [
        Tag::Updating,
        Tag::Queued,
        Tag::Invalid,
        Tag::Failed,
        Tag::Checking,
        Tag::Error,
        Tag::Update,
        Tag::Ok,
        Tag::Unknown,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, tag)| Row {
        name: format!("app{i}"),
        tag,
        versions: if tag == Tag::Update {
            "1.0 → 1.1".into()
        } else {
            String::new()
        },
        detail: (tag == Tag::Error).then(|| "curl failed".to_string()),
    })
    .collect()
}

#[test]
fn every_tag_is_drawn_with_its_label() {
    for (w, h) in [(120, 30), (80, 40)] {
        let layout = compute_layout(w, h, false);
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let rows = rows();
        AppsPanel {
            rows: &rows,
            selected: 0,
            focused: true,
            filter: "",
            total: rows.len(),
            spinner: '*',
        }
        .render(layout.list, &mut buf);
        let screen = text(&buf, layout.list);
        for row in &rows {
            let label = theme::tag_label(row.tag);
            assert!(screen.contains(label), "{w}x{h}: {label} missing\n{screen}");
        }
        assert!(screen.contains("1.0 → 1.1"), "{screen}");
        assert!(screen.contains("curl failed"), "{screen}");
    }
}

#[test]
fn empty_list_says_how_to_start() {
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    AppsPanel {
        rows: &[],
        selected: 0,
        focused: true,
        filter: "",
        total: 0,
        spinner: '*',
    }
    .render(area, &mut buf);
    assert!(text(&buf, area).contains("Press a to add one"));
}

#[test]
fn terminal_without_run_shows_the_last_check() {
    let area = Rect::new(0, 0, 70, 10);
    let mut buf = Buffer::empty(area);
    TerminalPanel {
        app_name: Some("kitty"),
        record: None,
        running: false,
        focused: false,
        detail: Some("timeout"),
    }
    .render(area, &mut buf);
    let t = text(&buf, area);
    assert!(
        t.contains("Logs · kitty") && t.contains("Last check: timeout"),
        "{t}"
    );
}
