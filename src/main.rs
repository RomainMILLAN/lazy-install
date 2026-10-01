use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, LazyLock};

use clap::Parser;

use lazy_install::cli;
use lazy_install::config::paths::{default_config_path, logs_dir};
use lazy_install::config::ConfigStore;
use lazy_install::jobs::CheckRunner;
use lazy_install::script::contract::TEMPLATE;
use lazy_install::script::BashCheckRunner;
use lazy_install::session::Session;
use lazy_install::ui::style::theme;
use lazy_install::ui::tui::Tui;

/// Tells you which of your apps need an update, and runs their update script in
/// an embedded terminal.
#[derive(Parser)]
#[command(about)]
#[command(version = long_version())]
struct Cli {
    /// Use the light theme (for light terminal backgrounds)
    #[arg(long)]
    light: bool,

    /// Config file (default: ~/.config/lazy-install/config.json)
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print the script template and exit
    #[arg(long)]
    template: bool,

    /// Check every app without the TUI, print what needs attention, and exit:
    /// 0 = at least one update, 1 = no update found (some apps may be in
    /// error), 2 = nothing could be checked
    #[arg(long, conflicts_with_all = ["template", "light"])]
    check: bool,

    /// With --check: print the report as JSON
    #[arg(long, requires = "check")]
    json: bool,
}

/// `--check`: headless, so before the logger (whose init truncates the TUI's
/// debug.log) and before anything touches the terminal.
fn check(path: &Path, json: bool) -> ExitCode {
    let (store, catalog) = match ConfigStore::load(path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("lazy-install: {e}");
            return ExitCode::from(2);
        }
    };
    if catalog.is_empty() {
        eprintln!(
            "lazy-install: no application configured in {}",
            path.display()
        );
        return ExitCode::from(2);
    }
    let bash = BashCheckRunner::default();
    let per_check = bash.worst_case();
    let runner: Arc<dyn CheckRunner> = Arc::new(bash);
    let workers = store.settings_ref().max_parallel_checks();
    let deadline = cli::budget(catalog.len(), workers, per_check);
    let report = cli::collect(catalog, runner, workers, deadline);
    if json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.to_text());
    }
    // The process convention lives here, not in the report.
    if report.has_updates() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// OS and arch are read here, in the binary, where they describe the target; a
/// build script only knows the host and would mislabel every cross build.
static LONG_VERSION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "version={}, commit={}, build date={}, os={}, arch={}",
        env!("CARGO_PKG_VERSION"),
        env!("LI_GIT_COMMIT"),
        env!("LI_BUILD_DATE"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
});

fn long_version() -> &'static str {
    LONG_VERSION.as_str()
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.template {
        print!("{TEMPLATE}");
        return ExitCode::SUCCESS;
    }

    let path = cli.config.unwrap_or_else(default_config_path);
    if cli.check {
        return check(&path, cli.json);
    }

    if let Err(e) = lazy_install::logger::init() {
        eprintln!("Warning: could not init logger: {e}");
    }

    // An unreadable config stops here, untouched: starting empty would let the
    // next save erase it.
    let (store, catalog) = match ConfigStore::load(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("lazy-install: {e}");
            return ExitCode::from(2);
        }
    };

    if cli.light {
        theme::set_mode(theme::ThemeMode::Light);
    } else {
        theme::set_mode(theme::detect_mode());
    }

    log::info!(
        "starting lazy-install with {} apps from {}",
        catalog.len(),
        path.display()
    );
    let mut tui = Tui::new(Session::new(catalog), store, Some(logs_dir()));
    if let Err(e) = tui.run() {
        log::error!("program error: {e}");
        eprintln!("Error: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
