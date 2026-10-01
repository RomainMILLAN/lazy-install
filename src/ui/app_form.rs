//! The add/edit form as a state machine with no terminal: name, then script,
//! then (if the file does not exist) the template offer.

use std::path::{Path, PathBuf};

use crate::catalog::{AppId, AppName, Application, ScriptRef};
use crate::config::paths::resolve_script;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormStep {
    AskName {
        initial: String,
        error: Option<String>,
    },
    AskScript {
        initial: String,
        error: Option<String>,
    },
    OfferTemplate {
        path: PathBuf,
    },
    Ready {
        name: AppName,
        script: ScriptRef,
    },
}

#[derive(Debug, Clone)]
pub struct AppForm {
    editing: Option<AppId>,
    name: Option<AppName>,
    script_initial: String,
    pending_script: Option<ScriptRef>,
}

impl AppForm {
    pub fn add() -> (Self, FormStep) {
        (
            AppForm {
                editing: None,
                name: None,
                script_initial: String::new(),
                pending_script: None,
            },
            FormStep::AskName {
                initial: String::new(),
                error: None,
            },
        )
    }

    pub fn edit(app: &Application) -> (Self, FormStep) {
        (
            AppForm {
                editing: Some(app.id()),
                name: None,
                script_initial: app.script().raw().to_string(),
                pending_script: None,
            },
            FormStep::AskName {
                initial: app.name().as_str().to_string(),
                error: None,
            },
        )
    }

    pub fn editing(&self) -> Option<AppId> {
        self.editing
    }

    pub fn name_entered(&mut self, raw: &str) -> FormStep {
        match AppName::parse(raw) {
            Ok(name) => {
                self.name = Some(name);
                FormStep::AskScript {
                    initial: self.script_initial.clone(),
                    error: None,
                }
            }
            Err(e) => FormStep::AskName {
                initial: raw.to_string(),
                error: Some(e.to_string()),
            },
        }
    }

    pub fn script_entered(&mut self, raw: &str, scripts_dir: &Path) -> FormStep {
        if raw.trim().is_empty() {
            return FormStep::AskScript {
                initial: String::new(),
                error: Some("the script path is empty".into()),
            };
        }
        let script = resolve_script(raw, scripts_dir);
        self.script_initial = raw.trim().to_string();
        if !script.path().exists() {
            let path = script.path().to_path_buf();
            self.pending_script = Some(script);
            return FormStep::OfferTemplate { path };
        }
        self.ready(script)
    }

    /// The template was created (or the user declined and we go back).
    pub fn template_answered(&mut self, created: bool) -> FormStep {
        match (created, self.pending_script.take()) {
            (true, Some(script)) => self.ready(script),
            _ => FormStep::AskScript {
                initial: self.script_initial.clone(),
                error: None,
            },
        }
    }

    /// Validation refused the script: ask again, with the reason.
    pub fn script_refused(&self, error: String) -> FormStep {
        FormStep::AskScript {
            initial: self.script_initial.clone(),
            error: Some(error),
        }
    }

    fn ready(&mut self, script: ScriptRef) -> FormStep {
        match self.name.clone() {
            Some(name) => FormStep::Ready { name, script },
            None => FormStep::AskName {
                initial: String::new(),
                error: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_name_then_script() {
        let (mut f, step) = AppForm::add();
        assert!(matches!(step, FormStep::AskName { .. }));
        assert!(matches!(
            f.name_entered("  "),
            FormStep::AskName { error: Some(_), .. }
        ));
        assert!(matches!(
            f.name_entered("kitty"),
            FormStep::AskScript { .. }
        ));
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        match f.script_entered("Cargo.toml", dir) {
            FormStep::Ready { name, script } => {
                assert_eq!(name.as_str(), "kitty");
                assert_eq!(script.raw(), "Cargo.toml");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_file_offers_the_template() {
        let (mut f, _) = AppForm::add();
        f.name_entered("x");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(matches!(
            f.script_entered("nope.sh", dir),
            FormStep::OfferTemplate { .. }
        ));
        assert!(matches!(
            f.template_answered(false),
            FormStep::AskScript { .. }
        ));
        f.script_entered("nope.sh", dir);
        assert!(matches!(f.template_answered(true), FormStep::Ready { .. }));
    }
}
