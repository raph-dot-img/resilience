//! `resilience-maps` — the offline map viewer of Resilience OS (ADR 0016).
//!
//! Runs as the user, in the foreground, never with privileges. Map files
//! are untrusted; this process lowers its own address-space limit to
//! 1 GiB before it parses anything.
//!
//! ```text
//! resilience-maps view [--pack FILE]... [--at LAT,LON] [--scale M] [--search WORDS]
//!                                       the map viewer (needs a terminal);
//!                                       see src/args.rs
//! resilience-maps info [--check] FILE   what a map pack says about itself
//!                                       (format: src/info.rs); --check also
//!                                       runs SQLite's quick_check
//! ```
//!
//! Exit status: 0 success; 1 the file cannot be read (missing, permission)
//! or another error (no terminal); 2 the file is refused (not a map pack,
//! damaged or unsafe; for `view`: every pack given was refused); 64 usage
//! error.
//!
//! The viewer starts no program and writes only its last position
//! (`~/.local/state/resilience/maps.toml`). The terminal is restored on
//! every way out: `q`, Ctrl-C, SIGTERM, SIGHUP and SIGINT (a flag the event
//! loop sees within 50 ms), errors, and panics of the main thread
//! (ratatui's panic hook). A panic of the reader thread does not touch the
//! terminal: the viewer says the reader stopped.

use std::ffi::OsString;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};
use resilience_core::config::{self, UserSettings};
use resilience_core::packs::PACKS_CONFIG_PATH;
use resilience_maps::app::{Effect, MapApp, Start};
use resilience_maps::info;
use resilience_maps::loader::{self, Catalog, Channel, Worker};
use resilience_maps::pack::Pack;
use resilience_maps::rlimit::{ADDRESS_SPACE_LIMIT, limit_address_space};
use resilience_maps::text::clean;
use resilience_maps::{args, position, ui};

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;

const USAGE: &str = "\
Usage:
  resilience-maps view [--pack FILE]... [--at LAT,LON] [--scale M]
                                        Show maps (at most 8 packs)
  resilience-maps info [--check] FILE   Show what a map pack says about itself";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "view") {
        return run_view(&args[1..]);
    }
    let Some(argv) = args
        .iter()
        .map(|a| a.to_str())
        .collect::<Option<Vec<&str>>>()
    else {
        eprintln!("{USAGE}");
        return ExitCode::from(EXIT_USAGE);
    };
    match argv.as_slice() {
        ["info", file] => run_info(Path::new(file), false),
        ["info", "--check", file] => run_info(Path::new(file), true),
        ["--version"] => {
            println!("resilience-maps {}", resilience_core::VERSION);
            ExitCode::SUCCESS
        }
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn run_info(file: &Path, check: bool) -> ExitCode {
    if let Err(e) = limit_address_space(ADDRESS_SPACE_LIMIT) {
        eprintln!("Error: cannot limit memory use: {e}");
        return ExitCode::from(EXIT_ERROR);
    }
    let name = clean(&file.to_string_lossy());
    match Pack::open(file) {
        Ok(pack) => {
            let result = check.then(|| pack.quick_check());
            print!("{}", info::render(&pack, result.as_ref()));
            match result {
                Some(Err(e)) => {
                    eprintln!("Refused: {name}: {e}");
                    ExitCode::from(EXIT_REFUSED)
                }
                _ => ExitCode::SUCCESS,
            }
        }
        Err(e) if e.is_refusal() => {
            eprintln!("Refused: {name}: {e}");
            ExitCode::from(EXIT_REFUSED)
        }
        Err(e) => {
            eprintln!("Error: {name}: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// Set by SIGTERM, SIGHUP and SIGINT; the event loop then leaves cleanly.
static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic, which is
        // async-signal-safe.
        unsafe {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
}

/// Keep ratatui's terminal-restoring panic hook for the main thread only:
/// the reader thread's panic must not leave the screen to a running
/// viewer (the viewer notices the reader is gone and says so).
fn main_thread_only_panic_hook() {
    let restoring = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some("main") {
            restoring(info);
        }
    }));
}

fn run_view(argv: &[OsString]) -> ExitCode {
    if let Err(e) = limit_address_space(ADDRESS_SPACE_LIMIT) {
        eprintln!("Error: cannot limit memory use: {e}");
        return ExitCode::from(EXIT_ERROR);
    }
    let a = match args::parse_view(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("resilience-maps view: {e}\n\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("resilience-maps view: the map viewer needs a terminal");
        return ExitCode::from(EXIT_ERROR);
    }
    let home = config::home_dir();
    let saved_at = position::path(&config::state_home(&home));
    let (settings, _) = UserSettings::load(&UserSettings::path(&config::config_home(&home)));
    let catalog = Catalog::load(Path::new(PACKS_CONFIG_PATH));
    let mut app = MapApp::new(Start {
        packs: a.packs,
        at: a.at,
        scale: a.scale,
        search: a.search,
        saved: position::load(&saved_at),
        theme: settings.theme,
        now: Instant::now(),
    });
    let channel = match loader::spawn(Worker::new(catalog)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: cannot start the map reader: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    install_signal_handlers();
    let mut terminal = match ratatui::try_init() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: cannot use the terminal: {e}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    main_thread_only_panic_hook();
    let result = event_loop(&mut terminal, &mut app, &channel);
    ratatui::restore();
    drop(channel);
    if let Some(v) = app.view_to_save()
        && let Err(e) = position::save(&saved_at, &v)
    {
        eprintln!("Note: the map position could not be saved: {e}");
    }
    for f in app.failures() {
        eprintln!("Refused: {f}");
    }
    match result {
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::from(EXIT_ERROR)
        }
        Ok(()) => ExitCode::from(app.exit_code()),
    }
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut MapApp, ch: &Channel) -> io::Result<()> {
    loop {
        if STOP.load(Ordering::SeqCst) {
            return Ok(());
        }
        app.on_tick(Instant::now());
        match ch.ready() {
            Ok(responses) => {
                for r in responses {
                    app.on_response(r);
                }
            }
            Err(loader::ReaderGone) => app.on_reader_lost(),
        }
        if app.dirty {
            app.dirty = false;
            terminal.draw(|f| ui::draw(f, app))?;
        }
        // Drawing plans the view: send what it needs read.
        for r in app.take_requests() {
            if ch.requests.send(r).is_err() {
                app.on_reader_lost();
            }
        }
        let ready = match event::poll(Duration::from_millis(50)) {
            Ok(r) => r,
            // A signal: the loop looks at STOP first.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if ready {
            match event::read()? {
                Event::Key(k) => {
                    if app.handle_key(k) == Effect::Quit {
                        return Ok(());
                    }
                }
                Event::Resize(..) => app.dirty = true,
                _ => {}
            }
        }
    }
}
