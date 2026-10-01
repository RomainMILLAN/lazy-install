use std::path::PathBuf;
use std::process;
use std::sync::LazyLock;

use clap::Parser;

use lazy_install::config::paths::{default_config_path, logs_dir};
use lazy_install::config::ConfigStore;
use lazy_install::script::contract::TEMPLATE;
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

fn main() {
    let cli = Cli::parse();

    if cli.template {
        print!("{TEMPLATE}");
        return;
    }

    if let Err(e) = lazy_install::logger::init() {
        eprintln!("Warning: could not init logger: {e}");
    }

    let path = cli.config.unwrap_or_else(default_config_path);
    // An unreadable config stops here, untouched: starting empty would let the
    // next save erase it.
    let (store, catalog) = match ConfigStore::load(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("lazy-install: {e}");
            process::exit(2);
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
        process::exit(1);
    }
}
