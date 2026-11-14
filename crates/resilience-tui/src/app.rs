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
use resilience_core::catalog::{Availability, Catalog, Category};
use resilience_core::config::{SystemConfig, ThemeName, UserSettings};
use resilience_core::handbook;
use resilience_core::mode::{Detected, Mode};
use resilience_core::network::{self, NetworkState, ProbeConfig, ProbeReport};
use resilience_core::notes;
use resilience_core::proc::run_full;
use resilience_core::status::{self, Snapshot, SystemState};

use crate::theme::Theme;

const FAST_REFRESH: Duration = Duration::from_secs(5);
const SLOW_REFRESH: Duration = Duration::from_secs(30);
const TOAST_DURATION: Duration = Duration::from_secs(6);
const MAX_NOTICES: usize = 100;

/// Functions built into the interface (as opposed to catalog programs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Handbook,
    Notes,
    Network,
    SystemStatus,
    Notifications,
    Settings,
    Help,
    Shell,
    Logout,
    Reboot,
    PowerOff,
}

impl Builtin {
    pub const ALL: [Builtin; 11] = [
        Builtin::Handbook,
        Builtin::Notes,
        Builtin::Network,
        Builtin::SystemStatus,
        Builtin::Notifications,
        Builtin::Settings,
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
    Help {
        topic: HelpTopic,
        scroll: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Logout,
    Reboot,
    PowerOff,
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
}

/// A program to run in the foreground with the terminal released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub label: String,
    pub argv: Vec<String>,
    pub extra_args: Vec<PathBuf>,
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
        let (title, body, confirm) = match action {
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
        }
    }

    fn execute(&mut self, action: Pending) -> Effect {
        let verb = match action {
            Pending::Logout => return Effect::Quit,
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
        self.reload_notes();
        self.refresh_fast();
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
