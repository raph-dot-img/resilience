//! TUI behaviour tests: navigation state machine and rendering at 80x24,
//! using ratatui's in-memory TestBackend (no real terminal needed).

use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use resilience_core::admin;
use resilience_core::catalog::{Catalog, Category};
use resilience_core::clock;
use resilience_core::config::{SystemConfig, ThemeName, ToolPaths, UserSettings};
use resilience_core::mode::{Detected, Mode};

use crate::app::{
    App, BACKUP_LABEL, Builtin, CHECK_LABEL, Effect, Entry, Modal, Paths, Pending, RESTORE_LABEL,
    Startup, View,
};
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
        tools: fake_media_tools(&home),
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
        paths: test_paths(&home),
        home,
        settings: UserSettings::default(),
        warnings: vec![],
    })
}

/// A fake system: helper configuration with three accounts (demo logs in
/// automatically, bob is Stable, carol is the only administrator), the group
/// database matching it, and no system generations.
fn test_paths(home: &Path) -> Paths {
    let sys = home.join("sys");
    std::fs::create_dir_all(sys.join("profiles")).expect("mkdir");
    std::fs::write(
        sys.join("admin.toml"),
        format!(
            r#"
state_file = "{0}/modes.toml"
audit_log = "{0}/audit.log"
lock_file = "{0}/lock"
gpasswd = "/bin/gpasswd"
loginctl = "/bin/loginctl"
timedatectl = "/bin/timedatectl"
helper = "/store/bin/resilience-admin"
pkexec = "/run/wrappers/bin/pkexec"
pkttyagent = "/store/bin/pkttyagent"
session_shell = "/run/current-system/sw/bin/resilience-session"
autologin_user = "demo"
[users.demo]
declared_mode = "ultra-stable"
[users.bob]
declared_mode = "stable"
[users.carol]
declared_mode = "advanced"
description = "Carol the admin"
"#,
            sys.display()
        ),
    )
    .expect("write admin.toml");
    std::fs::write(
        sys.join("group"),
        "wheel:x:1:carol\nresilience-stable:x:990:bob,carol\nnetworkmanager:x:57:bob,carol\n",
    )
    .expect("write group");
    Paths {
        admin_config: sys.join("admin.toml"),
        group_file: sys.join("group"),
        profiles: sys.join("profiles"),
        booted_system: sys.join("booted"),
        running_system: sys.join("current"),
    }
}

/// Fake `lsblk` and `udisksctl`: an internal disk, a USB stick (mounted
/// once `udisksctl mount` ran) and an encrypted SD card. udisksctl records
/// its arguments in `sys/udisks.log`; `sys/busy` makes unmount fail.
fn fake_media_tools(home: &Path) -> ToolPaths {
    use std::os::unix::fs::PermissionsExt;
    let sys = home.join("sys");
    std::fs::create_dir_all(&sys).expect("mkdir");
    let d = sys.display();
    let lsblk = format!(
        r#"#!/bin/sh
if [ -f {d}/mounted ]; then MP="/run/media/tester/STICK"; AV=1000000; else MP=""; AV=""; fi
cat <<LSBLK
NAME="vdb" PATH="/dev/vdb" PKNAME="" TYPE="disk" RM="0" HOTPLUG="0" RO="0" SIZE="67108864" LABEL="INTERNAL" FSTYPE="ext4" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
NAME="sda" PATH="/dev/sda" PKNAME="" TYPE="disk" RM="1" HOTPLUG="1" RO="0" SIZE="134217728" LABEL="" FSTYPE="" MOUNTPOINT="" FSAVAIL="" TRAN="usb" MODEL="Stick"
NAME="sda1" PATH="/dev/sda1" PKNAME="sda" TYPE="part" RM="0" HOTPLUG="0" RO="0" SIZE="134217728" LABEL="STICK" FSTYPE="vfat" MOUNTPOINT="$MP" FSAVAIL="$AV" TRAN="" MODEL=""
NAME="mmcblk0" PATH="/dev/mmcblk0" PKNAME="" TYPE="disk" RM="0" HOTPLUG="1" RO="0" SIZE="32000000000" LABEL="SECRET" FSTYPE="crypto_LUKS" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
LSBLK
"#
    );
    let udisksctl = format!(
        r#"#!/bin/sh
echo "$@" >> {d}/udisks.log
case "$1" in
  mount) touch {d}/mounted; echo "Mounted /dev/sda1 at /run/media/tester/STICK" ;;
  unmount)
    if [ -f {d}/busy ]; then
      echo "Error unmounting /dev/sda1: GDBus.Error:org.freedesktop.UDisks2.Error.DeviceBusy: Error unmounting /dev/sda1: target is busy" >&2
      exit 1
    fi
    rm -f {d}/mounted ;;
esac
"#
    );
    for (name, text) in [("lsblk", lsblk), ("udisksctl", udisksctl)] {
        let p = sys.join(name);
        std::fs::write(&p, text).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    ToolPaths {
        lsblk: sys.join("lsblk").display().to_string(),
        udisksctl: sys.join("udisksctl").display().to_string(),
        backup: "/store/bin/resilience-backup".into(),
        ..ToolPaths::default()
    }
}

fn udisks_log(a: &App) -> String {
    std::fs::read_to_string(a.home.join("sys/udisks.log")).unwrap_or_default()
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
    while a.stack.len() > 1 {
        press(&mut a, KeyCode::Esc);
    }
    open(&mut a, Builtin::Users);
    press(&mut a, KeyCode::Down);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::End);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    a.modal = None;
    open(&mut a, Builtin::Recovery);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Char('2'));
    println!("{}\n", render(&mut a, 80, 24));
    open(&mut a, Builtin::Backup);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
    press(&mut a, KeyCode::Enter);
    println!("{}\n", render(&mut a, 80, 24));
}

/// Every non-ASCII glyph the interface draws must exist in the console font
/// (ter-i16n, CP437 repertoire). ter-v16n once rendered the logo as garbage
/// on real hardware; the unit tests could not see it. Keep this list in sync
/// with nix/modules/interface.nix.
const CONSOLE_GLYPHS: &str = "▀▄█═║╔╗╚╝─│┌┐└┘░▒▓";

fn assert_console_glyphs(screen: &str) {
    for ch in screen.chars() {
        assert!(
            ch.is_ascii() || CONSOLE_GLYPHS.contains(ch),
            "glyph {ch:?} (U+{:04X}) is not in the console font",
            ch as u32
        );
    }
}

#[test]
fn only_console_font_glyphs_are_drawn() {
    let mut a = app(Mode::Advanced, "glyphs");
    let sequences: [&[KeyCode]; 7] = [
        &[],
        &[KeyCode::F(1)],
        &[KeyCode::F(5)],
        &[KeyCode::Char('/'), KeyCode::Char('e')],
        &[KeyCode::Char('2')],
        &[KeyCode::Char('q')],
        &[KeyCode::Char('1'), KeyCode::Enter],
    ];
    for seq in sequences {
        for c in seq {
            press(&mut a, *c);
        }
        for (w, h) in [(80, 24), (60, 20), (160, 50)] {
            for ch in render(&mut a, w, h).chars() {
                assert!(
                    ch.is_ascii() || CONSOLE_GLYPHS.contains(ch),
                    "glyph {ch:?} (U+{:04X}) is not in the console font",
                    ch as u32
                );
            }
        }
        a.modal = None;
        while a.stack.len() > 1 {
            press(&mut a, KeyCode::Esc);
        }
    }
}

fn open(a: &mut App, b: Builtin) {
    let (v, e) = a.activate(Entry::Builtin(b));
    assert_eq!(e, Effect::None);
    a.stack.push(v.expect("builtin opens a view"));
}

/// Move the selection of the top list view to the row whose text matches.
fn select_user(a: &mut App, name: &str) {
    let View::Users { rows, .. } = a.view() else {
        panic!("not on the users view")
    };
    let i = rows
        .iter()
        .position(|r| r.name == name)
        .expect("user listed");
    press(a, KeyCode::Home);
    for _ in 0..i {
        press(a, KeyCode::Down);
    }
}

#[test]
fn users_and_recovery_are_offered_in_every_mode() {
    for m in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let a = app(m, &format!("offer-{m:?}"));
        let sys = a.entries(Category::System);
        assert!(sys.contains(&Entry::Builtin(Builtin::Users)), "{m:?}");
        assert!(sys.contains(&Entry::Builtin(Builtin::Recovery)), "{m:?}");
        assert!(sys.contains(&Entry::Builtin(Builtin::Backup)), "{m:?}");
    }
}

#[test]
fn mode_change_explains_consequences_and_defaults_to_cancel() {
    let mut a = app(Mode::UltraStable, "setmode");
    open(&mut a, Builtin::Users);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("USERS & SECURITY MODES"), "{screen}");
    assert!(
        screen.contains("carol") && screen.contains("ADVANCED"),
        "{screen}"
    );
    select_user(&mut a, "bob");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.view(), View::ChooseMode { .. }));
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("NEW MODE FOR 'bob'"), "{screen}");
    press(&mut a, KeyCode::End); // Advanced
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Confirm {
            body,
            action,
            yes_focused,
            ..
        }) => {
            assert!(!yes_focused, "the safe choice is the default");
            assert!(body.contains("sudo"), "{body}");
            assert!(body.contains("administrator must type"), "{body}");
            assert!(body.contains("next login"), "{body}");
            assert_eq!(
                *action,
                Pending::SetMode {
                    user: "bob".into(),
                    to: Mode::Advanced
                }
            );
        }
        other => panic!("expected confirmation, got {other:?}"),
    }
    let shown = render(&mut a, 80, 24);
    assert!(shown.contains("CHANGE SECURITY MODE"), "{shown}");
    // Enter on the default button cancels; nothing runs.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
    // Confirm explicitly: the helper runs through pkexec with a fixed argv.
    press(&mut a, KeyCode::Enter);
    match press(&mut a, KeyCode::Char('y')) {
        Effect::Run(r) => {
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec",
                    "/store/bin/resilience-admin",
                    "set-mode",
                    "bob",
                    "advanced"
                ]
            );
            assert!(r.pause_after, "the helper's answer stays on screen");
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
            assert!(matches!(a.view(), View::Users { .. }), "{:?}", a.view());
            assert!(r.extra_args.is_empty());
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
            assert_eq!(
                r.env,
                vec![(
                    "SHELL".to_string(),
                    "/run/current-system/sw/bin/resilience-session".to_string()
                )]
            );
        }
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn demotion_warns_that_sessions_end() {
    let mut a = app(Mode::Stable, "demote");
    open(&mut a, Builtin::Users);
    select_user(&mut a, "bob");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Home); // Ultra-Stable
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm { body, .. }) = &a.modal else {
        panic!("expected confirmation")
    };
    assert!(body.contains("sessions of 'bob' will be closed"), "{body}");
}

#[test]
fn unsafe_mode_changes_are_refused_before_asking_for_a_password() {
    let mut a = app(Mode::Advanced, "refuse");
    open(&mut a, Builtin::Users);
    // carol is the only administrator.
    select_user(&mut a, "carol");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Home);
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Message { title, body, error }) => {
            assert_eq!(title, "NOT POSSIBLE");
            assert!(body.contains("last Advanced account"), "{body}");
            assert!(error);
        }
        other => panic!("expected refusal, got {other:?}"),
    }
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Esc);
    // demo logs in automatically and must stay Ultra-Stable.
    select_user(&mut a, "demo");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::End);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, .. }) if body.contains("automatically")),
        "{:?}",
        a.modal
    );
}

#[test]
fn no_administrator_means_no_mode_change() {
    let mut a = app(Mode::UltraStable, "noadmin");
    std::fs::write(&a.paths.group_file, "").expect("w");
    let cfg = std::fs::read_to_string(&a.paths.admin_config).expect("r");
    std::fs::write(
        &a.paths.admin_config,
        cfg.replace("declared_mode = \"advanced\"", "declared_mode = \"stable\""),
    )
    .expect("w");
    open(&mut a, Builtin::Users);
    press(&mut a, KeyCode::Enter);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, .. }) if title == "NO ADMINISTRATOR"),
        "{:?}",
        a.modal
    );
}

#[test]
fn users_view_survives_missing_or_corrupt_configuration() {
    let mut a = app(Mode::UltraStable, "badcfg");
    std::fs::write(&a.paths.admin_config, "this is = not [valid").expect("w");
    open(&mut a, Builtin::Users);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("could not be read"), "{screen}");
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    let mut b = app(Mode::UltraStable, "nocfg");
    std::fs::remove_file(&b.paths.admin_config).expect("rm");
    open(&mut b, Builtin::Users);
    assert!(render(&mut b, 80, 24).contains("could not be read"));
}

#[test]
fn recovery_section_shows_health_generations_and_guide() {
    let mut a = app(Mode::UltraStable, "recovery");
    std::fs::write(
        a.sys.handbook_dir.join("40-recovery.md"),
        "# Recovery\nWhen something goes wrong.\n",
    )
    .expect("w");
    open(&mut a, Builtin::Recovery);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("RECOVERY & DIAGNOSTICS"), "{screen}");
    assert!(screen.contains("[2] System generations"), "{screen}");
    press(&mut a, KeyCode::Char('1'));
    assert!(matches!(a.view(), View::Pager { title, .. } if title.contains("HEALTH")));
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('2'));
    match a.view() {
        View::Pager { text, .. } => {
            assert!(text.contains("No generations are recorded"), "{text}");
            assert!(text.contains("boot menu"), "{text}");
            assert!(text.contains("never your personal files"), "{text}");
        }
        other => panic!("expected pager, got {other:?}"),
    }
    press(&mut a, KeyCode::Esc);
    press(&mut a, KeyCode::Char('4'));
    assert!(
        matches!(a.view(), View::Pager { text, .. } if text.contains("When something goes wrong")),
        "{:?}",
        a.view()
    );
}

#[test]
fn diagnostic_report_is_private_and_never_overwritten() {
    use std::os::unix::fs::PermissionsExt;
    let mut a = app(Mode::UltraStable, "report");
    open(&mut a, Builtin::Recovery);
    press(&mut a, KeyCode::Char('3'));
    let Some(Modal::Message { title, body, error }) = &a.modal else {
        panic!("expected message, got {:?}", a.modal)
    };
    assert_eq!(title, "REPORT SAVED", "{body}");
    assert!(!error);
    let dir = a.home.join("Diagnostics");
    let files: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].path()).expect("read");
    assert!(text.contains("RESILIENCE OS DIAGNOSTIC REPORT"));
    assert!(text.contains("ULTRA-STABLE mode"), "{text}");
    let mode = files[0].metadata().expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn new_views_render_at_several_sizes() {
    let mut a = app(Mode::Advanced, "newsizes");
    for b in [Builtin::Users, Builtin::Recovery, Builtin::Backup] {
        open(&mut a, b);
        for (w, h) in [(80, 24), (40, 12), (120, 40), (200, 60)] {
            assert_console_glyphs(&render(&mut a, w, h));
        }
        if b == Builtin::Users {
            press(&mut a, KeyCode::Enter); // choose mode for the first account
            press(&mut a, KeyCode::Down);
            for (w, h) in [(80, 24), (40, 12), (120, 40)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            press(&mut a, KeyCode::Enter); // confirmation or refusal modal
            assert!(a.modal.is_some());
            for (w, h) in [(80, 24), (60, 20)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            a.modal = None;
        }
        if b == Builtin::Backup {
            press(&mut a, KeyCode::Enter); // open the stick
            assert!(matches!(a.view(), View::Backups { .. }));
            for (w, h) in [(80, 24), (40, 12), (120, 40)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            press(&mut a, KeyCode::Enter); // new backup confirmation
            assert!(a.modal.is_some());
            for (w, h) in [(80, 24), (60, 20)] {
                assert_console_glyphs(&render(&mut a, w, h));
            }
            a.modal = None;
        }
        while a.stack.len() > 1 {
            press(&mut a, KeyCode::Esc);
        }
    }
}

// ----- backups & USB sticks ---------------------------------------------------

fn media_names(a: &App) -> Vec<String> {
    let View::Media { volumes, .. } = a.view() else {
        panic!("not on the media view")
    };
    volumes.iter().map(|v| v.display_name()).collect()
}

#[test]
fn only_removable_media_are_listed_and_opening_mounts_without_a_password() {
    let mut a = app(Mode::UltraStable, "media");
    open(&mut a, Builtin::Backup);
    // The internal disk is never offered; the encrypted card is shown as unusable.
    assert_eq!(media_names(&a), vec!["STICK", "SECRET"]);
    let screen = render(&mut a, 80, 24);
    assert!(
        !screen.contains("INTERNAL") && !screen.contains("vdb"),
        "{screen}"
    );
    // The encrypted card cannot be opened.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    match &a.modal {
        Some(Modal::Message { title, body, .. }) => {
            assert_eq!(title, "CANNOT USE THIS DEVICE");
            assert!(body.contains("encrypted"), "{body}");
        }
        other => panic!("expected a message, got {other:?}"),
    }
    a.modal = None;
    assert_eq!(udisks_log(&a), "", "nothing was mounted");
    // The stick: mounted through udisks, never asking for a password.
    press(&mut a, KeyCode::Up);
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        udisks_log(&a),
        "mount --no-user-interaction --block-device /dev/sda1\n"
    );
    let View::Backups { mount, files, .. } = a.view() else {
        panic!("not on the backups view: {:?}", a.view())
    };
    assert_eq!(mount, Path::new("/run/media/tester/STICK"));
    assert!(files.is_empty());
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("BACKUPS ON STICK"), "{screen}");
    assert!(screen.contains("Make a new backup"), "{screen}");
    // Back on the list the stick shows as open; opening it again does not remount.
    press(&mut a, KeyCode::Esc);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains(" open"), "{screen}");
    press(&mut a, KeyCode::Enter);
    assert_eq!(udisks_log(&a).lines().count(), 1);
}

#[test]
fn new_backup_asks_first_then_runs_the_unprivileged_tool() {
    let mut a = app(Mode::UltraStable, "newbackup");
    std::fs::write(a.home.join("file"), vec![0u8; 4096]).expect("write");
    open(&mut a, Builtin::Backup);
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Enter); // "+ Make a new backup"
    let Some(Modal::Confirm {
        body,
        action,
        yes_focused,
        ..
    }) = &a.modal
    else {
        panic!("expected a confirmation, got {:?}", a.modal)
    };
    assert!(body.contains("passphrase"), "{body}");
    assert!(body.contains("Your files are not changed"), "{body}");
    assert!(!yes_focused, "Cancel is the default");
    assert_eq!(
        *action,
        Pending::Backup {
            mount: "/run/media/tester/STICK".into()
        }
    );
    // Enter on the default choice cancels.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());
    press(&mut a, KeyCode::Enter);
    let Effect::Run(req) = press(&mut a, KeyCode::Char('y')) else {
        panic!("expected a run request")
    };
    assert_eq!(req.label, BACKUP_LABEL);
    assert_eq!(req.argv, vec!["/store/bin/resilience-backup", "create"]);
    assert_eq!(
        req.extra_args,
        vec![PathBuf::from("/run/media/tester/STICK")]
    );
    assert!(req.pause_after && req.polkit_agent.is_none() && req.env.is_empty());
    a.after_run(&req.label, &Ok(std::process::ExitStatus::default()));
    assert!(matches!(a.view(), View::Backups { .. }));
}

#[test]
fn restore_and_check_use_the_selected_backup() {
    let mut a = app(Mode::Stable, "restore");
    let stick = a.home.join("stick");
    let dir = stick.join("Resilience-backups");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("home-20261114-103000.tar.zst.age"), "x").expect("w");
    std::fs::write(dir.join("home-20261101-080000.tar.zst.age"), "x").expect("w");
    std::fs::write(dir.join("home-20261101-080000.tar.zst.age.sha256"), "x").expect("w");
    open(&mut a, Builtin::Backup);
    let View::Media { volumes, .. } = a.view() else {
        panic!("media")
    };
    let volume = volumes[0].clone();
    a.stack.push(View::Backups {
        volume,
        mount: stick.clone(),
        files: resilience_core::backup::list_backups(&stick),
        list: ratatui::widgets::ListState::default().with_selected(Some(0)),
    });
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("2026-11-14 10:30:00 UTC"), "{screen}");
    press(&mut a, KeyCode::Down); // newest backup
    // c checks without asking anything.
    let Effect::Run(req) = press(&mut a, KeyCode::Char('c')) else {
        panic!("expected a run request")
    };
    assert_eq!(req.label, CHECK_LABEL);
    assert_eq!(req.argv[1], "check");
    assert_eq!(
        req.extra_args,
        vec![dir.join("home-20261114-103000.tar.zst.age")]
    );
    // Enter restores, after a confirmation that says where the files go.
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm {
        body, yes_focused, ..
    }) = &a.modal
    else {
        panic!("expected a confirmation")
    };
    assert!(!yes_focused);
    assert!(body.contains("Restored/backup-20261114-103000"), "{body}");
    assert!(body.contains("Nothing that exists is"), "{body}");
    assert!(body.contains("no checksum file"), "{body}");
    let Effect::Run(req) = press(&mut a, KeyCode::Char('y')) else {
        panic!("expected a run request")
    };
    assert_eq!(req.label, RESTORE_LABEL);
    assert_eq!(req.argv, vec!["/store/bin/resilience-backup", "restore"]);
    assert_eq!(
        req.extra_args,
        vec![dir.join("home-20261114-103000.tar.zst.age")]
    );
    // The older backup has a checksum: no warning.
    press(&mut a, KeyCode::Down);
    press(&mut a, KeyCode::Enter);
    let Some(Modal::Confirm { body, .. }) = &a.modal else {
        panic!("expected a confirmation")
    };
    assert!(!body.contains("no checksum file"), "{body}");
    a.modal = None;
    // The first row has nothing to check.
    press(&mut a, KeyCode::Home);
    assert_eq!(press(&mut a, KeyCode::Char('c')), Effect::None);
}

#[test]
fn eject_says_when_it_is_safe_to_unplug() {
    let mut a = app(Mode::UltraStable, "eject");
    open(&mut a, Builtin::Backup);
    press(&mut a, KeyCode::Enter); // mount and open
    std::fs::write(a.home.join("sys/busy"), "").expect("w");
    press(&mut a, KeyCode::Char('e'));
    match &a.modal {
        Some(Modal::Message { title, body, error }) => {
            assert_eq!(title, "CANNOT EJECT");
            assert!(*error && body.contains("Do not unplug it yet"), "{body}");
            assert!(body.contains("still in use"), "{body}");
        }
        other => panic!("expected a message, got {other:?}"),
    }
    a.modal = None;
    // Back on the list (the backups view closed); try again once not busy.
    assert!(matches!(a.view(), View::Media { .. }));
    std::fs::remove_file(a.home.join("sys/busy")).expect("rm");
    press(&mut a, KeyCode::Char('e'));
    match &a.modal {
        Some(Modal::Message { title, error, .. }) => {
            assert_eq!(title, "SAFE TO UNPLUG");
            assert!(!error);
        }
        other => panic!("expected a message, got {other:?}"),
    }
    let log = udisks_log(&a);
    assert!(
        log.ends_with(
            "unmount --no-user-interaction --block-device /dev/sda1\n\
             power-off --no-user-interaction --block-device /dev/sda\n"
        ),
        "{log}"
    );
    let View::Media { volumes, .. } = a.view() else {
        panic!("media")
    };
    assert!(volumes[0].mountpoint.is_none());
}

#[test]
fn media_view_survives_a_failing_lsblk() {
    let mut a = app(Mode::UltraStable, "nolsblk");
    a.sys.tools.lsblk = "/nonexistent/lsblk".into();
    open(&mut a, Builtin::Backup);
    let screen = render(&mut a, 80, 24);
    assert!(screen.contains("could not be read"), "{screen}");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Char('e'));
    assert!(a.modal.is_none());
    assert!(matches!(a.view(), View::Media { .. }));
}

// ----- date and time ------------------------------------------------------------

fn type_time(a: &mut App, text: &str) {
    for _ in 0..25 {
        press(a, KeyCode::Backspace);
    }
    for c in text.chars() {
        press(a, KeyCode::Char(c));
    }
}

fn open_clock(a: &mut App) {
    let (v, e) = a.activate(Entry::Builtin(Builtin::Clock));
    assert!(v.is_none());
    assert_eq!(e, Effect::None);
}

#[test]
fn clock_is_offered_in_every_mode_and_shows_the_current_time() {
    for m in [Mode::UltraStable, Mode::Stable, Mode::Advanced] {
        let a = app(m, &format!("clock-{m:?}"));
        assert!(
            a.entries(Category::System)
                .contains(&Entry::Builtin(Builtin::Clock)),
            "{m:?}"
        );
    }
    let mut a = app(Mode::UltraStable, "clock-open");
    open_clock(&mut a);
    match &a.modal {
        Some(Modal::SetTime { value, error, .. }) => {
            // Prefilled with the current local time, ready to correct.
            assert!(clock::parse_local(value).is_ok(), "{value}");
            assert!(error.is_none());
        }
        other => panic!("expected the clock dialog, got {other:?}"),
    }
    for (w, h) in [(80, 24), (60, 20), (40, 12), (120, 40)] {
        let s = render(&mut a, w, h);
        assert_console_glyphs(&s);
        if w >= 80 {
            assert!(s.contains("DATE & TIME") && s.contains("time zone"), "{s}");
        }
    }
}

#[test]
fn setting_the_clock_confirms_then_runs_the_helper_with_utc() {
    let mut a = app(Mode::UltraStable, "clock-set");
    open_clock(&mut a);
    type_time(&mut a, "2031-03-01 10:00");
    press(&mut a, KeyCode::Char('x')); // only digits and separators are typed
    press(&mut a, KeyCode::Enter);
    let expected = clock::local_to_unix(clock::parse_local("2031-03-01 10:00").expect("valid"))
        .expect("representable");
    match &a.modal {
        Some(Modal::Confirm {
            title,
            body,
            action,
            yes_focused,
            ..
        }) => {
            assert_eq!(title, "SET THE CLOCK");
            assert!(!yes_focused, "the safe choice is the default");
            assert!(body.contains("2031-03-01 10:00"), "{body}");
            assert!(body.contains("administrator must type"), "{body}");
            assert!(body.contains(&admin::rfc3339_utc(expected)), "{body}");
            assert_eq!(*action, Pending::SetTime { utc: expected });
        }
        other => panic!("expected confirmation, got {other:?}"),
    }
    assert_console_glyphs(&render(&mut a, 80, 24));
    // Enter on the default button cancels; nothing runs.
    assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
    assert!(a.modal.is_none());

    open_clock(&mut a);
    type_time(&mut a, "2031-03-01 10:00");
    press(&mut a, KeyCode::Enter);
    match press(&mut a, KeyCode::Char('y')) {
        Effect::Run(r) => {
            assert_eq!(
                r.argv,
                vec![
                    "/run/wrappers/bin/pkexec".to_string(),
                    "/store/bin/resilience-admin".into(),
                    "set-time".into(),
                    admin::rfc3339_utc(expected),
                ]
            );
            assert!(r.extra_args.is_empty());
            assert!(r.pause_after);
            assert_eq!(r.polkit_agent.as_deref(), Some("/store/bin/pkttyagent"));
            assert_eq!(r.env[0].0, "SHELL");
            a.after_run(&r.label, &Ok(std::process::ExitStatus::default()));
        }
        other => panic!("expected run, got {other:?}"),
    }
}

#[test]
fn impossible_or_out_of_range_times_are_explained_in_the_dialog() {
    let mut a = app(Mode::Stable, "clock-bad");
    open_clock(&mut a);
    for (input, why) in [
        ("2031-02-30 10:00", "does not exist"),
        ("2031-03-01 25:00", "does not exist"),
        ("2020-06-01 10:00", "2025 or later"),
        ("2100-06-01 10:00", "before the year 2100"),
        ("2031-3-1 10:00", "YYYY-MM-DD HH:MM"),
        ("", "YYYY-MM-DD HH:MM"),
    ] {
        type_time(&mut a, input);
        assert_eq!(press(&mut a, KeyCode::Enter), Effect::None);
        match &a.modal {
            Some(Modal::SetTime {
                error: Some(e),
                value,
                ..
            }) => {
                assert!(e.contains(why), "{input:?}: {e}");
                assert_eq!(value, input, "the typed text is kept for correction");
            }
            other => panic!("{input:?}: expected an error in the dialog, got {other:?}"),
        }
        assert_console_glyphs(&render(&mut a, 80, 24));
    }
    // Esc closes the dialog without doing anything.
    assert_eq!(press(&mut a, KeyCode::Esc), Effect::None);
    assert!(a.modal.is_none());
}

#[test]
fn no_administrator_means_no_clock_change() {
    let mut a = app(Mode::UltraStable, "clock-noadmin");
    std::fs::write(&a.paths.group_file, "").expect("w");
    let cfg = std::fs::read_to_string(&a.paths.admin_config).expect("r");
    std::fs::write(
        &a.paths.admin_config,
        cfg.replace("declared_mode = \"advanced\"", "declared_mode = \"stable\""),
    )
    .expect("w");
    open_clock(&mut a);
    assert!(
        matches!(&a.modal, Some(Modal::Message { title, body, .. })
            if title == "NO ADMINISTRATOR" && body.contains("Setting the clock")),
        "{:?}",
        a.modal
    );
}
