//! `resilience` — the Resilience OS terminal interface.
//!
//! ```text
//! resilience            start the interactive interface (needs a terminal)
//! resilience status     print a machine-readable status summary
//! resilience apps       list catalog entries and their availability
//! resilience --version  print the version
//! ```

mod app;
mod help;
mod theme;
mod ui;

use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};
use resilience_core::catalog::{Availability, Catalog};
use resilience_core::config::{self, SystemConfig, UserSettings};
use resilience_core::{launch, mode, network, status};

use crate::app::{App, Effect, Paths, RunRequest, Startup};

const USAGE: &str = "\
Usage: resilience [COMMAND]

Commands:
  (none)      Start the interactive Resilience interface
  status      Print a status summary (key: value lines)
  apps        List application catalog entries and their availability
  --version   Print the version
  --help      Print this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_interactive(),
        Some("status") => cli_status(),
        Some("apps") => cli_apps(),
        Some("-V" | "--version" | "version") => {
            println!("resilience {}", resilience_core::VERSION);
            ExitCode::SUCCESS
        }
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("resilience: unknown command '{other}'\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Gather configuration. Problems become warnings shown inside the
/// interface: a broken catalog must not prevent the interface from starting.
fn startup() -> Startup {
    let mut warnings = Vec::new();
    let sys = match SystemConfig::load() {
        Ok((c, path, found)) => {
            if !found {
                warnings.push(format!(
                    "{} not found; using built-in defaults",
                    path.display()
                ));
            }
            c
        }
        Err(e) => {
            warnings.push(format!("{e}; using built-in defaults"));
            SystemConfig::default()
        }
    };
    let catalog = match Catalog::load(&sys.catalog) {
        Ok(c) => c,
        Err(e) => {
            warnings.push(format!("application catalog unavailable: {e}"));
            Catalog::default()
        }
    };
    let home = config::home_dir();
    let settings_path = UserSettings::path(&config::config_home(&home));
    let (settings, warn) = UserSettings::load(&settings_path);
    warnings.extend(warn);
    Startup {
        sys,
        catalog,
        who: mode::detect(),
        home,
        settings,
        settings_path,
        paths: Paths::default(),
        warnings,
    }
}

fn cli_status() -> ExitCode {
    let s = startup();
    let snap = status::snapshot(&s.home);
    let net = network::passive_state();
    let state = status::system_state();
    let mode = s.who.mode;
    let ready = s
        .catalog
        .apps
        .iter()
        .filter(|a| {
            mode.allows(a.min_mode) && matches!(a.availability(mode), Availability::Ready(_))
        })
        .count();
    let visible = s
        .catalog
        .apps
        .iter()
        .filter(|a| mode.allows(a.min_mode))
        .count();
    println!("version: {}", resilience_core::VERSION);
    println!("edition: {}", s.sys.edition);
    println!("user: {}", s.who.user);
    println!(
        "mode: {}",
        match mode {
            mode::Mode::UltraStable => "ultra-stable",
            mode::Mode::Stable => "stable",
            mode::Mode::Advanced => "advanced",
        }
    );
    println!("system: {}", state.label().to_lowercase());
    println!("network: {}", net.level.label().to_lowercase());
    println!(
        "gateway: {}",
        net.gateway_v4
            .map(|(i, g)| format!("{g} via {i}"))
            .unwrap_or_else(|| "none".into())
    );
    println!("os: {}", snap.os_name);
    println!("apps: {visible} visible, {ready} ready");
    for w in &s.warnings {
        println!("warning: {w}");
    }
    ExitCode::SUCCESS
}

fn cli_apps() -> ExitCode {
    let s = startup();
    let mode = s.who.mode;
    for a in &s.catalog.apps {
        let avail = if !mode.allows(a.min_mode) {
            format!("requires-{}", a.min_mode.label().to_lowercase())
        } else {
            match a.availability(mode) {
                Availability::Ready(_) => "ready".to_string(),
                Availability::Missing(p) => format!("unavailable({p})"),
            }
        };
        println!("{}\t{:?}\t{:?}\t{avail}", a.id, a.category, a.min_mode);
    }
    for w in &s.warnings {
        eprintln!("warning: {w}");
    }
    ExitCode::SUCCESS
}

/// Log to `$XDG_STATE_HOME/resilience/tui.log` (never to the terminal, which
/// belongs to the interface). Logging failures are not fatal.
fn init_logging(home: &Path) {
    let dir = config::state_home(home).join("resilience");
    if fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .is_err()
    {
        return;
    }
    let path = dir.join("tui.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = fs::rename(&path, dir.join("tui.log.old"));
    }
    let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let filter = if std::env::var_os("RESILIENCE_DEBUG").is_some() {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    let _ = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_max_level(filter)
        .try_init();
}

fn run_interactive() -> ExitCode {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        eprintln!(
            "resilience: the interactive interface needs a terminal (try 'resilience status')"
        );
        return ExitCode::from(2);
    }
    let s = startup();
    init_logging(&s.home);
    tracing::info!(version = resilience_core::VERSION, user = %s.who.user, mode = %s.who.mode, "interface starting");

    // Log panics before ratatui's hook restores the terminal.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(%info, "interface panicked");
        previous(info);
    }));

    let mut app = App::new(s);
    // Before the screen is taken: loadkeys may print to the console.
    app.apply_saved_layout();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    match result {
        Ok(()) => {
            tracing::info!("interface exited normally");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(error = %e, "interface failed");
            eprintln!("resilience: {e}");
            ExitCode::FAILURE
        }
    }
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(k) => match app.handle_key(k) {
                    Effect::None => {}
                    Effect::Quit => return Ok(()),
                    Effect::Run(req) => run_program(terminal, app, &req)?,
                },
                Event::Resize(..) => {}
                _ => {}
            }
        } else {
            app.on_tick();
        }
    }
}

/// Release the terminal, run a program in the foreground, take it back.
fn run_program(terminal: &mut DefaultTerminal, app: &mut App, req: &RunRequest) -> io::Result<()> {
    ratatui::restore();
    let mut out = io::stdout();
    // Clear the screen so the program starts on a clean console.
    let _ = write!(out, "\x1b[2J\x1b[H");
    if let Some(b) = &req.banner {
        let _ = writeln!(out, "{b}\n");
    }
    let _ = out.flush();
    let extra: Vec<&std::ffi::OsStr> = req.extra_args.iter().map(|p| p.as_os_str()).collect();
    let result = match &req.polkit_agent {
        Some(agent) => launch::run_with_polkit_agent(agent, &req.argv, &req.env),
        None => launch::run_foreground_env(&req.argv, &extra, &req.env, req.clean_env),
    };
    let failed = !matches!(&result, Ok(st) if st.success());
    if req.pause_after || failed {
        match &result {
            Ok(st) if !st.success() => println!("\n[{} exited with {st}]", req.label),
            Err(e) => println!("\n[{} could not start: {e}]", req.label),
            _ => {}
        }
        print!("\nPress Enter to return to Resilience...");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    app.after_run(&req.label, &result);
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}

#[cfg(test)]
mod tests;
