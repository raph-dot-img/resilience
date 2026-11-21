//! `resilience-gui` — the graphical interface of Resilience OS (ADR 0026).
//!
//! ```text
//! resilience-gui              the interface (inside cage, full screen)
//! resilience-gui run FILE     run one program request in this terminal
//!                             (started by the interface inside foot)
//! resilience-gui --version
//! ```
//!
//! The state machine is `resilience_shell::app::App`, the same one the
//! terminal interface drives. Home, categories and search are drawn
//! natively; every other screen is the terminal renderer's output shown as
//! a text panel until it gets its own graphical view.

mod run;
mod style;
mod textpanel;

use std::process::ExitCode;
use std::time::Duration;

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Fill, Length, Subscription, Task, keyboard, window};
use resilience_core::catalog::Category;
use resilience_shell::app::{App, Effect, Entry, SearchHit, Startup, View, category_description};
use resilience_shell::input::{KeyCode, KeyEvent, KeyModifiers};

use crate::style::Palette;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => match gui() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("resilience-gui: {e}");
                ExitCode::FAILURE
            }
        },
        Some("run") if args.len() == 2 => run::run_file(&args[1]),
        Some("-V" | "--version") => {
            println!("resilience-gui {}", resilience_core::VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("Usage: resilience-gui [--version | run FILE]");
            ExitCode::from(2)
        }
    }
}

fn gui() -> iced::Result {
    let mut a = iced::application(Gui::new, Gui::update, Gui::view)
        .title("Resilience")
        .subscription(Gui::subscription)
        .theme(Gui::theme)
        .window(window::Settings {
            size: iced::Size::new(1280.0, 800.0),
            decorations: false,
            ..window::Settings::default()
        });
    for f in style::load_fonts() {
        a = a.font(f);
    }
    a.default_font(style::UI_FONT).run()
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Key(keyboard::Event),
    Choose(usize),
    Back,
    Home,
    Search(String),
    Resized(iced::Size),
    RunDone(String, Result<i32, String>),
}

struct Gui {
    app: App,
    size: iced::Size,
    /// Label of the program running in its terminal window.
    running: Option<String>,
    /// The terminal renderer's output for screens without a native view.
    screen: textpanel::Screen,
}

impl Gui {
    fn new() -> (Gui, Task<Message>) {
        let mut app = App::new(Startup::load());
        app.apply_saved_layout();
        let gui = Gui {
            app,
            size: iced::Size::new(1280.0, 800.0),
            running: None,
            screen: textpanel::Screen::default(),
        };
        (gui, Task::none())
    }

    fn theme(&self) -> iced::Theme {
        style::theme(self.app.theme)
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick),
            keyboard::listen().map(Message::Key),
            window::resize_events().map(|(_, s)| Message::Resized(s)),
        ])
    }

    fn update(&mut self, m: Message) -> Task<Message> {
        let task = self.step(m);
        if self.wants_text_panel() {
            // Bars: ~72 px on top, ~40 px below, 28 px side padding.
            let (cols, rows) =
                textpanel::grid_for(self.size.width - 56.0, self.size.height - 120.0);
            self.screen = textpanel::render(&mut self.app, cols, rows);
        }
        task
    }

    /// Arrow keys on a grid of tiles: Left/Right move by one, Up/Down by a
    /// row. Returns false when the key is not for the grid.
    fn grid_move(&mut self, k: KeyEvent) -> bool {
        let n = match self.app.view() {
            View::Home { .. } => self.app.home_categories().len(),
            View::Category { cat, .. } => self.app.entries(*cat).len(),
            _ => return false,
        };
        if self.app.modal.is_some() || n == 0 || !k.modifiers.is_empty() {
            return false;
        }
        let cols = self.columns() as isize;
        let step = match k.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            KeyCode::Up => -cols,
            KeyCode::Down => cols,
            _ => return false,
        };
        if let Some(l) = self.app.list_mut() {
            let cur = l.selected().unwrap_or(0) as isize;
            let next = cur + step;
            if (0..n as isize).contains(&next) {
                l.select(Some(next as usize));
            }
        }
        true
    }

    fn wants_text_panel(&self) -> bool {
        self.running.is_none()
            && (self.app.modal.is_some()
                || !matches!(
                    self.app.view(),
                    View::Home { .. } | View::Category { .. } | View::Search { .. }
                ))
    }

    fn step(&mut self, m: Message) -> Task<Message> {
        let effect = match m {
            Message::Tick => {
                self.app.on_tick();
                Effect::None
            }
            Message::Resized(s) => {
                self.size = s;
                Effect::None
            }
            Message::Key(ev) => {
                if self.running.is_some() {
                    return Task::none();
                }
                match style::key_from_iced(&ev) {
                    Some(k) if self.grid_move(k) => Effect::None,
                    Some(k) => self.app.handle_key(k),
                    None => Effect::None,
                }
            }
            Message::Choose(i) => {
                if self.running.is_some() {
                    return Task::none();
                }
                self.app.choose_row(i)
            }
            Message::Back => self
                .app
                .handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Message::Home => {
                self.app.go_home();
                Effect::None
            }
            Message::Search(q) => {
                if q.is_empty() && matches!(self.app.view(), View::Search { .. }) {
                    self.app
                        .handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                } else {
                    self.app.set_search(&q);
                    Effect::None
                }
            }
            Message::RunDone(label, result) => {
                self.running = None;
                self.app.after_run(&label, &run::to_status(result));
                Effect::None
            }
        };
        match effect {
            Effect::None => Task::none(),
            Effect::Quit => iced::exit(),
            Effect::Run(req) => {
                let label = req.label.clone();
                self.running = Some(label.clone());
                Task::perform(run::in_terminal(req), move |r| {
                    Message::RunDone(label.clone(), r)
                })
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let p = Palette::of(self.app.theme);
        let body: Element<'_, Message> = if let Some(label) = &self.running {
            self.running_view(label)
        } else if self.wants_text_panel() {
            self.text_view(&p)
        } else {
            match self.app.view() {
                View::Home { .. } => self.home_view(&p),
                View::Category { cat, .. } => self.category_view(*cat, &p),
                View::Search { query, hits, .. } => self.search_view(query, hits, &p),
                _ => self.text_view(&p),
            }
        };
        let page = column![self.top_bar(&p), body, self.status_bar(&p)];
        container(page)
            .width(Fill)
            .height(Fill)
            .style(move |_| style::page(&p))
            .into()
    }

    fn top_bar(&self, p: &Palette) -> Element<'_, Message> {
        let at_home = self.app.stack.len() <= 1;
        let mut r = row![].spacing(12).align_y(iced::Center);
        if !at_home {
            r = r.push(
                button(text("\u{2190}  Back").size(16))
                    .on_press(Message::Back)
                    .padding([8, 16])
                    .style(style::ghost_button),
            );
            r = r.push(
                button(text("Home").size(16))
                    .on_press(Message::Home)
                    .padding([8, 16])
                    .style(style::ghost_button),
            );
        }
        r = r.push(text(self.title()).size(20).font(style::UI_FONT_BOLD));
        r = r.push(Space::new().width(Fill));
        let query = match self.app.view() {
            View::Search { query, .. } => query.clone(),
            _ => String::new(),
        };
        r = r.push(
            text_input("Search apps, guides, places\u{2026}", &query)
                .on_input(Message::Search)
                .padding([10, 16])
                .size(16)
                .width(Length::Fixed(380.0))
                .style(style::search_box),
        );
        let _ = p;
        container(r).padding([16, 28]).into()
    }

    fn title(&self) -> String {
        match self.app.view() {
            View::Home { .. } => format!("Hello, {}", self.app.who.user),
            View::Category { cat, .. } => section_name(*cat),
            View::Search { .. } => "Search".into(),
            _ => String::new(),
        }
    }

    fn home_view(&self, p: &Palette) -> Element<'_, Message> {
        let cats = self.app.home_categories();
        let tiles: Vec<Element<'_, Message>> = cats
            .iter()
            .enumerate()
            .map(|(i, c)| self.tile(i, &section_name(*c), category_description(*c), icon(*c), p))
            .collect();
        grid(tiles, self.columns())
    }

    fn category_view(&self, cat: Category, p: &Palette) -> Element<'_, Message> {
        let entries = self.app.entries(cat);
        if entries.is_empty() {
            return centered_note("Nothing is available here for your account.");
        }
        let tiles: Vec<Element<'_, Message>> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let name = self.app.entry_name(*e);
                let mut desc = self.entry_description(*e);
                if !self.app.entry_ready(*e) {
                    desc = format!("Not available on this computer. {desc}");
                }
                self.tile(i, &name, &desc, "", p)
            })
            .collect();
        grid(tiles, self.columns())
    }

    fn search_view(&self, query: &str, hits: &[SearchHit], p: &Palette) -> Element<'_, Message> {
        if hits.is_empty() {
            return centered_note(&format!("Nothing found for \u{201c}{query}\u{201d}."));
        }
        let sel = self.app.selected_row();
        let rows: Vec<Element<'_, Message>> = hits
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let (kind, name) = match h {
                    SearchHit::Category(c) => ("Section", section_name(*c)),
                    SearchHit::Entry(e) => ("App", self.app.entry_name(*e)),
                    SearchHit::Page(pg) => ("Guide", pg.title.clone()),
                };
                button(
                    row![
                        text(kind)
                            .size(13)
                            .width(Length::Fixed(80.0))
                            .style(style::dim_text),
                        text(name).size(17),
                    ]
                    .align_y(iced::Center),
                )
                .on_press(Message::Choose(i))
                .width(Fill)
                .padding([14, 20])
                .style(move |t, st| style::row_button(t, st, sel == Some(i)))
                .into()
            })
            .collect();
        let _ = p;
        scrollable(column(rows).spacing(6).padding([8, 28]))
            .height(Fill)
            .into()
    }

    fn entry_description(&self, e: Entry) -> String {
        match e {
            Entry::App(i) => self.app.catalog.apps[i].description.clone(),
            Entry::Builtin(b) => b.description().to_string(),
        }
    }

    fn tile<'a>(
        &self,
        i: usize,
        title: &str,
        desc: &str,
        glyph: &'static str,
        p: &Palette,
    ) -> Element<'a, Message> {
        let mut c = column![].spacing(8);
        if !glyph.is_empty() {
            c = c.push(text(glyph).size(28).color(p.accent));
        }
        c = c
            .push(text(title.to_string()).size(18).font(style::UI_FONT_BOLD))
            .push(text(desc.to_string()).size(14).style(style::dim_text));
        let selected = self.app.selected_row() == Some(i);
        button(c)
            .on_press(Message::Choose(i))
            .width(Fill)
            .height(Length::Fixed(168.0))
            .padding(22)
            .style(move |t, st| style::tile(t, st, selected))
            .into()
    }

    fn columns(&self) -> usize {
        ((self.size.width - 56.0) / 300.0).floor().clamp(1.0, 4.0) as usize
    }

    fn text_view(&self, p: &Palette) -> Element<'_, Message> {
        container(textpanel::view(&self.screen, p))
            .padding([4, 28])
            .height(Fill)
            .into()
    }

    fn running_view(&self, label: &str) -> Element<'_, Message> {
        centered_note(&format!(
            "{label} is open in its own window. Close it to come back here."
        ))
    }

    fn status_bar(&self, p: &Palette) -> Element<'_, Message> {
        let p = *p;
        let s = style::status_line(&self.app);
        let toast = self
            .app
            .current_toast()
            .map(|n| n.text.clone())
            .unwrap_or_default();
        container(
            row![
                text(toast).size(14),
                Space::new().width(Fill),
                text(s).size(14).style(style::dim_text),
            ]
            .spacing(16),
        )
        .padding([10, 28])
        .style(move |_| style::status_bar(&p))
        .into()
    }
}

fn grid<'a>(tiles: Vec<Element<'a, Message>>, cols: usize) -> Element<'a, Message> {
    let mut rows = column![].spacing(16);
    let mut it = tiles.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row![].spacing(16);
        for _ in 0..cols {
            match it.next() {
                Some(t) => r = r.push(t),
                None => r = r.push(Space::new().width(Fill)),
            }
        }
        rows = rows.push(r);
    }
    scrollable(container(rows).padding([8, 28]))
        .height(Fill)
        .into()
}

fn centered_note<'a>(s: &str) -> Element<'a, Message> {
    container(text(s.to_string()).size(18).style(style::dim_text))
        .center(Fill)
        .into()
}

/// "KNOWLEDGE & DOCUMENTATION" -> "Knowledge & documentation".
fn section_name(c: Category) -> String {
    let t = c.title().to_lowercase();
    let mut ch = t.chars();
    match ch.next() {
        Some(f) => f.to_uppercase().chain(ch).collect(),
        None => t,
    }
}

/// One plain glyph per section (the bundled font has them; no icon font).
fn icon(c: Category) -> &'static str {
    match c {
        Category::Knowledge => "\u{25a4}",
        Category::Maps => "\u{25ce}",
        Category::Communications => "\u{25c8}",
        Category::Networking => "\u{25cd}",
        Category::Radio => "\u{25c9}",
        Category::Security => "\u{25a3}",
        Category::Productivity => "\u{25a7}",
        Category::Entertainment => "\u{25c6}",
        Category::System => "\u{25a6}",
    }
}
