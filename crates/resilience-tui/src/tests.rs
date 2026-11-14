//! TUI behaviour tests: navigation state machine and rendering at 80x24,
//! using ratatui's in-memory TestBackend (no real terminal needed).

use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use resilience_core::catalog::{Catalog, Category};
use resilience_core::config::{SystemConfig, ThemeName, UserSettings};
use resilience_core::mode::{Detected, Mode};

use crate::app::{App, Builtin, Effect, Entry, Modal, Startup, View};
use crate::ui;

const CATALOG: &str = r#"
[[apps]]
id = "editor"
name = "Text editor"
category = "productivity"
description = "Edit text files"
command = ["/bin/sh", "-c", "true"]
restricted_command = ["/bin/sh", "-c", "restricted"]

[[apps]]
id = "monitor"
name = "Process monitor"
category = "system"
description = "Advanced-only tool"
command = ["/bin/sh"]
min_mode = "advanced"

[[apps]]
id = "ghost"
name = "Ghost game"
category = "entertainment"
description = "Program is missing"
command = ["/nonexistent/ghost"]
"#;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("resilience-tui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn app(mode: Mode, tag: &str) -> App {
    let home = tmp(tag);
    let hb = home.join("handbook");
    std::fs::create_dir_all(&hb).expect("mkdir");
    std::fs::write(
        hb.join("10-first.md"),
        "# First steps\nWelcome to Resilience.\n",
    )
    .expect("w");
    let sys = SystemConfig {
        handbook_dir: hb,
        edition: "test".into(),
        ..SystemConfig::default()
    };
    App::new(Startup {
        sys,
        catalog: Catalog::parse(CATALOG, Path::new("test")).expect("catalog"),
        who: Detected {
            mode,
            user: "tester".into(),
            groups: vec![],
        },
        settings_path: UserSettings::path(&home.join(".config")),
        home,
        settings: UserSettings::default(),
        warnings: vec![],
    })
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn press(app: &mut App, c: KeyCode) -> Effect {
    app.handle_key(key(c))
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    t.draw(|f| ui::draw(f, app)).expect("draw");
    let buf = t.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn home_renders_at_80x24() {
    let mut a = app(Mode::UltraStable, "home");
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("MAIN MENU"), "{screen}");
    assert!(screen.contains("STATUS"));
    assert!(screen.contains("ULTRA-STABLE"));
    assert!(screen.contains("[1] KNOWLEDGE & DOCUMENTATION"));
    assert!(screen.contains("F1 Help"));
    // Empty categories (maps, radio...) are not offered.
    assert!(!screen.contains("MAPS & NAVIGATION"));
    assert!(!screen.contains("RADIO"));
}

#[test]
fn renders_every_view_without_panicking_at_several_sizes() {
    let mut a = app(Mode::Advanced, "sizes");
    let opens: [&[KeyCode]; 6] = [
        &[],
        &[KeyCode::F(1)],
        &[KeyCode::F(5)],
        &[KeyCode::Char('/'), KeyCode::Char('n')],
        &[KeyCode::Char('1')],
        &[KeyCode::Char('1'), KeyCode::Enter],
    ];
    for seq in opens {
        for c in seq {
            press(&mut a, *c);
        }
        for (w, h) in [(80, 24), (40, 12), (120, 40), (20, 5), (200, 60)] {
            let s = render(&mut a, w, h);
            if w < 40 || h < 12 {
                assert!(s.contains("Terminal"), "small terminal notice at {w}x{h}");
            }
        }
        while a.stack.len() > 1 {
            press(&mut a, KeyCode::Esc);
        }
    }
}

#[test]
fn ultra_stable_has_no_shell_or_admin_entries() {
    let a = app(Mode::UltraStable, "ultra");
    let sys = a.entries(Category::System);
    assert!(!sys.contains(&Entry::Builtin(Builtin::Shell)));
    assert!(
        sys.iter().all(|e| !matches!(e, Entry::App(_))),
        "advanced-only app hidden"
    );
    assert!(
        a.search("shell").is_empty()
            || a.search("terminal").iter().all(|h| !matches!(
                h,
                crate::app::SearchHit::Entry(Entry::Builtin(Builtin::Shell))
            ))
    );
}

#[test]
fn stable_and_advanced_get_more() {
    let s = app(Mode::Stable, "stable");
    assert!(
        s.entries(Category::System)
            .contains(&Entry::Builtin(Builtin::Shell))
    );
    assert!(
        !s.entries(Category::System)
            .iter()
            .any(|e| matches!(e, Entry::App(_)))
    );
    let adv = app(Mode::Advanced, "adv");
    assert!(
        adv.entries(Category::System)
            .iter()
            .any(|e| matches!(e, Entry::App(_)))
    );
}

#[test]
fn ultra_stable_uses_restricted_command() {
    let mut a = app(Mode::UltraStable, "restricted");
    let cats = a.home_categories();
    let idx = cats
        .iter()
        .position(|c| *c == Category::Productivity)
        .expect("productivity");
    press(&mut a, KeyCode::Char(char::from(b'1' + idx as u8)));
    // Entries: [Notes builtin, Text editor app]
    let eff = press(&mut a, KeyCode::Char('2'));
    match eff {
        Effect::Run(r) => assert_eq!(r.argv, vec!["/bin/sh", "-c", "restricted"]),
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn missing_program_shows_error_not_crash() {
    let mut a = app(Mode::UltraStable, "missing");
    let cats = a.home_categories();
    let idx = cats
        .iter()
        .position(|c| *c == Category::Entertainment)
        .expect("entertainment");
    press(&mut a, KeyCode::Char(char::from(b'1' + idx as u8)));
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(a.modal, Some(Modal::Message { error: true, .. })));
    assert!(render(&mut a, 80, 24).contains("PROGRAM UNAVAILABLE"));
    press(&mut a, KeyCode::Enter);
    assert!(a.modal.is_none());
}

#[test]
fn power_off_requires_confirmation_and_defaults_to_cancel() {
    let mut a = app(Mode::UltraStable, "power");
    let (_, eff) = a.activate(Entry::Builtin(Builtin::PowerOff));
    assert_eq!(eff, Effect::None);
    assert!(matches!(
        a.modal,
        Some(Modal::Confirm {
            yes_focused: false,
            ..
        })
    ));
    // Enter on the default button cancels: nothing happens.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
}

#[test]
fn logout_confirmation_quits() {
    let mut a = app(Mode::UltraStable, "logout");
    assert_eq!(press(&mut a, KeyCode::Char('q')), Effect::None);
    assert!(a.modal.is_some());
    assert_eq!(press(&mut a, KeyCode::Char('n')), Effect::None);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Char('q'));
    assert_eq!(press(&mut a, KeyCode::Char('y')), Effect::Quit);
}

#[test]
fn ctrl_c_and_esc_never_leave_home() {
    let mut a = app(Mode::UltraStable, "ctrlc");
    assert_eq!(
        a.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Effect::None
    );
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.stack.len(), 1);
    assert!(matches!(a.view(), View::Home { .. }));
}

#[test]
fn handbook_opens_and_reads() {
    let mut a = app(Mode::UltraStable, "handbook");
    press(&mut a, KeyCode::Char('1')); // Knowledge
    press(&mut a, KeyCode::Enter); // Handbook
    assert!(matches!(a.view(), View::Handbook { .. }));
    press(&mut a, KeyCode::Enter); // First page
    match a.view() {
        View::Pager { title, text, .. } => {
            assert_eq!(title, "FIRST STEPS");
            assert!(text.contains("Welcome"));
        }
        _ => panic!("expected pager"),
    }
    assert!(render(&mut a, 80, 24).contains("Welcome to Resilience"));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.stack.len(), 1);
}

#[test]
fn search_finds_apps_tools_and_pages() {
    let mut a = app(Mode::UltraStable, "search");
    press(&mut a, KeyCode::Char('/'));
    for c in "first".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    let View::Search { hits, .. } = a.view() else {
        panic!("search view")
    };
    assert!(
        hits.iter()
            .any(|h| matches!(h, crate::app::SearchHit::Page(_)))
    );
    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    for c in "editor".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    // '?' and 'q' are typed into the query, not interpreted as commands.
    press(&mut a, KeyCode::Char('?'));
    let View::Search { query, .. } = a.view() else {
        panic!("search view")
    };
    assert_eq!(query, "editor?");
}

#[test]
fn new_note_validates_name_and_opens_editor() {
    let mut a = app(Mode::UltraStable, "notes");
    let (v, _) = a.activate(Entry::Builtin(Builtin::Notes));
    a.stack.push(v.expect("notes view"));
    press(&mut a, KeyCode::Char('n'));
    for c in "../bad".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(matches!(
        a.modal,
        Some(Modal::NewNote { error: Some(_), .. })
    ));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('n'));
    for c in "groceries".chars() {
        press(&mut a, KeyCode::Char(c));
    }
    match press(&mut a, KeyCode::Enter) {
        Effect::Run(r) => {
            assert_eq!(r.argv, a.sys.restricted_editor);
            assert!(r.extra_args[0].ends_with("Notes/groceries.txt"));
        }
        other => panic!("expected editor, got {other:?}"),
    }
    let View::Notes { notes, .. } = a.view() else {
        panic!("notes view")
    };
    assert_eq!(notes.len(), 1);
}

#[test]
fn theme_change_is_saved() {
    let mut a = app(Mode::UltraStable, "theme");
    let (v, _) = a.activate(Entry::Builtin(Builtin::Settings));
    a.stack.push(v.expect("settings"));
    press(&mut a, KeyCode::End); // Mono
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.settings.theme, ThemeName::Mono);
    let (loaded, _) = UserSettings::load(&a.settings_path);
    assert_eq!(loaded.theme, ThemeName::Mono);
    assert!(render(&mut a, 80, 24).contains("(current)"));
}

#[test]
fn shell_request_comes_from_system_config() {
    let mut a = app(Mode::Stable, "shell");
    let (_, eff) = a.activate(Entry::Builtin(Builtin::Shell));
    match eff {
        Effect::Run(r) => assert_eq!(r.argv, a.sys.shell),
        other => panic!("expected shell, got {other:?}"),
    }
    // Defence in depth: even if requested directly, ultra-stable gets nothing.
    let mut u = app(Mode::UltraStable, "shell-ultra");
    let (_, eff) = u.activate(Entry::Builtin(Builtin::Shell));
    assert_eq!(eff, Effect::None);
}

#[test]
fn wrap_preserves_content() {
    use ratatui::text::Line;
    let long = Line::from("alpha beta gamma delta epsilon zeta eta theta iota kappa");
    let out = ui::wrap(vec![long], 12);
    assert!(out.iter().all(|l| l.width() <= 12));
    let joined: Vec<String> = out
        .iter()
        .map(|l| l.to_string().trim().to_string())
        .collect();
    assert_eq!(
        joined.join(" "),
        "alpha beta gamma delta epsilon zeta eta theta iota kappa"
    );
    let huge = ui::wrap(vec![Line::from("x".repeat(50))], 10);
    assert_eq!(huge.len(), 5);
}

/// `RESILIENCE_DUMP=1 cargo test dump_screens -- --nocapture` prints screens
/// for visual review.
#[test]
fn dump_screens() {
    if std::env::var_os("RESILIENCE_DUMP").is_none() {
        return;
    }
    let mut a = app(Mode::Stable, "dump");
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Char('4'));
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
}
