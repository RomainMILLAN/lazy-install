//! The loop: routes keys to intentions, runs the session's effects, draws.
//!
//! It decides nothing about the domain. `Session` says what should happen,
//! `Tui` makes it happen and reports back what did.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::{Frame, Terminal};

use crate::catalog::{AppId, Application};
use crate::config::paths::{is_inside, spell};
use crate::config::ConfigStore;
use crate::jobs::{CheckScheduler, UpdateRunner};
use crate::pty::screen::looks_like_password_prompt;
use crate::pty::{key_to_bytes, PtySpawner};
use crate::script::contract::TEMPLATE;
use crate::script::{validate, BashCheckRunner};
use crate::session::{Effect, Fact, Session};

use super::app_form::{AppForm, FormStep};
use super::components::statusbar::{list_hints, terminal_hints};
use super::components::{
    BrowserOutcome, Choice, ChoiceDialog, ConfirmDialog, FileBrowser, HelpPopup, InputBox,
    StatusBar,
};
use super::keys::{default_key_map, KeyMap};
use super::layout::{centered, compute_layout, Layout};
use super::messages::{Action, BgMsg};
use super::panels::apps::{AppsPanel, Row};
use super::panels::terminal::TerminalPanel;
use super::style::{styles, theme};
use super::text::{fuzzy_match, truncate_chars};
use super::view::{Focus, ViewState};

/// Hang-up to kill, when quitting or terminating a run.
const GRACE: Duration = Duration::from_secs(5);
const FLASH_FOR: Duration = Duration::from_secs(5);
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// What carries the session's effects out.
pub struct Jobs {
    checks: CheckScheduler,
    updates: UpdateRunner,
    store: ConfigStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputPurpose {
    Name,
    Script,
    Filter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfirmPurpose {
    UpdateAll,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChoicePurpose {
    Delete { app: AppId, script: Option<PathBuf> },
    Template { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Modal {
    None,
    Input(InputPurpose),
    Confirm(ConfirmPurpose),
    Choice(ChoicePurpose),
    Browser,
    Help,
}

/// The widgets and the transient UI state that is not the domain's.
struct Widgets {
    km: KeyMap,
    modal: Modal,
    input: InputBox,
    confirm: ConfirmDialog,
    choice: ChoiceDialog,
    browser: FileBrowser,
    help: HelpPopup,
    form: Option<AppForm>,
    flash: Option<(String, bool, Instant)>,
    /// Set by "entry + script": deleted once the config no longer lists it.
    delete_after_persist: Option<PathBuf>,
    tick: usize,
    pty_size: (u16, u16),
    quitting: Option<Instant>,
    should_quit: bool,
}

pub struct Tui {
    session: Session,
    jobs: Jobs,
    view: ViewState,
    widgets: Widgets,
    channel: (Sender<BgMsg>, Receiver<BgMsg>),
}

impl Tui {
    pub fn new(session: Session, store: ConfigStore, logs_dir: Option<PathBuf>) -> Self {
        let (tx, rx) = channel::<BgMsg>();
        let sink_tx = tx.clone();
        let checks = CheckScheduler::new(
            Arc::new(BashCheckRunner::default()),
            store.settings_ref().max_parallel_checks(),
            Arc::new(move |app, generation, outcome| {
                let _ = sink_tx.send(BgMsg::Fact(Fact::CheckCompleted {
                    app,
                    generation,
                    outcome,
                }));
            }),
        );
        let updates = UpdateRunner::new(Box::new(PtySpawner::new()), logs_dir);
        Tui {
            session,
            jobs: Jobs {
                checks,
                updates,
                store,
            },
            view: ViewState::new(),
            widgets: Widgets {
                km: default_key_map(),
                modal: Modal::None,
                input: InputBox::new(),
                confirm: ConfirmDialog::new(),
                choice: ChoiceDialog::new(),
                browser: FileBrowser::new(),
                help: HelpPopup::new(),
                form: None,
                flash: None,
                delete_after_persist: None,
                tick: 0,
                pty_size: (0, 0),
                quitting: None,
                should_quit: false,
            },
            channel: (tx, rx),
        }
    }

    // --- loop ---------------------------------------------------------------

    pub fn run(&mut self) -> io::Result<()> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        terminal.clear()?;

        let start = self.session.request_check_all();
        self.execute(start);

        let result = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        loop {
            self.sync_pty_size(terminal.size()?.into());
            terminal.draw(|f| self.render(f))?;
            self.drain_background();
            self.widgets.tick = self.widgets.tick.wrapping_add(1);

            if let Some(deadline) = self.widgets.quitting {
                if self.jobs.updates.active_app().is_none() || Instant::now() > deadline {
                    return Ok(());
                }
            }
            if self.widgets.should_quit {
                return Ok(());
            }

            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
                    Event::Paste(text) if self.view.focus == Focus::Terminal => {
                        self.jobs.updates.write(text.as_bytes());
                    }
                    _ => {}
                }
            }
        }
    }

    fn drain_background(&mut self) {
        while let Ok(msg) = self.channel.1.try_recv() {
            match msg {
                BgMsg::PtyOutputReceived => {}
                BgMsg::Fact(fact) => {
                    if let Fact::UpdateExited { app, outcome } = &fact {
                        self.jobs.updates.exited(*app, *outcome);
                    }
                    let effects = self.session.apply(fact);
                    self.execute(effects);
                }
            }
        }
    }

    fn layout_for(&self, area: Rect) -> Layout {
        let banner = self.jobs.updates.active_app().is_some() && self.view.focus == Focus::List;
        compute_layout(area.width, area.height, banner)
    }

    fn sync_pty_size(&mut self, area: Rect) {
        let size = self.layout_for(area).pty_size();
        if size != self.widgets.pty_size {
            self.widgets.pty_size = size;
            self.jobs.updates.resize(size.0, size.1);
        }
    }

    // --- effects ------------------------------------------------------------

    /// Carries effects out, feeding every result back as a fact, until nothing
    /// is left. `Persist` is synchronous: its answer is applied before the next
    /// key is read, so no intention can slip in while a change is pending.
    fn execute(&mut self, effects: Vec<Effect>) {
        let mut queue: VecDeque<Effect> = effects.into();
        while let Some(effect) = queue.pop_front() {
            let facts = match effect {
                Effect::ScheduleCheck {
                    app,
                    script,
                    generation,
                } => {
                    self.jobs.checks.schedule(app, script, generation);
                    vec![]
                }
                Effect::Spawn { app, script, slug } => {
                    let tx_out = self.channel.0.clone();
                    let tx_exit = self.channel.0.clone();
                    let started = self.jobs.updates.start(
                        app,
                        &script,
                        &slug,
                        self.widgets.pty_size,
                        Arc::new(move || {
                            let _ = tx_out.send(BgMsg::PtyOutputReceived);
                        }),
                        Arc::new(move |app, outcome| {
                            let _ = tx_exit.send(BgMsg::Fact(Fact::UpdateExited { app, outcome }));
                        }),
                    );
                    match started {
                        Ok(()) => {
                            self.select(app);
                            self.view.update_started();
                            vec![Fact::UpdateStarted { app }]
                        }
                        Err(reason) => vec![Fact::UpdateNotStarted { app, reason }],
                    }
                }
                Effect::TerminateActive => {
                    self.jobs.updates.terminate(GRACE);
                    vec![]
                }
                Effect::DropRunRecord(app) => {
                    self.jobs.updates.drop_record(app);
                    vec![]
                }
                Effect::Persist(catalog) => match self.jobs.store.save(&catalog) {
                    Ok(()) => {
                        if let Some(path) = self.widgets.delete_after_persist.take() {
                            match fs::remove_file(&path) {
                                Ok(()) => self.flash(&format!("Deleted {}", path.display()), false),
                                Err(e) => {
                                    self.flash(&format!("Could not delete the script: {e}"), true)
                                }
                            }
                        } else {
                            self.flash("Saved", false);
                        }
                        vec![Fact::PersistSucceeded]
                    }
                    Err(e) => {
                        self.widgets.delete_after_persist = None;
                        self.flash(&format!("Not saved: {e}"), true);
                        vec![Fact::PersistFailed {
                            reason: e.to_string(),
                        }]
                    }
                },
                Effect::UpdateEnded(_) => {
                    self.view.update_ended();
                    vec![]
                }
            };
            for fact in facts {
                queue.extend(self.session.apply(fact));
            }
        }
    }

    // --- keys ---------------------------------------------------------------

    fn handle_key(&mut self, key: KeyEvent) {
        // 1. modals
        if self.widgets.modal != Modal::None {
            self.handle_modal_key(key);
            return;
        }
        // 2. the terminal, which takes every key but its way out
        if self.view.focus == Focus::Terminal {
            self.handle_terminal_key(key);
            return;
        }
        // 3. the guard after focus came back on its own
        if self.view.ignoring_keys() {
            return;
        }
        // 4. global and list keys
        let km = &self.widgets.km;
        if km.toggle_theme.matches(&key) {
            theme::toggle_mode();
        } else if km.help.matches(&key) {
            self.widgets.help.toggle();
            self.widgets.modal = Modal::Help;
        } else if km.quit.matches(&key)
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            self.request_quit();
        } else if km.up.matches(&key) {
            self.view.selected = self.view.selected.saturating_sub(1);
        } else if km.down.matches(&key) {
            if self.view.selected + 1 < self.visible().len() {
                self.view.selected += 1;
            }
        } else if km.escape.matches(&key) {
            self.view.filter.clear();
        } else if km.filter.matches(&key) {
            let current = self.view.filter.clone();
            self.open_input(
                InputPurpose::Filter,
                "Filter",
                "type part of a name",
                &current,
            );
        } else if km.update.matches(&key) {
            if let Some(app) = self.selected_app() {
                self.intention(|s| s.request_update(app));
            }
        } else if km.update_all.matches(&key) {
            self.ask_update_all();
        } else if km.add.matches(&key) {
            let (form, step) = AppForm::add();
            self.widgets.form = Some(form);
            self.show_step(step);
        } else if km.edit.matches(&key) {
            self.start_edit();
        } else if km.delete.matches(&key) {
            self.ask_delete();
        } else if km.check.matches(&key) {
            if let Some(app) = self.selected_app() {
                self.intention(|s| s.request_check(app));
            }
        } else if km.check_all.matches(&key) {
            let effects = self.session.request_check_all();
            self.execute(effects);
        } else if km.focus_terminal.matches(&key) {
            self.focus_terminal();
        } else if km.scroll_up.matches(&key) || km.scroll_down.matches(&key) {
            self.scroll(km.scroll_up.matches(&key));
        }
    }

    fn handle_terminal_key(&mut self, key: KeyEvent) {
        let km = &self.widgets.km;
        if km.focus_list.matches(&key) {
            self.view.focus = Focus::List;
            return;
        }
        if km.scroll_up.matches(&key) || km.scroll_down.matches(&key) {
            self.scroll(km.scroll_up.matches(&key));
            return;
        }
        if self.jobs.updates.active_app().is_none() {
            // Nothing to type into: any key returns to the list.
            self.view.focus = Focus::List;
            return;
        }
        let app_cursor = self.active_screen().is_some_and(|s| s.application_cursor());
        if let Some(screen) = self.active_screen() {
            screen.reset_scroll();
        }
        // Keys are never logged: one of them may be a password.
        let bytes = key_to_bytes(key, app_cursor);
        self.jobs.updates.write(&bytes);
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        match self.widgets.modal.clone() {
            Modal::Help => {
                self.widgets.help.hide();
                self.widgets.modal = Modal::None;
            }
            Modal::Browser => match self.widgets.browser.handle_key(key) {
                BrowserOutcome::None => {}
                BrowserOutcome::Cancelled => {
                    self.reopen_script_input(None);
                }
                BrowserOutcome::PickFile(path) => {
                    let raw = spell(&path, self.jobs.store.settings_ref().scripts_dir());
                    self.reopen_script_input(Some(raw));
                }
            },
            Modal::Input(purpose) => {
                if purpose == InputPurpose::Script && key.code == KeyCode::Tab {
                    let start = self.browse_start();
                    self.widgets.input.hide();
                    self.widgets.browser.open(&start);
                    self.widgets.modal = Modal::Browser;
                    return;
                }
                match self.widgets.input.handle_key(key) {
                    Some(Action::InputSubmit(value)) => {
                        self.widgets.modal = Modal::None;
                        self.input_submitted(purpose, &value);
                    }
                    Some(Action::InputCancel) => {
                        self.widgets.modal = Modal::None;
                        self.widgets.form = None;
                    }
                    _ => {
                        if purpose == InputPurpose::Filter {
                            self.view.filter = self.widgets.input.value();
                            self.view.selected = 0;
                        }
                    }
                }
            }
            Modal::Confirm(purpose) => {
                if self.widgets.confirm.handle_key(key).is_none() {
                    return;
                }
                self.widgets.modal = Modal::None;
                if !self.widgets.confirm.confirmed {
                    return;
                }
                match purpose {
                    ConfirmPurpose::UpdateAll => self.intention(|s| s.request_update_all()),
                    ConfirmPurpose::Quit => {
                        let effects = self.session.quit();
                        self.execute(effects);
                        self.widgets.quitting =
                            Some(Instant::now() + GRACE + Duration::from_secs(1));
                    }
                }
            }
            Modal::Choice(purpose) => {
                let Some(c) = self.widgets.choice.handle_key(key) else {
                    return;
                };
                self.widgets.modal = Modal::None;
                match purpose {
                    ChoicePurpose::Delete { app, script } => match c {
                        'e' => self.intention(|s| s.remove(app)),
                        's' if script.is_some() => {
                            self.widgets.delete_after_persist = script;
                            self.intention(|s| s.remove(app));
                        }
                        _ => {}
                    },
                    ChoicePurpose::Template { path } => {
                        let created = c == 'c' && self.create_template(&path);
                        let step = match self.widgets.form.as_mut() {
                            Some(f) => f.template_answered(created),
                            None => return,
                        };
                        self.show_step(step);
                    }
                }
            }
            Modal::None => {}
        }
    }

    // --- intentions -----------------------------------------------------------

    /// Runs an intention: effects on success, a message on refusal.
    fn intention(
        &mut self,
        f: impl FnOnce(&mut Session) -> Result<Vec<Effect>, crate::session::Refusal>,
    ) {
        match f(&mut self.session) {
            Ok(effects) => self.execute(effects),
            Err(refusal) => self.flash(&refusal.to_string(), true),
        }
    }

    fn request_quit(&mut self) {
        match self.jobs.updates.active_app() {
            Some(app) => {
                let name = self.app_name(app);
                self.widgets
                    .confirm
                    .show(&format!("An update is running ({name}). Kill it and quit?"));
                self.widgets.modal = Modal::Confirm(ConfirmPurpose::Quit);
            }
            None => self.widgets.should_quit = true,
        }
    }

    fn ask_update_all(&mut self) {
        let targets = self.session.update_targets();
        if targets.is_empty() {
            self.flash("No application has an update available", true);
            return;
        }
        let names: Vec<String> = targets.iter().map(|a| self.app_name(*a)).collect();
        self.widgets.confirm.show(&format!(
            "Update {} app(s), one after the other: {}?",
            names.len(),
            names.join(", ")
        ));
        self.widgets.modal = Modal::Confirm(ConfirmPurpose::UpdateAll);
    }

    fn start_edit(&mut self) {
        let Some(app) = self.selected_app() else {
            return;
        };
        if !self.session.runtime(app).is_some_and(|r| r.can_edit()) {
            self.flash("An update is queued or running for this application", true);
            return;
        }
        let Some(application) = self.session.catalog().get(app) else {
            return;
        };
        let (form, step) = AppForm::edit(application);
        self.widgets.form = Some(form);
        self.show_step(step);
    }

    fn ask_delete(&mut self) {
        let Some(app) = self.selected_app() else {
            return;
        };
        if !self.session.runtime(app).is_some_and(|r| r.can_remove()) {
            self.flash("An update is queued or running for this application", true);
            return;
        }
        let Some(application) = self.session.catalog().get(app) else {
            return;
        };
        let script = self.deletable_script(application);
        let mut choices = vec![Choice {
            key: 'e',
            label: "remove the entry only (keep the script)".into(),
        }];
        if script.is_some() {
            choices.push(Choice {
                key: 's',
                label: "remove the entry and delete the script".into(),
            });
        }
        self.widgets.choice.show(
            &format!("Delete \"{}\"?", application.name().as_str()),
            choices,
        );
        self.widgets.modal = Modal::Choice(ChoicePurpose::Delete { app, script });
    }

    /// The script may go with its entry only when it lives in `scripts_dir`
    /// (compared by components) and no other application uses it.
    fn deletable_script(&self, application: &Application) -> Option<PathBuf> {
        let path = application.script().path();
        if !is_inside(path, self.jobs.store.settings_ref().scripts_dir()) {
            return None;
        }
        let canonical = path.canonicalize().ok()?;
        let shared = self.session.catalog().iter().any(|other| {
            other.id() != application.id()
                && other.script().path().canonicalize().ok().as_deref() == Some(&canonical)
        });
        (!shared).then_some(canonical)
    }

    // --- the form -------------------------------------------------------------

    fn show_step(&mut self, step: FormStep) {
        match step {
            FormStep::AskName { initial, error } => {
                let label = match error {
                    Some(e) => format!("Name — {e}"),
                    None => "Name".into(),
                };
                self.open_input(InputPurpose::Name, &label, "e.g. kitty", &initial);
            }
            FormStep::AskScript { initial, error } => {
                let label = match error {
                    Some(e) => format!("Script — {e}"),
                    None => "Script (tab: browse)".into(),
                };
                self.open_input(
                    InputPurpose::Script,
                    &label,
                    "kitty.sh (in scripts_dir), ~/path/x.sh or /abs/x.sh",
                    &initial,
                );
            }
            FormStep::OfferTemplate { path } => {
                self.widgets.choice.show(
                    &format!(
                        "{} does not exist. Create it from the template?",
                        path.display()
                    ),
                    vec![
                        Choice {
                            key: 'c',
                            label: "create it".into(),
                        },
                        Choice {
                            key: 'b',
                            label: "back to the path".into(),
                        },
                    ],
                );
                self.widgets.modal = Modal::Choice(ChoicePurpose::Template { path });
            }
            FormStep::Ready { name, script } => {
                let Some(form) = self.widgets.form.clone() else {
                    return;
                };
                match validate(&script) {
                    Ok(validated) => {
                        self.widgets.form = None;
                        match form.editing() {
                            Some(app) => self.intention(|s| s.edit(app, name, validated)),
                            None => self.intention(|s| s.add(name, validated)),
                        }
                    }
                    Err(e) => {
                        let step = form.script_refused(e.to_string());
                        self.show_step(step);
                    }
                }
            }
        }
    }

    fn input_submitted(&mut self, purpose: InputPurpose, value: &str) {
        match purpose {
            InputPurpose::Filter => {
                self.view.filter = value.trim().to_string();
                self.view.selected = 0;
            }
            InputPurpose::Name => {
                let Some(form) = self.widgets.form.as_mut() else {
                    return;
                };
                let step = form.name_entered(value);
                self.show_step(step);
            }
            InputPurpose::Script => {
                let scripts_dir = self.jobs.store.settings_ref().scripts_dir().to_path_buf();
                let Some(form) = self.widgets.form.as_mut() else {
                    return;
                };
                let step = form.script_entered(value, &scripts_dir);
                self.show_step(step);
            }
        }
    }

    fn reopen_script_input(&mut self, value: Option<String>) {
        let current = value.unwrap_or_else(|| self.widgets.input.value());
        self.open_input(InputPurpose::Script, "Script (tab: browse)", "", &current);
    }

    fn browse_start(&self) -> PathBuf {
        let dir = self.jobs.store.settings_ref().scripts_dir();
        if dir.is_dir() {
            dir.to_path_buf()
        } else {
            crate::config::paths::home()
        }
    }

    /// `O_EXCL`, 700: never overwrites, even if the file appeared meanwhile.
    fn create_template(&mut self, path: &Path) -> bool {
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent)?;
                    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
                }
            }
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(path)?;
            f.write_all(TEMPLATE.as_bytes())
        })();
        match result {
            Ok(()) => {
                self.flash(
                    &format!("Created {} — edit it, then press r", path.display()),
                    false,
                );
                true
            }
            Err(e) => {
                self.flash(&format!("Could not create {}: {e}", path.display()), true);
                false
            }
        }
    }

    fn open_input(&mut self, purpose: InputPurpose, label: &str, placeholder: &str, value: &str) {
        self.widgets
            .input
            .show_with_value(label, placeholder, value);
        self.widgets.modal = Modal::Input(purpose);
    }

    // --- selection ------------------------------------------------------------

    /// The applications shown, in config order, after the filter.
    fn visible(&self) -> Vec<AppId> {
        let filter = &self.view.filter;
        self.session
            .catalog()
            .iter()
            .filter(|a| fuzzy_match(a.name().as_str(), filter).is_some())
            .map(Application::id)
            .collect()
    }

    fn selected_app(&self) -> Option<AppId> {
        self.visible().get(self.view.selected).copied()
    }

    fn select(&mut self, app: AppId) {
        if !self.visible().contains(&app) {
            self.view.filter.clear();
        }
        if let Some(i) = self.visible().iter().position(|a| *a == app) {
            self.view.selected = i;
        }
    }

    fn focus_terminal(&mut self) {
        match self.jobs.updates.active_app() {
            Some(active) => {
                self.select(active);
                self.view.focus = Focus::Terminal;
            }
            None => self.flash("No update is running", false),
        }
    }

    fn active_screen(&self) -> Option<crate::pty::Screen> {
        let app = self.jobs.updates.active_app()?;
        Some(self.jobs.updates.record(app)?.screen.clone())
    }

    fn scroll(&mut self, up: bool) {
        let Some(app) = self.selected_app() else {
            return;
        };
        if let Some(r) = self.jobs.updates.record(app) {
            let page = self.widgets.pty_size.0.max(2) as isize - 1;
            r.screen.scroll(if up { page } else { -page });
        }
    }

    fn app_name(&self, app: AppId) -> String {
        self.session
            .catalog()
            .get(app)
            .map(|a| a.name().as_str().to_string())
            .unwrap_or_default()
    }

    fn flash(&mut self, msg: &str, error: bool) {
        self.widgets.flash = Some((msg.to_string(), error, Instant::now()));
    }

    // --- drawing --------------------------------------------------------------

    fn rows(&self, ids: &[AppId]) -> Vec<Row> {
        ids.iter()
            .filter_map(|id| {
                let app = self.session.catalog().get(*id)?;
                let rt = self.session.runtime(*id)?;
                Some(Row {
                    name: app.name().as_str().to_string(),
                    tag: rt.tag(),
                    versions: rt.versions().map(|v| v.label()).unwrap_or_default(),
                    detail: rt.detail(),
                })
            })
            .collect()
    }

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let layout = self.layout_for(area);
        let buf = f.buffer_mut();
        buf.set_style(area, Style::default().bg(theme::color_background()));

        let ids = self.visible();
        if self.view.selected >= ids.len() {
            self.view.selected = ids.len().saturating_sub(1);
        }
        let rows = self.rows(&ids);
        let spinner = SPINNER[(self.widgets.tick / 2) % SPINNER.len()];
        AppsPanel {
            rows: &rows,
            selected: self.view.selected,
            focused: self.view.focus == Focus::List,
            filter: &self.view.filter,
            total: self.session.catalog().len(),
            spinner,
        }
        .render(layout.list, buf);

        let selected = ids.get(self.view.selected).copied();
        let detail = selected
            .and_then(|a| self.session.runtime(a))
            .and_then(|r| r.detail());
        let name = selected.map(|a| self.app_name(a));
        TerminalPanel {
            app_name: name.as_deref(),
            record: selected.and_then(|a| self.jobs.updates.record(a)),
            running: selected.is_some() && selected == self.jobs.updates.active_app(),
            focused: self.view.focus == Focus::Terminal,
            detail: detail.as_deref(),
        }
        .render(layout.terminal, buf);

        if let Some(banner) = layout.banner {
            self.render_banner(banner, buf);
        }
        self.render_status(layout.status, buf);
        self.render_modal(area, buf);
    }

    /// While an update runs and the list has the keys, say so — louder when the
    /// run seems to be waiting for a password.
    fn render_banner(&self, area: Rect, buf: &mut Buffer) {
        let prompt = self
            .active_screen()
            .is_some_and(|s| looks_like_password_prompt(&s.last_line()));
        let (text, style) = if prompt {
            (
                " ⚠ The update is waiting for a password — press tab to type it in the terminal ",
                Style::default()
                    .fg(theme::color_background())
                    .bg(theme::color_danger())
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                " keys → list (tab to type in the terminal) ",
                Style::default()
                    .fg(theme::color_background())
                    .bg(theme::color_warning()),
            )
        };
        buf.set_string(area.x, area.y, " ".repeat(area.width as usize), style);
        buf.set_string(
            area.x,
            area.y,
            truncate_chars(text, area.width as usize),
            style,
        );
    }

    fn render_status(&mut self, area: Rect, buf: &mut Buffer) {
        if let Some((msg, error, at)) = &self.widgets.flash {
            if at.elapsed() < FLASH_FOR {
                let style = if *error {
                    styles::error_style().bg(theme::color_surface())
                } else {
                    styles::success_style().bg(theme::color_surface())
                };
                buf.set_string(
                    area.x,
                    area.y,
                    " ".repeat(area.width as usize),
                    styles::bar_style(),
                );
                buf.set_string(
                    area.x + 1,
                    area.y,
                    truncate_chars(msg, area.width as usize - 2),
                    style,
                );
                return;
            }
            self.widgets.flash = None;
        }
        let hints = match self.view.focus {
            Focus::Terminal => terminal_hints(&self.widgets.km),
            Focus::List => list_hints(&self.widgets.km),
        };
        StatusBar::render(area, buf, &hints);
    }

    fn render_modal(&self, area: Rect, buf: &mut Buffer) {
        match &self.widgets.modal {
            Modal::None => {}
            Modal::Help => self
                .widgets
                .help
                .render(centered(area, 66, 30), buf, &self.widgets.km),
            Modal::Browser => self.widgets.browser.render(centered(area, 80, 24), buf),
            Modal::Input(_) => self.widgets.input.render(centered(area, 76, 6), buf),
            Modal::Confirm(_) => {
                render_message_box(
                    centered(area, 70, 7),
                    buf,
                    " Confirm ",
                    self.widgets.confirm.message(),
                    self.widgets.confirm.hint(),
                    true,
                );
            }
            Modal::Choice(_) => {
                let view = self.widgets.choice.view();
                let mut lines = view.lines();
                let message = lines.next().unwrap_or("").to_string();
                let rest: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
                render_message_box(
                    centered(area, 76, 6 + rest.len() as u16),
                    buf,
                    " Choose ",
                    &message,
                    &rest.join("\n"),
                    false,
                );
            }
        }
    }
}

fn render_message_box(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    message: &str,
    footer: &str,
    warn: bool,
) {
    use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget, Wrap};
    Clear.render(area, buf);
    let border = if warn {
        Style::default().fg(theme::color_warning())
    } else {
        styles::border_style(true)
    };
    let block = Block::default()
        .title(title)
        .title_style(styles::block_title_style(true))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border);
    let text = format!("{message}\n\n{footer}");
    Paragraph::new(text)
        .style(styles::description_style())
        .wrap(Wrap { trim: false })
        .block(block)
        .render(area, buf);
}

/// Restores the terminal before the panic message prints, so a crash never
/// leaves the shell in raw mode on the alternate screen.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default(info);
    }));
}
