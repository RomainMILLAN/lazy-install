//! Keys typed in the terminal panel, as the bytes a real terminal would send.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn key_to_bytes(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let arrow = |c: char| {
        if application_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let mut out = match key.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            c @ 'a'..='z' => vec![(c as u8) & 0x1f],
            '@' | ' ' => vec![0],
            '[' => vec![0x1b],
            '\\' => vec![0x1c],
            ']' => vec![0x1d],
            '^' => vec![0x1e],
            '_' => vec![0x1f],
            _ => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => arrow('A'),
        KeyCode::Down => arrow('B'),
        KeyCode::Right => arrow('C'),
        KeyCode::Left => arrow('D'),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => vec![],
        },
        _ => vec![],
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> Vec<u8> {
        key_to_bytes(KeyEvent::new(code, m), false)
    }

    #[test]
    fn encodes_the_usual_keys() {
        assert_eq!(k(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(k(KeyCode::Char('é'), KeyModifiers::NONE), "é".as_bytes());
        assert_eq!(k(KeyCode::Char('c'), KeyModifiers::CONTROL), vec![3]);
        assert_eq!(k(KeyCode::Char('D'), KeyModifiers::CONTROL), vec![4]);
        assert_eq!(k(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(k(KeyCode::Backspace, KeyModifiers::NONE), vec![0x7f]);
        assert_eq!(k(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(k(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
    }

    #[test]
    fn application_cursor_mode() {
        let up = key_to_bytes(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true);
        assert_eq!(up, b"\x1bOA");
    }
}
