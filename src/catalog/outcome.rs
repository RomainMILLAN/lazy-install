//! Value objects describing what a script said, or how a run ended.

/// Text that came from a script, made safe to put on screen.
///
/// Everything a script prints is untrusted for display: a control character can
/// move the cursor, and a bidi override (U+202E, category Cf) can make a version
/// read backwards. Both are dropped here, once, so no panel has to remember to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayText(String);

impl DisplayText {
    pub const VERSION_MAX: usize = 32;
    pub const MESSAGE_MAX: usize = 200;

    pub fn new(raw: &str, max_chars: usize) -> Self {
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_control() && !is_format_char(*c))
            .collect();
        let trimmed = cleaned.trim();
        DisplayText(trimmed.chars().take(max_chars).collect())
    }

    pub fn message(raw: &str) -> Self {
        Self::new(raw, Self::MESSAGE_MAX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for DisplayText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unicode general category Cf (format characters). The standard library has no
/// category lookup, so the ranges are listed; they are the ones that matter for
/// a terminal: bidi controls, zero-width characters, the BOM, tag characters.
fn is_format_char(c: char) -> bool {
    matches!(c as u32,
        0x00AD
        | 0x0600..=0x0605
        | 0x061C
        | 0x06DD
        | 0x070F
        | 0x180E
        | 0x200B..=0x200F
        | 0x202A..=0x202E
        | 0x2060..=0x2064
        | 0x2066..=0x206F
        | 0xFEFF
        | 0xFFF9..=0xFFFB
        | 0x110BD
        | 0x1D173..=0x1D17A
        | 0xE0001
        | 0xE0020..=0xE007F)
}

/// The installed and latest versions, as reported through the helpers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Versions {
    installed: Option<DisplayText>,
    latest: Option<DisplayText>,
}

impl Versions {
    pub fn new(installed: &str, latest: &str) -> Self {
        let field = |s: &str| {
            let t = DisplayText::new(s, DisplayText::VERSION_MAX);
            (!t.is_empty()).then_some(t)
        };
        Versions {
            installed: field(installed),
            latest: field(latest),
        }
    }

    /// Read-only projections, for formats that need the two values apart (the
    /// `--check --json` report). Display uses [`Versions::label`].
    pub fn installed(&self) -> Option<&str> {
        self.installed.as_ref().map(DisplayText::as_str)
    }

    pub fn latest(&self) -> Option<&str> {
        self.latest.as_ref().map(DisplayText::as_str)
    }

    /// `0.44.1 → 0.45.0`, `0.45.0`, or nothing at all.
    pub fn label(&self) -> String {
        match (&self.installed, &self.latest) {
            (Some(i), Some(l)) if i == l => i.to_string(),
            (Some(i), Some(l)) => format!("{i} → {l}"),
            (Some(i), None) => i.to_string(),
            (None, Some(l)) => format!("→ {l}"),
            (None, None) => String::new(),
        }
    }
}

/// What a `needs_update` run established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    UpdateAvailable(Versions),
    UpToDate(Versions),
    /// The script itself is unusable: refused by the trust check, the static
    /// contract check, or missing the function at run time.
    Invalid(DisplayText),
    /// The check ran and failed: no token, a crash, a timeout.
    Errored(DisplayText),
}

/// How a process that did start ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    Code(i32),
    Signal(i32),
}

impl ExitOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, ExitOutcome::Code(0))
    }

    pub fn label(&self) -> String {
        match self {
            ExitOutcome::Code(c) => format!("exit {c}"),
            ExitOutcome::Signal(s) => format!("signal {s}"),
        }
    }
}

/// Why an update never started (refused before any process existed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotStartedReason(pub DisplayText);

impl NotStartedReason {
    pub fn new(msg: &str) -> Self {
        NotStartedReason(DisplayText::message(msg))
    }
}

/// The one thing the list shows per application. A projection of the runtime
/// state, never stored: see `AppRuntime::tag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Updating,
    Queued,
    Invalid,
    Failed,
    Checking,
    Error,
    Update,
    Ok,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_text_drops_control_and_bidi_characters() {
        let t = DisplayText::message("1.2\u{202E}3\x1b[31m\n");
        assert_eq!(t.as_str(), "1.23[31m");
    }

    #[test]
    fn display_text_is_bounded() {
        let t = DisplayText::new(&"x".repeat(100), DisplayText::VERSION_MAX);
        assert_eq!(t.as_str().chars().count(), 32);
    }

    #[test]
    fn versions_label() {
        assert_eq!(Versions::new("1", "2").label(), "1 → 2");
        assert_eq!(Versions::new("1", "1").label(), "1");
        assert_eq!(Versions::new("", "").label(), "");
        assert_eq!(Versions::new("1:9.0", "1:9.1").label(), "1:9.0 → 1:9.1");
    }
}
