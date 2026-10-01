//! What `--check` found, and how it is printed. A read-only projection of the
//! check axis of each application; pure, tested without bash.

use serde::Serialize;

use crate::catalog::{CheckState, Versions};

/// Why an application is listed under `errors`. The same words in the text
/// and in the JSON `kind`: one vocabulary, not two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// The check ran and failed (crash, timeout, no result token).
    Error,
    /// The script itself is refused (trust rule, contract).
    Invalid,
    /// No result came back before the deadline, or it was lost.
    NoAnswer,
}

impl ErrorKind {
    fn label(self) -> &'static str {
        match self {
            ErrorKind::Error => "ERROR",
            ErrorKind::Invalid => "INVALID",
            ErrorKind::NoAnswer => "NO-ANSWER",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateEntry {
    name: String,
    versions: Versions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorEntry {
    name: String,
    kind: ErrorKind,
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckReport {
    updates: Vec<UpdateEntry>,
    errors: Vec<ErrorEntry>,
    up_to_date: usize,
}

impl CheckReport {
    /// Projects `(name, check state)` pairs, in the order given — the caller
    /// passes them in catalog order, never in the order results arrived.
    pub fn project<'a>(apps: impl IntoIterator<Item = (&'a str, &'a CheckState)>) -> Self {
        let mut report = CheckReport::default();
        for (name, state) in apps {
            let name = name.to_string();
            // Exhaustive, no `_`: a new state must be placed here on purpose.
            match state {
                CheckState::UpdateAvailable(versions) => report.updates.push(UpdateEntry {
                    name,
                    versions: versions.clone(),
                }),
                CheckState::UpToDate(_) => report.up_to_date += 1,
                CheckState::Errored(m) => report.errors.push(ErrorEntry {
                    name,
                    kind: ErrorKind::Error,
                    message: Some(m.to_string()),
                }),
                CheckState::Invalid(m) => report.errors.push(ErrorEntry {
                    name,
                    kind: ErrorKind::Invalid,
                    message: Some(m.to_string()),
                }),
                CheckState::Checking { .. } | CheckState::Unknown => {
                    report.errors.push(ErrorEntry {
                        name,
                        kind: ErrorKind::NoAnswer,
                        message: None,
                    })
                }
            }
        }
        report
    }

    pub fn has_updates(&self) -> bool {
        !self.updates.is_empty()
    }

    /// One line per application that needs attention, then a summary.
    pub fn to_text(&self) -> String {
        let width = self
            .updates
            .iter()
            .map(|u| u.name.chars().count())
            .chain(self.errors.iter().map(|e| e.name.chars().count()))
            .max()
            .unwrap_or(0);
        let mut out = String::new();
        let mut line = |kind: &str, name: &str, rest: &str| {
            let row = format!("{kind:<10} {name:<width$}  {rest}");
            out.push_str(row.trim_end());
            out.push('\n');
        };
        for u in &self.updates {
            line("UPDATE", &u.name, &u.versions.label());
        }
        for e in &self.errors {
            line(e.kind.label(), &e.name, e.message.as_deref().unwrap_or(""));
        }
        out.push_str(&self.summary());
        out.push('\n');
        out
    }

    fn summary(&self) -> String {
        let count = |kind: ErrorKind| self.errors.iter().filter(|e| e.kind == kind).count();
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = vec![plural(self.updates.len(), "update", "updates")];
        for (kind, one, many) in [
            (ErrorKind::Error, "error", "errors"),
            (ErrorKind::Invalid, "invalid", "invalid"),
            (ErrorKind::NoAnswer, "no answer", "no answer"),
        ] {
            let n = count(kind);
            if n > 0 {
                parts.push(plural(n, one, many));
            }
        }
        parts.push(format!("{} up to date", self.up_to_date));
        parts.join(", ")
    }

    /// The published contract read by `notify.sh`. Built from private DTOs so
    /// the domain never learns the exchange format.
    pub fn to_json(&self) -> String {
        let dto = ReportDto {
            updates: self
                .updates
                .iter()
                .map(|u| UpdateDto {
                    name: &u.name,
                    installed: u.versions.installed(),
                    latest: u.versions.latest(),
                })
                .collect(),
            errors: self
                .errors
                .iter()
                .map(|e| ErrorDto {
                    name: &e.name,
                    kind: e.kind,
                    message: e.message.as_deref(),
                })
                .collect(),
            up_to_date: self.up_to_date,
        };
        serde_json::to_string_pretty(&dto).unwrap_or_else(|_| "{}".to_string())
    }
}

#[derive(Serialize)]
struct ReportDto<'a> {
    updates: Vec<UpdateDto<'a>>,
    errors: Vec<ErrorDto<'a>>,
    up_to_date: usize,
}

#[derive(Serialize)]
struct UpdateDto<'a> {
    name: &'a str,
    installed: Option<&'a str>,
    latest: Option<&'a str>,
}

#[derive(Serialize)]
struct ErrorDto<'a> {
    name: &'a str,
    kind: ErrorKind,
    message: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::DisplayText;

    fn all_states() -> Vec<(&'static str, CheckState)> {
        vec![
            (
                "vscode",
                CheckState::UpdateAvailable(Versions::new("1.139.1", "1.140.0")),
            ),
            (
                "kitty",
                CheckState::UpToDate(Versions::new("0.49.2", "0.49.2")),
            ),
            (
                "appsec",
                CheckState::Errored(DisplayText::message("rate-limited")),
            ),
            (
                "old",
                CheckState::Invalid(DisplayText::message("writable by others")),
            ),
            ("rdm", CheckState::Checking { generation: 1 }),
            ("never", CheckState::Unknown),
        ]
    }

    fn report() -> CheckReport {
        let states = all_states();
        CheckReport::project(states.iter().map(|(n, s)| (*n, s)))
    }

    #[test]
    fn every_state_lands_in_its_place() {
        let r = report();
        assert_eq!(r.updates.len(), 1);
        assert_eq!(r.up_to_date, 1);
        let kinds: Vec<(&str, ErrorKind)> =
            r.errors.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("appsec", ErrorKind::Error),
                ("old", ErrorKind::Invalid),
                ("rdm", ErrorKind::NoAnswer),
                ("never", ErrorKind::NoAnswer),
            ]
        );
    }

    #[test]
    fn errors_alone_are_not_updates() {
        let errored = CheckState::Errored(DisplayText::message("x"));
        assert!(!CheckReport::project([("a", &errored)]).has_updates());
        assert!(report().has_updates());
    }

    #[test]
    fn json_carries_exactly_the_published_fields() {
        let v: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
        let mut top: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        top.sort();
        assert_eq!(top, vec!["errors", "up_to_date", "updates"]);
        assert_eq!(
            v["updates"][0],
            serde_json::json!({"name": "vscode", "installed": "1.139.1", "latest": "1.140.0"})
        );
        assert_eq!(
            v["errors"][0],
            serde_json::json!({"name": "appsec", "kind": "error", "message": "rate-limited"})
        );
        assert_eq!(v["errors"][2]["kind"], "no-answer");
        assert_eq!(v["errors"][2]["message"], serde_json::Value::Null);
        assert_eq!(v["up_to_date"], 1);
    }

    #[test]
    fn text_uses_the_json_vocabulary() {
        let t = report().to_text();
        assert!(t.contains("UPDATE     vscode  1.139.1 → 1.140.0"), "{t}");
        assert!(t.contains("ERROR      appsec  rate-limited"), "{t}");
        assert!(t.contains("INVALID    old     writable by others"), "{t}");
        assert!(t.contains("NO-ANSWER  rdm\n"), "{t}");
        assert!(
            t.ends_with("1 update, 1 error, 1 invalid, 2 no answer, 1 up to date\n"),
            "{t}"
        );
    }

    #[test]
    fn label_edge_cases_are_pinned() {
        // Installed version unknown, and an update reported with equal versions.
        assert_eq!(Versions::new("", "1.2").label(), "→ 1.2");
        assert_eq!(Versions::new("1.2", "1.2").label(), "1.2");
    }

    #[test]
    fn nothing_to_report() {
        let ok = CheckState::UpToDate(Versions::default());
        let t = CheckReport::project([("a", &ok), ("b", &ok)]).to_text();
        assert_eq!(t, "0 updates, 2 up to date\n");
    }
}
