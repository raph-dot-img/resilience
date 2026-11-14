//! Application state and input handling.
//!
//! The [`App`] owns all state; [`App::handle_key`] is a pure state transition
//! that returns an [`Effect`] for the main loop to execute (running a program
//! needs the terminal to be released, which only the main loop can do). This
//! keeps all navigation logic testable without a terminal.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use resilience_core::admin::{self, AccountRow, AdminConfig, ModeState};
use resilience_core::backup::{self, BackupFile};
use resilience_core::catalog::{Availability, Catalog, Category};
use resilience_core::clock;
use resilience_core::config::{SystemConfig, ThemeName, UserSettings};
use resilience_core::handbook;
use resilience_core::human_bytes;
use resilience_core::media::{self, Ejected, Volume};
use resilience_core::mode::{Detected, Mode};
use resilience_core::network::{self, NetworkState, ProbeConfig, ProbeReport};
use resilience_core::notes;
use resilience_core::proc::run_full;
use resilience_core::recovery;
use resilience_core::status::{self, Snapshot, SystemState};

use crate::theme::Theme;

const FAST_REFRESH: Duration = Duration::from_secs(5);
const SLOW_REFRESH: Duration = Duration::from_secs(30);
const TOAST_DURATION: Duration = Duration::from_secs(6);
const MAX_NOTICES: usize = 100;
/// Label of the run request that changes a security mode (see `after_run`).
pub const SET_MODE_LABEL: &str = "Change security mode";
pub const SET_TIME_LABEL: &str = "Set the clock";
/// Labels of the backup tool runs (see `after_run`).
pub const BACKUP_LABEL: &str = "Backup of your files";
pub const CHECK_LABEL: &str = "Check a backup";
pub const RESTORE_LABEL: &str = "Restore a backup";

/// System files the interface reads. Real paths by default; tests point
/// them into a temporary directory.
#[derive(Debug, Clone)]
pub struct Paths {
    pub admin_config: PathBuf,
    pub group_file: PathBuf,
    pub profiles: PathBuf,
    pub booted_system: PathBuf,
    pub running_system: PathBuf,
}

impl Default for Paths {
    fn default() -> Paths {
        Paths {
            admin_config: admin::ADMIN_CONFIG_PATH.into(),
            group_file: "/etc/group".into(),
            profiles: recovery::PROFILES_DIR.into(),
            booted_system: "/run/booted-system".into(),
            running_system: "/run/current-system".into(),
        }
    }
}

/// Entries of the Recovery section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryItem {
    Health,
    Generations,
    Report,
    Emergency,
}

impl RecoveryItem {
    pub const ALL: [RecoveryItem; 4] = [
        RecoveryItem::Health,
        RecoveryItem::Generations,
        RecoveryItem::Report,
        RecoveryItem::Emergency,
    ];

    pub fn name(self) -> &'static str {
        match self {
            RecoveryItem::Health => "System health & recent failures",
            RecoveryItem::Generations => "System generations (roll back)",
            RecoveryItem::Report => "Export a diagnostic report",
            RecoveryItem::Emergency => "Emergency guide",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            RecoveryItem::Health => {
                "Overall state, services that failed, and recent error messages from this boot."
            }
            RecoveryItem::Generations => {
                "Every earlier version of the system is kept. Shows them and how to start one \
                 from the boot menu. Your personal files are not affected by a system rollback."
            }
            RecoveryItem::Report => {
                "Saves a plain-text report (status, failures, generations, interface log) in \
                 your Diagnostics folder, to show to someone who can help. Contains no passwords."
            }
            RecoveryItem::Emergency => "What to do when something is wrong: step-by-step, offline.",
        }
    }
}

/// Functions built into the interface (as opposed to catalog programs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Handbook,
    Notes,
    Network,
    SystemStatus,
    Notifications,
    Settings,
    Clock,
    Users,
    Recovery,
    Backup,
    Help,
    Shell,
    Logout,
    Reboot,
    PowerOff,
}

impl Builtin {
    pub const ALL: [Builtin; 15] = [
        Builtin::Handbook,
        Builtin::Notes,
        Builtin::Network,
        Builtin::SystemStatus,
        Builtin::Recovery,
        Builtin::Backup,
        Builtin::Notifications,
        Builtin::Settings,
        Builtin::Clock,
        Builtin::Users,
        Builtin::Help,
        Builtin::Shell,
        Builtin::Logout,
        Builtin::Reboot,
        Builtin::PowerOff,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Handbook => "Resilience Handbook",
            Builtin::Notes => "Notes",
            Builtin::Network => "Network status & diagnostics",
            Builtin::SystemStatus => "System status",
            Builtin::Notifications => "Notification history",
            Builtin::Settings => "Settings",
            Builtin::Clock => "Date & time",
            Builtin::Users => "Users & security modes",
            Builtin::Recovery => "Recovery & diagnostics",
            Builtin::Backup => "Backups & USB sticks",
            Builtin::Help => "Help & keyboard shortcuts",
            Builtin::Shell => "Terminal shell",
            Builtin::Logout => "Log out",
            Builtin::Reboot => "Restart computer",
            Builtin::PowerOff => "Power off",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Builtin::Handbook => {
                "The built-in user guide: first steps, security modes, recovery. Works fully offline."
            }
            Builtin::Notes => {
                "Write and keep personal text notes. Notes are private files in your Notes folder."
            }
            Builtin::Network => {
                "Shows whether this computer has a network link, a route, and (on request) Internet access."
            }
            Builtin::SystemStatus => {
                "Detailed health report: services, storage, memory, power, system generation."
            }
            Builtin::Notifications => "Messages the interface has shown during this session.",
            Builtin::Settings => "Choose the colour theme. Your security mode is shown here too.",
            Builtin::Clock => {
                "Shows the date, time and time zone, and sets the clock when it is wrong \
                 (there is no network time). Setting it asks for an administrator password."
            }
            Builtin::Users => {
                "See each account's security mode. An administrator can change a mode here; \
                 every change asks for an administrator password and is recorded."
            }
            Builtin::Recovery => {
                "When something is wrong: system health, earlier system versions, a report \
                 to share, and the emergency guide."
            }
            Builtin::Backup => {
                "Save your files, encrypted with a passphrase, to a USB stick; check a backup; \
                 restore one into a new folder. Also ejects sticks safely."
            }
            Builtin::Help => "How to move around the interface and what each key does.",
            Builtin::Shell => "An unprivileged command-line shell. Type 'exit' to return here.",
            Builtin::Logout => "End this session.",
            Builtin::Reboot => "Restart the computer. Unsaved work in open programs is lost.",
            Builtin::PowerOff => "Shut the computer down safely.",
        }
    }

    pub fn category(self) -> Category {
        match self {
            Builtin::Handbook => Category::Knowledge,
            Builtin::Notes => Category::Productivity,
            Builtin::Network => Category::Networking,
            _ => Category::System,
        }
    }

    pub fn min_mode(self) -> Mode {
        match self {
            Builtin::Shell => Mode::Stable,
            _ => Mode::UltraStable,
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Builtin::Handbook => "help guide manual documentation docs",
            Builtin::Notes => "write text memo journal",
            Builtin::Network => "wifi ethernet internet offline online dns",
            Builtin::SystemStatus => "health battery disk storage memory",
            Builtin::Settings => "theme colour color preferences",
            Builtin::Clock => "clock time date hour timezone zone ntp calendar rtc wrong",
            Builtin::Users => {
                "account accounts mode modes administrator admin permissions privileges sudo"
            }
            Builtin::Recovery => {
                "rescue repair rollback generation failure failed diagnostics report emergency broken"
            }
            Builtin::Backup => {
                "backup backups restore usb stick key removable media drive disk eject copy save sd card"
            }
            Builtin::Shell => "terminal command bash console",
            Builtin::PowerOff => "shutdown halt",
            Builtin::Reboot => "reboot",
            _ => "",
        }
    }
}

pub fn category_description(c: Category) -> &'static str {
    match c {
        Category::Knowledge => "Offline guides, manuals and reference material.",
        Category::Maps => "Offline maps, places and navigation.",
        Category::Communications => "Messaging over local networks and the Internet.",
        Category::Networking => "Network status, diagnostics and configuration.",
        Category::Radio => "Receive-only radio and SDR tools.",
        Category::Security => "Encryption, signatures and integrity checks.",
        Category::Productivity => "Editors, notes, files, calculators and calendars.",
        Category::Entertainment => "Games and leisure that work offline.",
        Category::System => "Status, settings, power and session management.",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    App(usize),
    Builtin(Builtin),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    Home,
    Category,
    Reader,
    Notes,
    Network,
    Settings,
    Search,
    Users,
    Recovery,
    Backup,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchHit {
    Category(Category),
    Entry(Entry),
    Page(handbook::Page),
}

#[derive(Debug)]
pub enum View {
    Home {
        list: ListState,
    },
    Category {
        cat: Category,
        list: ListState,
    },
    Handbook {
        pages: Vec<handbook::Page>,
        list: ListState,
        error: Option<String>,
    },
    Pager {
        title: String,
        text: String,
        scroll: usize,
    },
    Notes {
        notes: Vec<notes::Note>,
        list: ListState,
    },
    Network {
        scroll: usize,
    },
    Status {
        scroll: usize,
    },
    Settings {
        list: ListState,
    },
    Search {
        query: String,
        hits: Vec<SearchHit>,
        list: ListState,
    },
    Users {
        rows: Vec<AccountRow>,
        error: Option<String>,
        list: ListState,
    },
    ChooseMode {
        row: AccountRow,
        list: ListState,
    },
    Recovery {
        list: ListState,
    },
    /// Removable volumes.
    Media {
        volumes: Vec<Volume>,
        error: Option<String>,
        list: ListState,
    },
    /// Backups on one mounted volume; row 0 is "make a new backup".
    Backups {
        volume: Volume,
        mount: PathBuf,
        files: Vec<BackupFile>,
        list: ListState,
    },
    Help {
        topic: HelpTopic,
        scroll: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Logout,
    Reboot,
    PowerOff,
    SetMode {
        user: String,
        to: Mode,
    },
    /// Seconds since the epoch, validated by `clock::check_bounds`.
    SetTime {
        utc: i64,
    },
    Backup {
        mount: PathBuf,
    },
    Restore {
        file: PathBuf,
    },
}

#[derive(Debug)]
pub enum Modal {
    Confirm {
        title: String,
        body: String,
        confirm: String,
        action: Pending,
        yes_focused: bool,
    },
    Message {
        title: String,
        body: String,
        error: bool,
    },
    NewNote {
        value: String,
        error: Option<String>,
    },
    /// Type a new local date and time.
    SetTime {
        value: String,
        zone: String,
        error: Option<String>,
    },
}

/// A program to run in the foreground with the terminal released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub label: String,
    pub argv: Vec<String>,
    pub extra_args: Vec<PathBuf>,
    /// Environment overrides for the child (most runs: none).
    pub env: Vec<(String, String)>,
    /// For pkexec commands: polkit text agent to register first.
    pub polkit_agent: Option<String>,
    pub pause_after: bool,
    pub banner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    None,
    Quit,
    Run(RunRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub level: Level,
    pub text: String,
    pub at: SystemTime,
}

/// Everything the interface knows about the machine.
pub struct Probe {
    pub config: ProbeConfig,
    pub report: Option<ProbeReport>,
    pub running: Option<(mpsc::Receiver<ProbeReport>, Instant)>,
}

pub struct App {
    pub sys: SystemConfig,
    pub catalog: Catalog,
    pub who: Detected,
    pub home: PathBuf,
    pub settings: UserSettings,
    pub settings_path: PathBuf,
    pub paths: Paths,
    pub theme: Theme,
    pub snapshot: Snapshot,
    pub sys_state: SystemState,
    pub failed_units: Vec<String>,
    pub net: NetworkState,
    pub probe: Probe,
    pub stack: Vec<View>,
    pub modal: Option<Modal>,
    pub notices: Vec<Notice>,
    pub toast: Option<(usize, Instant)>,
    pub tick: u64,
    last_fast: Instant,
    last_slow: Instant,
}

/// Inputs gathered by `main` before the interface starts.
pub struct Startup {
    pub sys: SystemConfig,
    pub catalog: Catalog,
    pub who: Detected,
    pub home: PathBuf,
    pub settings: UserSettings,
    pub settings_path: PathBuf,
    pub paths: Paths,
    pub warnings: Vec<String>,
}

impl App {
    pub fn new(s: Startup) -> App {
        let (probe_config, invalid) = s.sys.probe.to_probe_config();
        let mut app = App {
            theme: Theme::new(s.settings.theme),
            snapshot: status::snapshot(&s.home),
            sys_state: SystemState::Unknown("not checked yet".into()),
            failed_units: Vec::new(),
            net: network::passive_state(),
            probe: Probe {
                config: probe_config,
                report: None,
                running: None,
            },
            stack: vec![View::Home {
                list: ListState::default().with_selected(Some(0)),
            }],
            modal: None,
            notices: Vec::new(),
            toast: None,
            tick: 0,
            last_fast: Instant::now(),
            last_slow: Instant::now(),
            sys: s.sys,
            catalog: s.catalog,
            who: s.who,
            home: s.home,
            settings: s.settings,
            settings_path: s.settings_path,
            paths: s.paths,
        };
        app.refresh_slow();
        for w in s.warnings {
            app.notify(Level::Warn, w);
        }
        for t in invalid {
            app.notify(Level::Warn, format!("ignored invalid probe target '{t}'"));
        }
        app
    }

    pub fn mode(&self) -> Mode {
        self.who.mode
    }

    // ----- periodic work ---------------------------------------------------

    pub fn refresh_fast(&mut self) {
        self.snapshot = status::snapshot(&self.home);
        self.net = network::passive_state();
        self.last_fast = Instant::now();
    }

    pub fn refresh_slow(&mut self) {
        self.sys_state = status::system_state();
        self.failed_units = status::failed_units();
        self.last_slow = Instant::now();
    }

    /// Called roughly four times per second by the main loop.
    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.last_fast.elapsed() >= FAST_REFRESH {
            self.refresh_fast();
        }
        if self.last_slow.elapsed() >= SLOW_REFRESH {
            self.refresh_slow();
        }
        if let Some((rx, _)) = &self.probe.running {
            match rx.try_recv() {
                Ok(report) => {
                    self.probe.running = None;
                    let ok = report.internet.is_ok();
                    self.probe.report = Some(report);
                    if ok {
                        self.notify(Level::Info, "Internet probe succeeded.");
                    } else {
                        self.notify(
                            Level::Warn,
                            "Internet is not reachable (details in Network status).",
                        );
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.probe.running = None;
                    self.notify(Level::Error, "Network probe stopped unexpectedly.");
                }
            }
        }
        if self
            .toast
            .is_some_and(|(_, t)| t.elapsed() >= TOAST_DURATION)
        {
            self.toast = None;
        }
    }

    pub fn start_probe(&mut self) {
        if self.probe.running.is_some() {
            return;
        }
        self.net = network::passive_state();
        let (tx, rx) = mpsc::channel();
        let cfg = self.probe.config.clone();
        let level = self.net.level;
        thread::spawn(move || {
            let _ = tx.send(network::probe(&cfg, level));
        });
        self.probe.running = Some((rx, Instant::now()));
        tracing::info!(?level, "network probe started");
    }

    pub fn notify(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        match level {
            Level::Info => tracing::info!(%text, "notice"),
            Level::Warn => tracing::warn!(%text, "notice"),
            Level::Error => tracing::error!(%text, "notice"),
        }
        self.notices.push(Notice {
            level,
            text,
            at: SystemTime::now(),
        });
        if self.notices.len() > MAX_NOTICES {
            self.notices.remove(0);
        }
        self.toast = Some((self.notices.len() - 1, Instant::now()));
    }

    pub fn current_toast(&self) -> Option<&Notice> {
        self.toast.and_then(|(i, _)| self.notices.get(i))
    }

    // ----- menu model -------------------------------------------------------

    /// Entries offered in a category for the current mode: built-ins first.
    pub fn entries(&self, cat: Category) -> Vec<Entry> {
        let mode = self.mode();
        Builtin::ALL
            .iter()
            .filter(|b| b.category() == cat && mode.allows(b.min_mode()))
            .map(|b| Entry::Builtin(*b))
            .chain(
                self.catalog
                    .in_category(cat, mode)
                    .map(|(i, _)| Entry::App(i)),
            )
            .collect()
    }

    /// Categories with at least one entry. Empty categories are not shown,
    /// so the menu never contains an item that does nothing.
    pub fn home_categories(&self) -> Vec<Category> {
        Category::ALL
            .into_iter()
            .filter(|c| !self.entries(*c).is_empty())
            .collect()
    }

    pub fn entry_name(&self, e: Entry) -> String {
        match e {
            Entry::App(i) => self.catalog.apps[i].name.clone(),
            Entry::Builtin(b) => b.name().to_string(),
        }
    }

    pub fn entry_ready(&self, e: Entry) -> bool {
        match e {
            Entry::App(i) => matches!(
                self.catalog.apps[i].availability(self.mode()),
                Availability::Ready(_)
            ),
            Entry::Builtin(_) => true,
        }
    }

    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mode = self.mode();
        let mut hits: Vec<SearchHit> = self
            .home_categories()
            .into_iter()
            .filter(|c| c.title().to_lowercase().contains(&q))
            .map(SearchHit::Category)
            .collect();
        hits.extend(
            Builtin::ALL
                .iter()
                .filter(|b| mode.allows(b.min_mode()))
                .filter(|b| {
                    [b.name(), b.description(), b.keywords()]
                        .iter()
                        .any(|s| s.to_lowercase().contains(&q))
                })
                .map(|b| SearchHit::Entry(Entry::Builtin(*b))),
        );
        hits.extend(
            self.catalog
                .apps
                .iter()
                .enumerate()
                .filter(|(_, a)| mode.allows(a.min_mode) && a.matches(&q))
                .map(|(i, _)| SearchHit::Entry(Entry::App(i))),
        );
        if let Ok(pages) = handbook::list(&self.sys.handbook_dir) {
            hits.extend(
                pages
                    .into_iter()
                    .filter(|p| p.title.to_lowercase().contains(&q))
                    .map(SearchHit::Page),
            );
        }
        hits
    }

    // ----- navigation -------------------------------------------------------

    pub fn view(&self) -> &View {
        self.stack.last().expect("view stack is never empty")
    }

    fn push(&mut self, v: View) {
        self.stack.push(v);
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn help_topic(&self) -> HelpTopic {
        match self.view() {
            View::Home { .. } => HelpTopic::Home,
            View::Category { .. } => HelpTopic::Category,
            View::Handbook { .. }
            | View::Pager { .. }
            | View::Status { .. }
            | View::Help { .. } => HelpTopic::Reader,
            View::Notes { .. } => HelpTopic::Notes,
            View::Network { .. } => HelpTopic::Network,
            View::Settings { .. } => HelpTopic::Settings,
            View::Users { .. } | View::ChooseMode { .. } => HelpTopic::Users,
            View::Recovery { .. } => HelpTopic::Recovery,
            View::Media { .. } | View::Backups { .. } => HelpTopic::Backup,
            View::Search { .. } => HelpTopic::Search,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if key.kind != KeyEventKind::Press {
            return Effect::None;
        }
        if let Some(modal) = self.modal.take() {
            return self.handle_modal(modal, key);
        }
        let typing = matches!(self.view(), View::Search { .. });
        match key.code {
            KeyCode::F(1) => {
                let topic = self.help_topic();
                if !matches!(self.view(), View::Help { .. }) {
                    self.push(View::Help { topic, scroll: 0 });
                }
                return Effect::None;
            }
            KeyCode::Char('?') if !typing => {
                let topic = self.help_topic();
                if !matches!(self.view(), View::Help { .. }) {
                    self.push(View::Help { topic, scroll: 0 });
                }
                return Effect::None;
            }
            KeyCode::F(5) => {
                self.refresh_fast();
                self.refresh_slow();
                if !matches!(self.view(), View::Status { .. }) {
                    self.push(View::Status { scroll: 0 });
                }
                return Effect::None;
            }
            KeyCode::Esc => {
                self.back();
                return Effect::None;
            }
            KeyCode::Char('/') if !typing => {
                self.push(View::Search {
                    query: String::new(),
                    hits: Vec::new(),
                    list: ListState::default(),
                });
                return Effect::None;
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl-L: the main loop redraws every frame anyway; refresh data.
                self.refresh_fast();
                return Effect::None;
            }
            // Ctrl-C must never drop the user out of the interface.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Effect::None;
            }
            _ => {}
        }

        let view = self.stack.pop().expect("view stack is never empty");
        let (view, effect) = self.handle_view_key(view, key);
        if let Some(v) = view {
            self.stack.push(v);
        }
        if self.stack.is_empty() {
            self.stack.push(View::Home {
                list: ListState::default().with_selected(Some(0)),
            });
        }
        effect
    }

    /// Handle a key for the top view. The view has been popped from the
    /// stack: returning `Some(view)` keeps it on top, `None` means it was
    /// either closed or already pushed back underneath a newly opened view.
    fn handle_view_key(&mut self, mut view: View, key: KeyEvent) -> (Option<View>, Effect) {
        let mut next: Option<View> = None;
        let mut effect = Effect::None;
        match &mut view {
            View::Home { list } => {
                let cats = self.home_categories();
                if let Some(i) = digit(key) {
                    if i < cats.len() {
                        list.select(Some(i));
                        next = Some(category_view(cats[i]));
                    }
                } else if is_activate(key) {
                    if let Some(c) = list.selected().and_then(|i| cats.get(i)) {
                        next = Some(category_view(*c));
                    }
                } else if key.code == KeyCode::Char('q') {
                    self.confirm(Pending::Logout);
                } else {
                    nav(list, cats.len(), key);
                }
            }
            View::Category { cat, list } => {
                let entries = self.entries(*cat);
                let chosen = if let Some(i) = digit(key) {
                    (i < entries.len()).then(|| {
                        list.select(Some(i));
                        entries[i]
                    })
                } else if is_activate(key) {
                    list.selected().and_then(|i| entries.get(i).copied())
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, entries.len(), key);
                    None
                };
                if let Some(e) = chosen {
                    let (v, eff) = self.activate(e);
                    next = v;
                    effect = eff;
                }
            }
            View::Handbook { pages, list, .. } => {
                if is_activate(key) {
                    if let Some(p) = list.selected().and_then(|i| pages.get(i)).cloned() {
                        next = Some(self.open_page(&p));
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, pages.len(), key);
                }
            }
            View::Pager { scroll, .. } | View::Status { scroll } | View::Help { scroll, .. } => {
                if is_back(key) {
                    return (None, Effect::None);
                }
                scroll_key(scroll, key);
            }
            View::Network { scroll } => match key.code {
                KeyCode::Char('p') => self.start_probe(),
                KeyCode::Char('r') => {
                    self.refresh_fast();
                    self.notify(Level::Info, "Network status refreshed.");
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => scroll_key(scroll, key),
            },
            View::Notes {
                notes: list_notes,
                list,
            } => match key.code {
                KeyCode::Char('n') => {
                    self.modal = Some(Modal::NewNote {
                        value: String::new(),
                        error: None,
                    })
                }
                KeyCode::Char('r') => {
                    *list_notes = notes::list(&notes::notes_dir(&self.home));
                    clamp_selection(list, list_notes.len());
                }
                _ if is_activate(key) => {
                    if let Some(n) = list.selected().and_then(|i| list_notes.get(i)) {
                        effect = self.edit_note(n.path.clone());
                    }
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => nav(list, list_notes.len(), key),
            },
            View::Settings { list } => {
                if is_activate(key) {
                    if let Some(t) = list.selected().and_then(|i| ThemeName::ALL.get(i)) {
                        self.set_theme(*t);
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, ThemeName::ALL.len(), key);
                }
            }
            View::Users { rows, error, list } => {
                if is_activate(key) {
                    if let Some(r) = list.selected().and_then(|i| rows.get(i)).cloned() {
                        next = self.choose_mode(r);
                    }
                } else if key.code == KeyCode::Char('r') {
                    let (r, e) = self.load_accounts();
                    *rows = r;
                    *error = e;
                    clamp_selection(list, rows.len());
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, rows.len(), key);
                }
            }
            View::ChooseMode { row, list } => {
                if is_activate(key) {
                    if let Some(m) = list.selected().and_then(|i| MODES.get(i)) {
                        self.confirm_set_mode(row, *m);
                    }
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, MODES.len(), key);
                }
            }
            View::Recovery { list } => {
                let chosen = if let Some(i) = digit(key) {
                    (i < RecoveryItem::ALL.len()).then(|| {
                        list.select(Some(i));
                        RecoveryItem::ALL[i]
                    })
                } else if is_activate(key) {
                    list.selected()
                        .and_then(|i| RecoveryItem::ALL.get(i).copied())
                } else if is_back(key) {
                    return (None, Effect::None);
                } else {
                    nav(list, RecoveryItem::ALL.len(), key);
                    None
                };
                if let Some(item) = chosen {
                    next = self.open_recovery(item);
                }
            }
            View::Media {
                volumes,
                error,
                list,
            } => {
                let selected = list.selected().and_then(|i| volumes.get(i)).cloned();
                match key.code {
                    KeyCode::Char('r') => {
                        (*volumes, *error) = self.load_volumes();
                        clamp_selection(list, volumes.len());
                    }
                    KeyCode::Char('e') => {
                        if let Some(v) = selected {
                            self.eject_volume(&v);
                            (*volumes, *error) = self.load_volumes();
                            clamp_selection(list, volumes.len());
                        }
                    }
                    _ if is_activate(key) => {
                        if let Some(v) = selected {
                            next = self.open_volume(v);
                            (*volumes, *error) = self.load_volumes();
                            clamp_selection(list, volumes.len());
                        }
                    }
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, volumes.len(), key),
                }
            }
            View::Backups {
                volume,
                mount,
                files,
                list,
            } => {
                let sel = list.selected().unwrap_or(0);
                match key.code {
                    KeyCode::Char('r') => {
                        *files = backup::list_backups(mount);
                        clamp_selection(list, files.len() + 1);
                    }
                    KeyCode::Char('c') => {
                        if let Some(f) = sel.checked_sub(1).and_then(|i| files.get(i)) {
                            effect = self.backup_tool_effect(CHECK_LABEL, "check", &f.path);
                        }
                    }
                    KeyCode::Char('e') => {
                        let v = volume.clone();
                        self.eject_volume(&v);
                        self.reload_volumes();
                        return (None, Effect::None);
                    }
                    _ if is_activate(key) => match sel.checked_sub(1) {
                        None => self.confirm_backup(volume, mount),
                        Some(i) => {
                            if let Some(f) = files.get(i) {
                                self.confirm_restore(f);
                            }
                        }
                    },
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, files.len() + 1, key),
                }
            }
            View::Search { query, hits, list } => match key.code {
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if query.chars().count() < 64 {
                        query.push(c);
                    }
                    *hits = self.search(query);
                    list.select((!hits.is_empty()).then_some(0));
                }
                KeyCode::Backspace => {
                    query.pop();
                    *hits = self.search(query);
                    list.select((!hits.is_empty()).then_some(0));
                }
                KeyCode::Enter => {
                    if let Some(h) = list.selected().and_then(|i| hits.get(i)).cloned() {
                        match h {
                            SearchHit::Category(c) => next = Some(category_view(c)),
                            SearchHit::Entry(e) => {
                                let (v, eff) = self.activate(e);
                                next = v;
                                effect = eff;
                            }
                            SearchHit::Page(p) => next = Some(self.open_page(&p)),
                        }
                    }
                }
                _ => nav(list, hits.len(), key),
            },
        }
        if let Some(n) = next {
            // Keep the current view below the newly opened one.
            self.stack.push(view);
            self.stack.push(n);
            return (None, effect);
        }
        (Some(view), effect)
    }

    /// Open an entry: returns a view to push and/or an effect to run.
    pub fn activate(&mut self, e: Entry) -> (Option<View>, Effect) {
        match e {
            Entry::App(i) => {
                let mode = self.mode();
                let app = &self.catalog.apps[i];
                match (app.availability(mode), app.command_for(mode)) {
                    (Availability::Ready(_), Some(argv)) => {
                        let req = RunRequest {
                            label: app.name.clone(),
                            argv: argv.to_vec(),
                            extra_args: Vec::new(),
                            env: Vec::new(),
                            polkit_agent: None,
                            pause_after: app.pause_after_exit,
                            banner: None,
                        };
                        (None, Effect::Run(req))
                    }
                    (Availability::Missing(what), _) => {
                        let body = format!(
                            "'{}' cannot start because its program is not available:\n\n  {what}\n\n\
                             The rest of the system is unaffected. If this persists, report it \
                             to your administrator.",
                            app.name
                        );
                        self.notify(Level::Error, format!("{} is not available", app.name));
                        self.modal = Some(Modal::Message {
                            title: "PROGRAM UNAVAILABLE".into(),
                            body,
                            error: true,
                        });
                        (None, Effect::None)
                    }
                    (Availability::Ready(_), None) => (None, Effect::None),
                }
            }
            Entry::Builtin(b) => self.activate_builtin(b),
        }
    }

    fn activate_builtin(&mut self, b: Builtin) -> (Option<View>, Effect) {
        let none = (None, Effect::None);
        if !self.mode().allows(b.min_mode()) {
            // Not reachable through the menu; defence in depth for search.
            self.notify(
                Level::Error,
                format!("{} requires {} mode", b.name(), b.min_mode()),
            );
            return none;
        }
        match b {
            Builtin::Handbook => {
                let (pages, error) = match handbook::list(&self.sys.handbook_dir) {
                    Ok(p) => (p, None),
                    Err(e) => (
                        Vec::new(),
                        Some(format!("{}: {e}", self.sys.handbook_dir.display())),
                    ),
                };
                let sel = (!pages.is_empty()).then_some(0);
                (
                    Some(View::Handbook {
                        pages,
                        list: ListState::default().with_selected(sel),
                        error,
                    }),
                    Effect::None,
                )
            }
            Builtin::Notes => {
                let n = notes::list(&notes::notes_dir(&self.home));
                let sel = (!n.is_empty()).then_some(0);
                (
                    Some(View::Notes {
                        notes: n,
                        list: ListState::default().with_selected(sel),
                    }),
                    Effect::None,
                )
            }
            Builtin::Network => {
                self.refresh_fast();
                (Some(View::Network { scroll: 0 }), Effect::None)
            }
            Builtin::SystemStatus => {
                self.refresh_fast();
                self.refresh_slow();
                (Some(View::Status { scroll: 0 }), Effect::None)
            }
            Builtin::Notifications => {
                let text = if self.notices.is_empty() {
                    "No notifications in this session.".to_string()
                } else {
                    self.notices
                        .iter()
                        .rev()
                        .map(|n| {
                            format!(
                                "{} {} {}",
                                crate::ui::clock(n.at),
                                level_tag(n.level),
                                n.text
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                (
                    Some(View::Pager {
                        title: "NOTIFICATION HISTORY".into(),
                        text,
                        scroll: 0,
                    }),
                    Effect::None,
                )
            }
            Builtin::Settings => {
                let i = ThemeName::ALL
                    .iter()
                    .position(|t| *t == self.settings.theme);
                (
                    Some(View::Settings {
                        list: ListState::default().with_selected(i),
                    }),
                    Effect::None,
                )
            }
            Builtin::Clock => {
                self.open_set_time();
                none
            }
            Builtin::Users => {
                let (rows, error) = self.load_accounts();
                let sel = (!rows.is_empty()).then_some(0);
                (
                    Some(View::Users {
                        rows,
                        error,
                        list: ListState::default().with_selected(sel),
                    }),
                    Effect::None,
                )
            }
            Builtin::Recovery => (
                Some(View::Recovery {
                    list: ListState::default().with_selected(Some(0)),
                }),
                Effect::None,
            ),
            Builtin::Backup => {
                let (volumes, error) = self.load_volumes();
                let sel = (!volumes.is_empty()).then_some(0);
                (
                    Some(View::Media {
                        volumes,
                        error,
                        list: ListState::default().with_selected(sel),
                    }),
                    Effect::None,
                )
            }
            Builtin::Help => (
                Some(View::Help {
                    topic: HelpTopic::Home,
                    scroll: 0,
                }),
                Effect::None,
            ),
            Builtin::Shell => {
                let req = RunRequest {
                    label: "Terminal shell".into(),
                    argv: self.sys.shell.clone(),
                    extra_args: Vec::new(),
                    env: Vec::new(),
                    polkit_agent: None,
                    pause_after: false,
                    banner: Some(format!(
                        "RESILIENCE SHELL — {} mode, unprivileged. Type 'exit' to return to the interface.",
                        self.mode()
                    )),
                };
                (None, Effect::Run(req))
            }
            Builtin::Logout => {
                self.confirm(Pending::Logout);
                none
            }
            Builtin::Reboot => {
                self.confirm(Pending::Reboot);
                none
            }
            Builtin::PowerOff => {
                self.confirm(Pending::PowerOff);
                none
            }
        }
    }

    fn open_page(&mut self, p: &handbook::Page) -> View {
        match handbook::read(&p.path) {
            Ok(text) => View::Pager {
                title: p.title.to_uppercase(),
                text,
                scroll: 0,
            },
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("cannot open {}: {e}", p.path.display()),
                );
                View::Pager {
                    title: "ERROR".into(),
                    text: format!("This page could not be read:\n\n{e}"),
                    scroll: 0,
                }
            }
        }
    }

    fn edit_note(&self, path: PathBuf) -> Effect {
        let argv = if self.mode() == Mode::UltraStable {
            &self.sys.restricted_editor
        } else {
            &self.sys.editor
        };
        Effect::Run(RunRequest {
            label: "Notes editor".into(),
            argv: argv.clone(),
            extra_args: vec![path],
            env: Vec::new(),
            polkit_agent: None,
            pause_after: false,
            banner: None,
        })
    }

    fn set_theme(&mut self, t: ThemeName) {
        self.settings.theme = t;
        self.theme = Theme::new(t);
        match self.settings.save(&self.settings_path) {
            Ok(()) => self.notify(Level::Info, format!("Theme set to {}.", t.label())),
            Err(e) => self.notify(Level::Error, format!("Theme applied but not saved: {e}")),
        }
    }

    fn confirm(&mut self, action: Pending) {
        let (title, body, confirm) = match &action {
            Pending::SetMode { .. }
            | Pending::SetTime { .. }
            | Pending::Backup { .. }
            | Pending::Restore { .. } => return,
            Pending::Logout => (
                "LOG OUT",
                "End this session? Programs you started from here will be closed.",
                "Log out",
            ),
            Pending::Reboot => (
                "RESTART",
                "Restart the computer now? Unsaved work in open programs will be lost.",
                "Restart",
            ),
            Pending::PowerOff => (
                "POWER OFF",
                "Shut the computer down now? Unsaved work in open programs will be lost.",
                "Power off",
            ),
        };
        self.modal = Some(Modal::Confirm {
            title: title.into(),
            body: body.into(),
            confirm: confirm.into(),
            action,
            yes_focused: false, // the safe choice is the default
        });
    }

    fn handle_modal(&mut self, modal: Modal, key: KeyEvent) -> Effect {
        match modal {
            Modal::Message { .. } => {
                if !matches!(key.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ')) {
                    self.modal = Some(modal);
                }
                Effect::None
            }
            Modal::Confirm {
                title,
                body,
                confirm,
                action,
                mut yes_focused,
            } => {
                let decided = match key.code {
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                        yes_focused = !yes_focused;
                        None
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
                    KeyCode::Enter => Some(yes_focused),
                    _ => None,
                };
                match decided {
                    None => {
                        self.modal = Some(Modal::Confirm {
                            title,
                            body,
                            confirm,
                            action,
                            yes_focused,
                        });
                        Effect::None
                    }
                    Some(false) => Effect::None,
                    Some(true) => self.execute(action),
                }
            }
            Modal::NewNote { mut value, error } => match key.code {
                KeyCode::Esc => Effect::None,
                KeyCode::Enter => match notes::create(&notes::notes_dir(&self.home), &value) {
                    Ok(path) => {
                        self.notify(Level::Info, format!("Created note {}", path.display()));
                        self.reload_notes();
                        self.edit_note(path)
                    }
                    Err(e) => {
                        self.modal = Some(Modal::NewNote {
                            value,
                            error: Some(e.to_string()),
                        });
                        Effect::None
                    }
                },
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::NewNote { value, error: None });
                    Effect::None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if value.chars().count() < 64 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::NewNote { value, error: None });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::NewNote { value, error });
                    Effect::None
                }
            },
            Modal::SetTime {
                mut value,
                zone,
                error,
            } => match key.code {
                KeyCode::Esc => Effect::None,
                KeyCode::Enter => {
                    self.submit_set_time(value, zone);
                    Effect::None
                }
                KeyCode::Backspace => {
                    value.pop();
                    self.modal = Some(Modal::SetTime {
                        value,
                        zone,
                        error: None,
                    });
                    Effect::None
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && (c.is_ascii_digit() || "-: ".contains(c)) =>
                {
                    if value.len() < 19 {
                        value.push(c);
                    }
                    self.modal = Some(Modal::SetTime {
                        value,
                        zone,
                        error: None,
                    });
                    Effect::None
                }
                _ => {
                    self.modal = Some(Modal::SetTime { value, zone, error });
                    Effect::None
                }
            },
        }
    }

    fn execute(&mut self, action: Pending) -> Effect {
        let verb = match action {
            Pending::Logout => return Effect::Quit,
            Pending::SetMode { user, to } => return self.set_mode_effect(&user, to),
            Pending::SetTime { utc } => return self.set_time_effect(utc),
            Pending::Backup { mount } => {
                return self.backup_tool_effect(BACKUP_LABEL, "create", &mount);
            }
            Pending::Restore { file } => {
                return self.backup_tool_effect(RESTORE_LABEL, "restore", &file);
            }
            Pending::Reboot => "reboot",
            Pending::PowerOff => "poweroff",
        };
        tracing::warn!(verb, "power action requested");
        // --no-ask-password: never let polkit start a password agent on top
        // of the TUI. Authorisation for local sessions is decided by logind.
        match run_full(
            &["systemctl", "--no-ask-password", verb],
            Duration::from_secs(10),
        ) {
            Some(o) if o.success() => {
                self.notify(Level::Info, format!("{verb} in progress..."));
            }
            Some(o) => {
                let msg = o.stderr.trim().to_string();
                self.notify(Level::Error, format!("{verb} refused: {msg}"));
                self.modal = Some(Modal::Message {
                    title: "ACTION REFUSED".into(),
                    body: format!("The system refused to {verb}:\n\n{msg}"),
                    error: true,
                });
            }
            None => {
                self.notify(Level::Error, format!("{verb}: systemctl did not answer"));
                self.modal = Some(Modal::Message {
                    title: "ACTION FAILED".into(),
                    body: "systemctl could not be run or did not answer in time.".into(),
                    error: true,
                });
            }
        }
        Effect::None
    }

    fn reload_notes(&mut self) {
        let dir = notes::notes_dir(&self.home);
        for v in self.stack.iter_mut() {
            if let View::Notes { notes: n, list } = v {
                *n = notes::list(&dir);
                clamp_selection(list, n.len());
            }
        }
    }

    /// Called by the main loop after a program started from the TUI exits.
    pub fn after_run(&mut self, label: &str, result: &std::io::Result<std::process::ExitStatus>) {
        match result {
            Ok(st) if st.success() => {}
            Ok(st) => self.notify(Level::Warn, format!("{label} exited with {st}")),
            Err(e) => self.notify(Level::Error, format!("{label} could not start: {e}")),
        }
        if label == SET_MODE_LABEL {
            // Back to the account list, which now shows the result.
            while matches!(self.view(), View::ChooseMode { .. }) {
                self.stack.pop();
            }
            self.reload_accounts();
        }
        if [BACKUP_LABEL, CHECK_LABEL, RESTORE_LABEL].contains(&label) {
            self.reload_backups();
        }
        self.reload_notes();
        self.refresh_fast();
    }

    // ----- users and security modes ----------------------------------------

    /// Managed accounts, from the root-owned helper configuration, the
    /// recorded mode changes and the live group database. Read-only.
    pub fn load_accounts(&self) -> (Vec<AccountRow>, Option<String>) {
        let cfg = match std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| format!("{}: {e}", self.paths.admin_config.display()))
            .and_then(|t| AdminConfig::parse(&t))
        {
            Ok(c) => c,
            Err(e) => return (Vec::new(), Some(e)),
        };
        let state = match std::fs::read_to_string(&cfg.state_file) {
            Ok(t) => match ModeState::parse(&t) {
                Ok(s) => s,
                Err(e) => {
                    return (
                        Vec::new(),
                        Some(format!("{} is damaged: {e}", cfg.state_file.display())),
                    );
                }
            },
            Err(_) => ModeState::default(),
        };
        let groups = std::fs::read_to_string(&self.paths.group_file).unwrap_or_default();
        (admin::overview(&cfg, &state, &groups), None)
    }

    fn reload_accounts(&mut self) {
        let (rows, error) = self.load_accounts();
        for v in self.stack.iter_mut() {
            if let View::Users {
                rows: r,
                error: e,
                list,
            } = v
            {
                *r = rows.clone();
                *e = error.clone();
                clamp_selection(list, r.len());
            }
        }
    }

    fn choose_mode(&mut self, row: AccountRow) -> Option<View> {
        if !self.has_administrator("Changing a security mode") {
            return None;
        }
        let i = MODES.iter().position(|m| *m == row.effective);
        Some(View::ChooseMode {
            row,
            list: ListState::default().with_selected(i),
        })
    }

    /// False (with an explanation shown) when no account could type an
    /// administrator password, so an action is not offered at all.
    fn has_administrator(&mut self, what: &str) -> bool {
        let (rows, _) = self.load_accounts();
        if admin::administrators(&rows) > 0 {
            return true;
        }
        self.modal = Some(Modal::Message {
            title: "NO ADMINISTRATOR".into(),
            body: format!(
                "{what} needs the password of an Advanced (administrator) account, \
                 and this system has none.\n\n\
                 On the live system this is intentional. On an installed system, \
                 see the handbook page 'Security modes'."
            ),
            error: true,
        });
        false
    }

    /// Explain the consequences before anything happens. The checks here
    /// are for clear messages only; the helper repeats all of them.
    fn confirm_set_mode(&mut self, row: &AccountRow, to: Mode) {
        let cfg = std::fs::read_to_string(&self.paths.admin_config)
            .ok()
            .and_then(|t| AdminConfig::parse(&t).ok());
        let state = cfg
            .as_ref()
            .and_then(|c| std::fs::read_to_string(&c.state_file).ok())
            .and_then(|t| ModeState::parse(&t).ok())
            .unwrap_or_default();
        if let Some(c) = &cfg
            && let Err(refusal) = admin::plan_set_mode(c, &state, &row.name, to)
        {
            self.modal = Some(Modal::Message {
                title: "NOT POSSIBLE".into(),
                body: format!("{}.", capitalise(&refusal.to_string())),
                error: true,
            });
            return;
        }
        let user = &row.name;
        let is_self = *user == self.who.user;
        let mut body = format!("Change '{user}' from {} to {to} mode?\n\n", row.effective);
        body.push_str(match to {
            Mode::Advanced => {
                "The account will be able to change the whole system with sudo and its \
                 password. Mistakes in this mode can damage the installation."
            }
            Mode::Stable => {
                "The account gets an unprivileged shell and can use the Nix package \
                 tools, but cannot change the system."
            }
            Mode::UltraStable => {
                "The account will only have the guided interface: no shell, no \
                 administration, no software installation."
            }
        });
        body.push_str("\n\n");
        if to < row.effective {
            body.push_str(&format!(
                "Rights are removed immediately: all sessions of '{user}' will be closed{}.",
                if is_self { ", including this one" } else { "" }
            ));
        } else {
            body.push_str(&format!("New rights apply at the next login of '{user}'."));
        }
        body.push_str(
            "\n\nAn administrator must type their password next. The change is \
             recorded in the audit log and kept after restarts.",
        );
        self.modal = Some(Modal::Confirm {
            title: "CHANGE SECURITY MODE".into(),
            body,
            confirm: "Continue".into(),
            action: Pending::SetMode {
                user: user.clone(),
                to,
            },
            yes_focused: false,
        });
    }

    /// The fixed argv: `pkexec <helper> set-mode <user> <mode>`. The user
    /// name comes from the root-owned configuration and is validated again.
    fn set_mode_effect(&mut self, user: &str, to: Mode) -> Effect {
        let cfg = match std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))
        {
            Ok(c) => c,
            Err(e) => {
                self.notify(Level::Error, format!("mode change unavailable: {e}"));
                return Effect::None;
            }
        };
        if !admin::valid_user_name(user) || !cfg.users.contains_key(user) {
            return Effect::None;
        }
        tracing::warn!(
            user,
            mode = admin::mode_id(to),
            "security mode change requested"
        );
        Effect::Run(RunRequest {
            label: SET_MODE_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "set-mode".into(),
                user.into(),
                admin::mode_id(to).into(),
            ],
            extra_args: Vec::new(),
            // pkexec refuses to run unless $SHELL is listed in /etc/shells;
            // Ultra-Stable sessions deliberately set it to nologin.
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            pause_after: true,
            banner: Some(format!(
                "CHANGE SECURITY MODE: '{user}' -> {to}. An administrator must now type \
                 their password. Press Ctrl-C to cancel."
            )),
        })
    }

    // ----- date and time ------------------------------------------------------

    fn open_set_time(&mut self) {
        if !self.has_administrator("Setting the clock") {
            return;
        }
        self.modal = Some(Modal::SetTime {
            value: crate::ui::date_time(SystemTime::now()),
            zone: clock::system_zone(),
            error: None,
        });
    }

    /// Validate the typed local time; on success ask for confirmation.
    /// The helper checks format and bounds again on the UTC value.
    fn submit_set_time(&mut self, value: String, zone: String) {
        let parsed = clock::parse_local(&value).and_then(|c| {
            clock::local_to_unix(c)
                .ok_or(clock::TimeRefusal::NoSuchDate)
                .and_then(clock::check_bounds)
        });
        let utc = match parsed {
            Ok(t) => t,
            Err(e) => {
                self.modal = Some(Modal::SetTime {
                    value,
                    zone,
                    error: Some(capitalise(&e.to_string())),
                });
                return;
            }
        };
        let now = crate::ui::date_time(SystemTime::now());
        let new = crate::ui::date_time(SystemTime::UNIX_EPOCH + Duration::from_secs(utc as u64));
        let body = format!(
            "Set the clock to {new} ({zone})?\n\n\
             The clock now shows {now}. In UTC the new time is {}.\n\n\
             An administrator must type their password next. The change is \
             recorded in the audit log, and the hardware clock keeps it after \
             a restart.",
            admin::rfc3339_utc(utc)
        );
        self.modal = Some(Modal::Confirm {
            title: "SET THE CLOCK".into(),
            body,
            confirm: "Continue".into(),
            action: Pending::SetTime { utc },
            yes_focused: false,
        });
    }

    /// The fixed argv: `pkexec <helper> set-time <YYYY-MM-DDTHH:MM:SSZ>`.
    fn set_time_effect(&mut self, utc: i64) -> Effect {
        let cfg = match std::fs::read_to_string(&self.paths.admin_config)
            .map_err(|e| e.to_string())
            .and_then(|t| AdminConfig::parse(&t))
        {
            Ok(c) => c,
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("setting the clock is unavailable: {e}"),
                );
                return Effect::None;
            }
        };
        let Ok(utc) = clock::check_bounds(utc) else {
            return Effect::None;
        };
        let at = admin::rfc3339_utc(utc);
        tracing::warn!(to = %at, "clock change requested");
        Effect::Run(RunRequest {
            label: SET_TIME_LABEL.into(),
            argv: vec![
                cfg.pkexec.to_string_lossy().into_owned(),
                cfg.helper.to_string_lossy().into_owned(),
                "set-time".into(),
                at.clone(),
            ],
            extra_args: Vec::new(),
            env: vec![(
                "SHELL".into(),
                cfg.session_shell.to_string_lossy().into_owned(),
            )],
            polkit_agent: Some(cfg.pkttyagent.to_string_lossy().into_owned()),
            pause_after: true,
            banner: Some(format!(
                "SET THE CLOCK to {at}. An administrator must now type their \
                 password. Press Ctrl-C to cancel."
            )),
        })
    }

    // ----- removable media and backups ---------------------------------------

    /// Removable volumes, through lsblk. Internal disks are never listed.
    pub fn load_volumes(&self) -> (Vec<Volume>, Option<String>) {
        match media::list(&self.sys.tools.lsblk) {
            Ok(v) => (v, None),
            Err(e) => (Vec::new(), Some(e)),
        }
    }

    fn reload_volumes(&mut self) {
        let (v, e) = self.load_volumes();
        for view in self.stack.iter_mut() {
            if let View::Media {
                volumes,
                error,
                list,
            } = view
            {
                *volumes = v.clone();
                *error = e.clone();
                clamp_selection(list, volumes.len());
            }
        }
    }

    fn reload_backups(&mut self) {
        for view in self.stack.iter_mut() {
            if let View::Backups {
                mount, files, list, ..
            } = view
            {
                *files = backup::list_backups(mount);
                clamp_selection(list, files.len() + 1);
            }
        }
    }

    /// Mount (if needed) and show the backups on a volume.
    fn open_volume(&mut self, v: Volume) -> Option<View> {
        if let Some(p) = &v.problem {
            self.modal = Some(Modal::Message {
                title: "CANNOT USE THIS DEVICE".into(),
                body: format!("'{}' cannot be used here: {p}.", v.display_name()),
                error: true,
            });
            return None;
        }
        match media::mount(&self.sys.tools.lsblk, &self.sys.tools.udisksctl, &v) {
            Ok(mount) => {
                if v.mountpoint.is_none() {
                    self.notify(
                        Level::Info,
                        format!("{} is ready ({})", v.display_name(), mount.display()),
                    );
                }
                let files = backup::list_backups(&mount);
                Some(View::Backups {
                    volume: Volume {
                        mountpoint: Some(mount.clone()),
                        ..v
                    },
                    mount,
                    files,
                    list: ListState::default().with_selected(Some(0)),
                })
            }
            Err(e) => {
                self.notify(
                    Level::Error,
                    format!("cannot open {}: {e}", v.display_name()),
                );
                self.modal = Some(Modal::Message {
                    title: "CANNOT OPEN THE STICK".into(),
                    body: format!("'{}' could not be opened:\n\n{e}", v.display_name()),
                    error: true,
                });
                None
            }
        }
    }

    fn eject_volume(&mut self, v: &Volume) {
        let name = v.display_name();
        match media::eject(&self.sys.tools.udisksctl, v) {
            Ok(how) => {
                let extra = match how {
                    Ejected::PoweredOff => String::new(),
                    Ejected::Unmounted(why) => {
                        format!("\n\n(It could not be switched off: {why}.)")
                    }
                };
                self.notify(Level::Info, format!("{name} can be unplugged."));
                self.modal = Some(Modal::Message {
                    title: "SAFE TO UNPLUG".into(),
                    body: format!(
                        "'{name}' can be unplugged now: everything has been written to it.{extra}"
                    ),
                    error: false,
                });
            }
            Err(e) => {
                self.notify(Level::Error, format!("cannot eject {name}: {e}"));
                self.modal = Some(Modal::Message {
                    title: "CANNOT EJECT".into(),
                    body: format!("'{name}' could not be ejected: {e}\n\nDo not unplug it yet."),
                    error: true,
                });
            }
        }
    }

    fn confirm_backup(&mut self, volume: &Volume, mount: &std::path::Path) {
        if volume.read_only {
            self.modal = Some(Modal::Message {
                title: "READ-ONLY STICK".into(),
                body: format!(
                    "'{}' is write-protected, so no backup can be saved on it. Backups on it \
                     can still be checked and restored.",
                    volume.display_name()
                ),
                error: true,
            });
            return;
        }
        let estimate = backup::estimate_size(&self.home);
        let free = status::storage(mount).map(|s| s.available);
        let mut body = format!(
            "Save your home folder (about {}) to '{}'{}?\n\n\
             The backup is encrypted with a passphrase you choose next. Without it nobody \
             can open the backup, not even you. Your files are not changed.",
            human_bytes(estimate),
            volume.display_name(),
            free.map(|f| format!(" ({} free)", human_bytes(f)))
                .unwrap_or_default()
        );
        if free.is_some_and(|f| f < estimate) {
            body.push_str(
                "\n\n[!!] The stick may be too small. Compression often helps; if it is \
                 still too small, the backup stops and nothing is kept.",
            );
        }
        self.modal = Some(Modal::Confirm {
            title: "NEW BACKUP".into(),
            body,
            confirm: "Back up".into(),
            action: Pending::Backup {
                mount: mount.to_path_buf(),
            },
            yes_focused: false,
        });
    }

    fn confirm_restore(&mut self, f: &BackupFile) {
        let mut body = format!(
            "Restore the backup of {}?\n\n\
             Its files go into a new folder, {}/backup-{}. Nothing that exists is \
             changed. You will need this backup's passphrase.",
            backup::human_stamp(&f.stamp),
            self.home.join(backup::RESTORED_DIR).display(),
            f.stamp
        );
        if !f.has_checksum {
            body.push_str(
                "\n\nThis copy has no checksum file; damage will still be detected while restoring.",
            );
        }
        self.modal = Some(Modal::Confirm {
            title: "RESTORE A BACKUP".into(),
            body,
            confirm: "Restore".into(),
            action: Pending::Restore {
                file: f.path.clone(),
            },
            yes_focused: false,
        });
    }

    /// Run `resilience-backup <op> <path>` in the foreground: age asks for
    /// the passphrase on the terminal itself.
    fn backup_tool_effect(&self, label: &str, op: &str, path: &std::path::Path) -> Effect {
        Effect::Run(RunRequest {
            label: label.into(),
            argv: vec![self.sys.tools.backup.clone(), op.into()],
            extra_args: vec![path.to_path_buf()],
            env: Vec::new(),
            polkit_agent: None,
            pause_after: true,
            banner: Some(format!("{}: press Ctrl-C to cancel.", label.to_uppercase())),
        })
    }

    // ----- recovery ----------------------------------------------------------

    fn open_recovery(&mut self, item: RecoveryItem) -> Option<View> {
        match item {
            RecoveryItem::Health => {
                self.refresh_slow();
                Some(View::Pager {
                    title: "SYSTEM HEALTH & RECENT FAILURES".into(),
                    text: self.health_text(),
                    scroll: 0,
                })
            }
            RecoveryItem::Generations => Some(View::Pager {
                title: "SYSTEM GENERATIONS".into(),
                text: self.generations_text(),
                scroll: 0,
            }),
            RecoveryItem::Report => {
                self.export_report();
                None
            }
            RecoveryItem::Emergency => {
                let pages = handbook::list(&self.sys.handbook_dir).unwrap_or_default();
                match pages.iter().find(|p| {
                    p.path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().contains("recovery"))
                }) {
                    Some(p) => Some(self.open_page(&p.clone())),
                    None => Some(View::Pager {
                        title: "EMERGENCY GUIDE".into(),
                        text: EMERGENCY_FALLBACK.into(),
                        scroll: 0,
                    }),
                }
            }
        }
    }

    pub fn health_text(&self) -> String {
        let mut t = String::new();
        let state = match &self.sys_state {
            SystemState::Running => "running: everything started correctly".to_string(),
            SystemState::Degraded => "DEGRADED: some services failed (listed below)".to_string(),
            other => format!("{other:?}"),
        };
        t.push_str(&format!("Overall state: {state}\n\n"));
        if self.failed_units.is_empty() {
            t.push_str("Failed services: none\n");
        } else {
            t.push_str(&format!("Failed services ({}):\n", self.failed_units.len()));
            for f in recovery::failure_details(&self.failed_units) {
                t.push_str(&format!(
                    "  {}\n    {}\n    result: {}   since: {}\n",
                    f.unit, f.description, f.result, f.since
                ));
            }
            t.push_str(
                "\nA failed service is often harmless for daily use. If something you need \
                 does not work, restart the computer; if it still fails, start the previous \
                 system generation from the boot menu (see 'System generations').\n",
            );
        }
        let errors = recovery::recent_errors(30);
        t.push_str("\nRecent error messages (this boot):\n");
        if errors.is_empty() {
            t.push_str("  none that this account may read\n");
        } else {
            for e in &errors {
                t.push_str(&format!("  {e}\n"));
            }
        }
        if self.mode() != Mode::Advanced {
            t.push_str(
                "\nOnly administrators can read every system message. This list shows the \
                 messages your account is allowed to see.\n",
            );
        }
        t
    }

    pub fn generations_text(&self) -> String {
        let gens = recovery::generations(
            &self.paths.profiles,
            &self.paths.booted_system,
            &self.paths.running_system,
        );
        let mut t = String::from(
            "Every change to the system creates a new generation; earlier ones are kept \
             and can be started from the boot menu. Starting an earlier generation changes \
             only the system software and its settings, never your personal files.\n\n",
        );
        if gens.is_empty() {
            t.push_str(
                "No generations are recorded on this machine. This is normal on the live \
                 system, which always starts the same way.\n",
            );
        } else {
            t.push_str(&format!(
                "{:>4}  {:<16}  {:<24}  {}\n",
                "#", "CREATED", "VERSION", "STATE"
            ));
            for g in &gens {
                let mut marks = Vec::new();
                if g.booted {
                    marks.push("started now");
                }
                if g.running && !g.booted {
                    marks.push("active");
                }
                if g.default {
                    marks.push("default at next boot");
                }
                t.push_str(&format!(
                    "{:>4}  {:<16}  {:<24}  {}\n",
                    g.number,
                    g.created
                        .map(crate::ui::date_time)
                        .unwrap_or_else(|| "?".into()),
                    g.nixos_version.as_deref().unwrap_or("?"),
                    marks.join(", ")
                ));
                if let Some(k) = &g.kernel {
                    t.push_str(&format!("{:>4}  kernel {k}\n", ""));
                }
            }
        }
        t.push_str(
            "\nTo start an earlier generation:\n\
             \x20 1. Restart the computer (System > Restart computer).\n\
             \x20 2. When the boot menu appears, use the arrow keys to choose an older entry\n\
             \x20    (on some menus it is under 'NixOS - All configurations').\n\
             \x20 3. Press Enter. Only this start uses the older generation: the next\n\
             \x20    restart returns to the newest one, unless an administrator makes\n\
             \x20    the older generation permanent.\n",
        );
        t
    }

    pub fn diagnostic_report(&self) -> String {
        let now = admin::now_rfc3339();
        let mut t = format!(
            "RESILIENCE OS DIAGNOSTIC REPORT\n\
             Created: {now} (UTC)\n\
             Contains: system status, failed services, generations, network link state\n\
             (no addresses), and the last lines of the interface log. No passwords, no\n\
             file names from your home directory.\n\n"
        );
        let s = &self.snapshot;
        t.push_str(&format!(
            "Resilience: {} (edition {})\nOS: {}\nKernel: {}\nGeneration: {}\nUser: {} ({} mode)\nUptime: {}\n",
            resilience_core::VERSION,
            self.sys.edition,
            s.os_name,
            s.kernel,
            s.system_generation.as_deref().unwrap_or("?"),
            self.who.user,
            self.mode(),
            s.uptime.map(status::format_uptime).unwrap_or_else(|| "?".into()),
        ));
        if let Some(m) = &s.memory {
            t.push_str(&format!(
                "Memory: {} available of {}\n",
                resilience_core::human_bytes(m.available_kib * 1024),
                resilience_core::human_bytes(m.total_kib * 1024)
            ));
        }
        for (label, d) in [("system", &s.root), ("home", &s.home)] {
            if let Some(d) = d {
                t.push_str(&format!(
                    "Storage ({label}): {} free of {}\n",
                    resilience_core::human_bytes(d.available),
                    resilience_core::human_bytes(d.total)
                ));
            }
        }
        t.push_str(&format!("Network: {:?}\n", self.net.level));
        for i in &self.net.interfaces {
            t.push_str(&format!(
                "  {} {:?} {}\n",
                i.name,
                i.kind,
                if i.is_connected() {
                    "link up"
                } else {
                    "no link"
                }
            ));
        }
        t.push_str("\n--- Health ---\n");
        t.push_str(&self.health_text());
        t.push_str("\n--- Generations ---\n");
        t.push_str(&self.generations_text());
        t.push_str("\n--- Interface log (last 50 lines) ---\n");
        let log = resilience_core::config::state_home(&self.home)
            .join("resilience")
            .join("tui.log");
        for l in recovery::tail(&log, 50) {
            t.push_str(&l);
            t.push('\n');
        }
        t
    }

    fn export_report(&mut self) {
        let name = recovery::report_file_name(&admin::now_rfc3339());
        let dir = recovery::reports_dir(&self.home);
        match recovery::write_report(&dir, &name, &self.diagnostic_report()) {
            Ok(path) => {
                self.notify(Level::Info, format!("Report saved: {}", path.display()));
                self.modal = Some(Modal::Message {
                    title: "REPORT SAVED".into(),
                    body: format!(
                        "The diagnostic report was saved as:\n\n  {}\n\n\
                         It is a plain text file. Copy it to a USB stick or show it to \
                         someone who can help. It contains no passwords.",
                        path.display()
                    ),
                    error: false,
                });
            }
            Err(e) => {
                self.notify(Level::Error, format!("report not saved: {e}"));
                self.modal = Some(Modal::Message {
                    title: "REPORT NOT SAVED".into(),
                    body: format!(
                        "The report could not be written to {}:\n\n{e}",
                        dir.display()
                    ),
                    error: true,
                });
            }
        }
    }
}

/// The modes offered by the mode chooser, least privileged first.
pub const MODES: [Mode; 3] = [Mode::UltraStable, Mode::Stable, Mode::Advanced];

const EMERGENCY_FALLBACK: &str = "The handbook is not available.\n\n\
1. Restart the computer.\n\
2. In the boot menu, choose an earlier entry and press Enter.\n\
3. If the interface does not start, press Ctrl-Alt-F2 and log in with an\n\
   administrator account.\n\
4. Your files are in your home folder; they are not changed by these steps.";

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

pub fn level_tag(l: Level) -> &'static str {
    match l {
        Level::Info => "[--]",
        Level::Warn => "[!!]",
        Level::Error => "[XX]",
    }
}

fn category_view(cat: Category) -> View {
    View::Category {
        cat,
        list: ListState::default().with_selected(Some(0)),
    }
}

fn digit(key: KeyEvent) -> Option<usize> {
    match key.code {
        KeyCode::Char(c @ '1'..='9') => Some(c as usize - '1' as usize),
        _ => None,
    }
}

fn is_activate(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
    ) && !key.modifiers.contains(KeyModifiers::CONTROL)
}

fn is_back(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h')
    )
}

fn clamp_selection(list: &mut ListState, len: usize) {
    if len == 0 {
        list.select(None);
    } else {
        list.select(Some(list.selected().unwrap_or(0).min(len - 1)));
    }
}

/// Up/Down/Home/End/PgUp/PgDn (and j/k) list navigation with wrap-around.
fn nav(list: &mut ListState, len: usize, key: KeyEvent) {
    if len == 0 {
        list.select(None);
        return;
    }
    let cur = list.selected().unwrap_or(0).min(len - 1);
    let next = match key.code {
        KeyCode::Up | KeyCode::Char('k') => (cur + len - 1) % len,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => (cur + 1) % len,
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => len - 1,
        KeyCode::PageUp => cur.saturating_sub(10),
        KeyCode::PageDown => (cur + 10).min(len - 1),
        _ => cur,
    };
    list.select(Some(next));
}

/// Scroll keys for read-only text. Rendering clamps the upper bound.
fn scroll_key(scroll: &mut usize, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
        KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
        KeyCode::PageDown | KeyCode::Char(' ') => *scroll += 10,
        KeyCode::Home | KeyCode::Char('g') => *scroll = 0,
        KeyCode::End | KeyCode::Char('G') => *scroll = usize::MAX / 2,
        _ => {}
    }
}
