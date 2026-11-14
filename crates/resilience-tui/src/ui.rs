//! Rendering. Pure functions of [`App`] state (scroll offsets are clamped
//! here, which is the only mutation).
//!
//! Layout targets 80x24 as the minimum comfortable size and degrades to a
//! "terminal too small" notice below 40x12. Only box-drawing and block
//! characters present in the Linux console's default font are used.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};
use resilience_core::catalog::{Availability, Category};
use resilience_core::config::ThemeName;
use resilience_core::human_bytes;
use resilience_core::mode::Mode;
use resilience_core::network::{LinkLevel, ProbeOutcome};
use resilience_core::status::format_uptime;

use crate::app::{App, Entry, Level, Modal, SearchHit, View, category_description, level_tag};
use crate::help;
use crate::theme::Theme;

const LOGO: [&str; 2] = [
    "█▀█ █▀▀ █▀ █ █   █ █▀▀ █▄ █ █▀▀ █▀▀",
    "█▀▄ ██▄ ▄█ █ █▄▄ █ ██▄ █ ▀█ █▄▄ ██▄",
];
const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

/// Local wall-clock time as HH:MM:SS.
pub fn clock(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    // SAFETY: localtime_r writes into the provided, zero-initialised struct.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return "--:--:--".into();
        }
        tm
    };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

fn block<'a>(title: &'a str, theme: &Theme) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(theme.border)
        .title(Span::styled(format!(" {title} "), theme.title))
}

/// Status marker that carries meaning without colour.
fn marker(ok: Option<bool>, t: &Theme) -> Span<'static> {
    match ok {
        Some(true) => Span::styled("[OK]", t.ok),
        Some(false) => Span::styled("[!!]", t.warn),
        None => Span::styled("[--]", t.dim),
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let theme = app.theme;
    f.render_widget(Block::new().style(theme.base), area);
    if area.width < 40 || area.height < 12 {
        let msg = Paragraph::new(format!(
            "Terminal too small ({}x{}).\nResilience needs at least 40x12; 80x24 recommended.",
            area.width, area.height
        ))
        .wrap(Wrap { trim: true })
        .style(theme.warn);
        f.render_widget(msg, area);
        return;
    }
    let tall = area.height >= 22 && area.width >= 60;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(if tall { 5 } else { 3 }),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(area);
    draw_header(f, app, header, tall);
    draw_body(f, app, body);
    draw_footer(f, app, footer);
    draw_modal(f, app, area);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect, tall: bool) {
    let t = &app.theme;
    let b = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(t.border);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let right = vec![
        Line::from(vec![Span::styled(clock(SystemTime::now()), t.accent)]),
        Line::from(Span::styled(
            format!("{} v{}", app.sys.edition.to_uppercase(), app.sys.version),
            t.dim,
        )),
        Line::from(Span::styled(format!("MODE: {}", app.mode()), t.accent)),
    ];
    if tall {
        let lines = vec![
            Line::from(Span::styled(LOGO[0], t.title)),
            Line::from(Span::styled(LOGO[1], t.title)),
            Line::from(Span::styled("OS :: AUTONOMOUS INFORMATION SYSTEM", t.base)),
        ];
        f.render_widget(Paragraph::new(lines), inner);
        f.render_widget(Paragraph::new(right).alignment(Alignment::Right), inner);
    } else {
        let line = Line::from(vec![
            Span::styled("RESILIENCE OS", t.title),
            Span::styled(" :: AUTONOMOUS INFORMATION SYSTEM", t.base),
        ]);
        f.render_widget(Paragraph::new(line), inner);
        let compact = format!("{}  {}", app.mode(), clock(SystemTime::now()));
        f.render_widget(
            Paragraph::new(Span::styled(compact, t.accent)).alignment(Alignment::Right),
            inner,
        );
    }
}

fn footer_hints(view: &View) -> &'static str {
    match view {
        View::Home { .. } => "1-9/Enter Open  / Search  F1 Help  F5 Status  q Log out",
        View::Category { .. } => "Enter Open  Esc Back  / Search  F1 Help  F5 Status",
        View::Handbook { .. } => "Enter Read  Esc Back  F1 Help",
        View::Pager { .. } | View::Status { .. } | View::Help { .. } => {
            "Up/Down PgUp/PgDn Scroll  Esc Back"
        }
        View::Notes { .. } => "Enter Open  n New note  r Refresh  Esc Back",
        View::Network { .. } => "p Probe Internet  r Refresh  Esc Back",
        View::Settings { .. } => "Enter Apply theme  Esc Back",
        View::Search { .. } => "Type to search  Up/Down Select  Enter Open  Esc Back",
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let [keys, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    f.render_widget(
        Paragraph::new(Span::styled(footer_hints(app.view()), t.accent)),
        keys,
    );
    let line = match app.current_toast() {
        Some(n) => {
            let style = match n.level {
                Level::Info => t.base,
                Level::Warn => t.warn,
                Level::Error => t.error,
            };
            Line::from(vec![
                Span::styled(level_tag(n.level), style),
                Span::raw(" "),
                Span::styled(n.text.clone(), style),
            ])
        }
        None => Line::from(Span::styled("Ready.", t.dim)),
    };
    f.render_widget(Paragraph::new(line), status);
    if app.probe.running.is_some() {
        let s = format!(
            "probing network {}",
            SPINNER[(app.tick as usize / 2) % SPINNER.len()]
        );
        f.render_widget(
            Paragraph::new(Span::styled(s, t.warn)).alignment(Alignment::Right),
            status,
        );
    }
}

fn draw_body(f: &mut Frame, app: &mut App, area: Rect) {
    // Take the top view out to satisfy the borrow checker; always put it back.
    let mut view = app.stack.pop().expect("view stack is never empty");
    match &mut view {
        View::Home { list } => draw_home(f, app, area, list),
        View::Category { cat, list } => draw_category(f, app, area, *cat, list),
        View::Handbook { pages, list, error } => {
            let t = app.theme;
            if let Some(e) = error {
                let p = Paragraph::new(format!("The handbook could not be opened.\n\n{e}"))
                    .wrap(Wrap { trim: false })
                    .style(t.error)
                    .block(block("RESILIENCE HANDBOOK", &t));
                f.render_widget(p, area);
            } else {
                let items: Vec<ListItem> = pages
                    .iter()
                    .map(|p| ListItem::new(p.title.clone()))
                    .collect();
                draw_list(
                    f,
                    &t,
                    area,
                    "RESILIENCE HANDBOOK",
                    items,
                    list,
                    "No handbook pages are installed.",
                );
            }
        }
        View::Pager {
            title,
            text,
            scroll,
        } => {
            let lines: Vec<Line> = text.lines().map(|l| Line::from(l.to_string())).collect();
            draw_scroll(f, &app.theme, area, title, lines, scroll);
        }
        View::Help { topic, scroll } => {
            let lines = help::lines(*topic, app.mode())
                .into_iter()
                .map(Line::from)
                .collect();
            draw_scroll(f, &app.theme, area, "HELP", lines, scroll);
        }
        View::Status { scroll } => {
            let lines = status_lines(app);
            draw_scroll(f, &app.theme, area, "SYSTEM STATUS", lines, scroll);
        }
        View::Network { scroll } => {
            let lines = network_lines(app);
            draw_scroll(
                f,
                &app.theme,
                area,
                "NETWORK STATUS & DIAGNOSTICS",
                lines,
                scroll,
            );
        }
        View::Notes { notes, list } => {
            let t = app.theme;
            let items: Vec<ListItem> = notes
                .iter()
                .map(|n| ListItem::new(format!("{:<40} {:>10}", n.name, human_bytes(n.size))))
                .collect();
            draw_list(
                f,
                &t,
                area,
                "NOTES (~/Notes)",
                items,
                list,
                "No notes yet. Press 'n' to create one.",
            );
        }
        View::Settings { list } => draw_settings(f, app, area, list),
        View::Search { query, hits, list } => draw_search(f, app, area, query, hits, list),
    }
    app.stack.push(view);
}

fn net_summary(app: &App) -> (Option<bool>, String) {
    if app.probe.running.is_some() {
        return (None, "PROBING...".into());
    }
    match (app.net.level, &app.probe.report) {
        (LinkLevel::Routed, Some(r)) if r.internet.is_ok() => {
            (Some(true), format!("ONLINE ({})", clock(r.finished)))
        }
        (LinkLevel::Routed, Some(_)) => (Some(false), "ROUTE UP, NO INTERNET".into()),
        (LinkLevel::Routed, None) => (None, "ROUTE UP (UNVERIFIED)".into()),
        (LinkLevel::LocalLink, _) => (None, "LOCAL LINK ONLY".into()),
        (LinkLevel::Offline, _) => (None, "OFFLINE".into()),
    }
}

fn status_panel(app: &App) -> Vec<Line<'static>> {
    let t = &app.theme;
    let s = &app.snapshot;
    let row = |label: &str, m: Span<'static>, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<8}"), t.dim),
            m,
            Span::raw(" "),
            Span::styled(value, t.base),
        ])
    };
    let failed = app.failed_units.len();
    let sys_value = if failed > 0 {
        format!("{} ({failed} failed)", app.sys_state.label())
    } else {
        app.sys_state.label()
    };
    let (net_ok, net_text) = net_summary(app);
    let power = match (s.batteries.first(), s.ac_power) {
        (Some(b), _) => {
            let low = b.capacity.is_some_and(|c| c <= 15) && b.status == "Discharging";
            let cap = b
                .capacity
                .map(|c| format!("{c}%"))
                .unwrap_or_else(|| "?%".into());
            (Some(!low), format!("BAT {cap} {}", b.status.to_uppercase()))
        }
        (None, Some(true)) => (Some(true), "AC POWER".into()),
        _ => (None, "NO BATTERY".into()),
    };
    let storage = match &s.home {
        Some(st) => {
            let low = st.total > 0 && st.available * 20 < st.total; // < 5 %
            (Some(!low), format!("{} FREE", human_bytes(st.available)))
        }
        None => (None, "UNKNOWN".into()),
    };
    let mem = match s.memory {
        Some(m) => {
            let used = m.total_kib.saturating_sub(m.available_kib) * 1024;
            (
                Some(m.available_kib * 10 >= m.total_kib),
                format!(
                    "{} / {}",
                    human_bytes(used),
                    human_bytes(m.total_kib * 1024)
                ),
            )
        }
        None => (None, "UNKNOWN".into()),
    };
    vec![
        row(
            "SYSTEM",
            marker(Some(app.sys_state.is_ok() && failed == 0), t),
            sys_value,
        ),
        row(
            "MODE",
            Span::styled("[--]", t.dim),
            app.mode().label().into(),
        ),
        row("NETWORK", marker(net_ok, t), net_text),
        row("POWER", marker(power.0, t), power.1),
        row("STORAGE", marker(storage.0, t), storage.1),
        row("MEMORY", marker(mem.0, t), mem.1),
        row(
            "UPTIME",
            Span::styled("[--]", t.dim),
            s.uptime.map(format_uptime).unwrap_or_else(|| "?".into()),
        ),
        row(
            "USER",
            Span::styled("[--]", t.dim),
            format!("{}@{}", app.who.user, s.hostname),
        ),
    ]
}

fn draw_home(f: &mut Frame, app: &App, area: Rect, list: &mut ListState) {
    let t = app.theme;
    let wide = area.width >= 76;
    let [status_area, menu_area] = if wide {
        Layout::horizontal([Constraint::Length(38), Constraint::Min(0)]).areas(area)
    } else {
        Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(area)
    };
    f.render_widget(
        Paragraph::new(status_panel(app)).block(block("STATUS", &t)),
        status_area,
    );

    let cats = app.home_categories();
    let items: Vec<ListItem> = cats
        .iter()
        .enumerate()
        .map(|(i, c)| ListItem::new(format!("[{}] {}", i + 1, c.title())))
        .collect();
    let b = block("MAIN MENU", &t);
    let inner = b.inner(menu_area);
    f.render_widget(b, menu_area);
    let desc_h = if inner.height > cats.len() as u16 + 3 {
        2
    } else {
        0
    };
    let [list_area, desc_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(desc_h)]).areas(inner);
    let l = List::new(items)
        .style(t.base)
        .highlight_style(t.selected)
        .highlight_symbol("> ");
    f.render_stateful_widget(l, list_area, list);
    if let Some(c) = list.selected().and_then(|i| cats.get(i)) {
        f.render_widget(
            Paragraph::new(category_description(*c))
                .style(t.dim)
                .wrap(Wrap { trim: true }),
            desc_area,
        );
    }
}

fn entry_detail(app: &App, e: Entry) -> Vec<Line<'static>> {
    let t = &app.theme;
    let mode = app.mode();
    match e {
        Entry::Builtin(b) => vec![
            Line::from(Span::styled(b.name().to_string(), t.accent)),
            Line::from(""),
            Line::from(b.description().to_string()),
            Line::from(""),
            Line::from(Span::styled("Built into the Resilience interface.", t.dim)),
        ],
        Entry::App(i) => {
            let a = &app.catalog.apps[i];
            let (m, state) = match a.availability(mode) {
                Availability::Ready(_) => {
                    (marker(Some(true), t), "Installed and ready".to_string())
                }
                Availability::Missing(p) => (marker(Some(false), t), format!("Unavailable: {p}")),
            };
            let mut lines = vec![
                Line::from(Span::styled(a.name.clone(), t.accent)),
                Line::from(""),
                Line::from(a.description.clone()),
                Line::from(""),
                Line::from(vec![m, Span::raw(" "), Span::raw(state)]),
            ];
            if mode == Mode::UltraStable && a.restricted_command.is_some() {
                lines.push(Line::from(Span::styled(
                    "Runs in restricted mode for your account.",
                    t.dim,
                )));
            }
            lines
        }
    }
}

fn draw_category(f: &mut Frame, app: &App, area: Rect, cat: Category, list: &mut ListState) {
    let t = app.theme;
    let entries = app.entries(cat);
    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let num = if i < 9 {
                format!("[{}] ", i + 1)
            } else {
                "    ".into()
            };
            let suffix = if app.entry_ready(*e) {
                ""
            } else {
                "  [UNAVAILABLE]"
            };
            ListItem::new(format!("{num}{}{suffix}", app.entry_name(*e)))
        })
        .collect();
    let [list_area, detail_area] = if area.width >= 76 {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(0), Constraint::Length(7)]).areas(area)
    };
    draw_list(
        f,
        &t,
        list_area,
        cat.title(),
        items,
        list,
        "Nothing is available here for your account.",
    );
    if let Some(e) = list.selected().and_then(|i| entries.get(i)) {
        let p = Paragraph::new(entry_detail(app, *e))
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("DETAILS", &t));
        f.render_widget(p, detail_area);
    }
}

fn draw_list(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    title: &str,
    items: Vec<ListItem>,
    list: &mut ListState,
    empty: &str,
) {
    let b = block(title, t);
    if items.is_empty() {
        f.render_widget(
            Paragraph::new(empty.to_string())
                .style(t.dim)
                .wrap(Wrap { trim: true })
                .block(b),
            area,
        );
        return;
    }
    let l = List::new(items)
        .style(t.base)
        .highlight_style(t.selected)
        .highlight_symbol("> ")
        .block(b);
    f.render_stateful_widget(l, area, list);
}

/// Word-wrap plain lines to `width` columns (by character count).
pub fn wrap(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(10);
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if line.width() <= width {
            out.push(line);
            continue;
        }
        // Styled lines that overflow are flattened to the first span's style.
        let style = line.spans.first().map(|s| s.style).unwrap_or_default();
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let indent: String = text.chars().take_while(|c| *c == ' ').collect();
        let mut cur = String::new();
        for word in text.split(' ').filter(|w| !w.is_empty()) {
            let need = cur.chars().count() + usize::from(!cur.is_empty()) + word.chars().count();
            if !cur.is_empty() && need > width {
                out.push(Line::styled(std::mem::take(&mut cur), style));
                cur.push_str(&indent);
            }
            if !cur.is_empty() && !cur.ends_with(' ') {
                cur.push(' ');
            }
            // Hard-split words longer than the width.
            let mut w = word.to_string();
            while w.chars().count() > width {
                let head: String = w.chars().take(width).collect();
                w = w.chars().skip(width).collect();
                if !cur.trim().is_empty() {
                    out.push(Line::styled(std::mem::take(&mut cur), style));
                }
                out.push(Line::styled(head, style));
            }
            cur.push_str(&w);
        }
        if !cur.is_empty() {
            out.push(Line::styled(cur, style));
        }
    }
    out
}

fn draw_scroll(
    f: &mut Frame,
    t: &Theme,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    scroll: &mut usize,
) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let inner_h = area.height.saturating_sub(2) as usize;
    let lines = wrap(lines, inner_w);
    let max = lines.len().saturating_sub(inner_h);
    *scroll = (*scroll).min(max);
    let pos = if max == 0 {
        String::new()
    } else {
        format!(" {}/{} ", *scroll + 1, max + 1)
    };
    let b = block(title, t).title_bottom(Line::from(Span::styled(pos, t.dim)).right_aligned());
    let visible: Vec<Line> = lines.into_iter().skip(*scroll).take(inner_h).collect();
    f.render_widget(
        Paragraph::new(Text::from(visible)).style(t.base).block(b),
        area,
    );
}

fn kv(t: &Theme, k: &str, v: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {k:<20}"), t.dim),
        Span::styled(v.into(), t.base),
    ])
}

fn heading(t: &Theme, s: &str) -> Line<'static> {
    Line::from(Span::styled(format!("== {s} =="), t.accent))
}

fn status_lines(app: &App) -> Vec<Line<'static>> {
    let t = &app.theme;
    let s = &app.snapshot;
    let mut l = vec![heading(t, "SYSTEM")];
    let state_ok = app.sys_state.is_ok() && app.failed_units.is_empty();
    l.push(Line::from(vec![
        Span::raw("  "),
        marker(Some(state_ok), t),
        Span::raw(format!(" systemd reports: {}", app.sys_state.label())),
    ]));
    if app.failed_units.is_empty() {
        l.push(kv(t, "Failed services", "none"));
    } else {
        l.push(kv(t, "Failed services", app.failed_units.len().to_string()));
        for u in &app.failed_units {
            l.push(Line::from(Span::styled(format!("    [XX] {u}"), t.error)));
        }
    }
    l.push(kv(t, "Operating system", s.os_name.clone()));
    l.push(kv(
        t,
        "Edition",
        format!(
            "{} (userland v{})",
            app.sys.edition,
            resilience_core::VERSION
        ),
    ));
    l.push(kv(t, "Kernel", s.kernel.clone()));
    l.push(kv(t, "Hostname", s.hostname.clone()));
    l.push(kv(
        t,
        "Uptime",
        s.uptime
            .map(format_uptime)
            .unwrap_or_else(|| "unknown".into()),
    ));
    if let Some([a, b, c]) = s.load {
        l.push(kv(t, "Load average", format!("{a:.2} {b:.2} {c:.2}")));
    }
    l.push(kv(
        t,
        "System generation",
        s.system_generation
            .clone()
            .unwrap_or_else(|| "not a NixOS system".into()),
    ));
    l.push(Line::from(""));
    l.push(heading(t, "ACCOUNT & SECURITY MODE"));
    l.push(kv(t, "User", app.who.user.clone()));
    l.push(kv(t, "Mode", app.mode().label()));
    l.push(kv(
        t,
        "Groups",
        if app.who.groups.is_empty() {
            "(none)".into()
        } else {
            app.who.groups.join(", ")
        },
    ));
    l.push(Line::from(format!("  {}", app.mode().description())));
    l.push(Line::from(""));
    l.push(heading(t, "STORAGE & MEMORY"));
    for (name, st) in [("System (/)", &s.root), ("Your files (home)", &s.home)] {
        match st {
            Some(st) => l.push(kv(
                t,
                name,
                format!(
                    "{} free of {}",
                    human_bytes(st.available),
                    human_bytes(st.total)
                ),
            )),
            None => l.push(kv(t, name, "unknown")),
        }
    }
    if let Some(m) = s.memory {
        l.push(kv(
            t,
            "Memory available",
            format!(
                "{} of {}",
                human_bytes(m.available_kib * 1024),
                human_bytes(m.total_kib * 1024)
            ),
        ));
    }
    l.push(Line::from(""));
    l.push(heading(t, "POWER"));
    if s.batteries.is_empty() {
        l.push(kv(t, "Battery", "none detected"));
    }
    for b in &s.batteries {
        let cap = b
            .capacity
            .map(|c| format!("{c}%"))
            .unwrap_or_else(|| "unknown".into());
        l.push(kv(
            t,
            &format!("Battery {}", b.name),
            format!("{cap}, {}", b.status),
        ));
    }
    l.push(kv(
        t,
        "Mains power",
        match s.ac_power {
            Some(true) => "connected",
            Some(false) => "disconnected",
            None => "no adapter reported",
        },
    ));
    l.push(Line::from(""));
    l.push(heading(t, "APPLICATIONS"));
    let mode = app.mode();
    let visible: Vec<_> = app
        .catalog
        .apps
        .iter()
        .filter(|a| mode.allows(a.min_mode))
        .collect();
    let missing: Vec<_> = visible
        .iter()
        .filter(|a| !matches!(a.availability(mode), Availability::Ready(_)))
        .collect();
    l.push(kv(
        t,
        "Available to you",
        format!("{} programs, {} unavailable", visible.len(), missing.len()),
    ));
    for a in missing {
        l.push(Line::from(Span::styled(
            format!("    [!!] {} ({})", a.name, a.id),
            t.warn,
        )));
    }
    l
}

fn outcome_line(t: &Theme, label: &str, o: &ProbeOutcome) -> Line<'static> {
    let (m, text) = match o {
        ProbeOutcome::Ok { detail, elapsed } => (
            marker(Some(true), t),
            format!("{detail} in {} ms", elapsed.as_millis()),
        ),
        ProbeOutcome::Failed(e) => (marker(Some(false), t), e.clone()),
        ProbeOutcome::Skipped(why) => (marker(None, t), why.clone()),
    };
    Line::from(vec![
        Span::styled(format!("  {label:<20}"), t.dim),
        m,
        Span::raw(format!(" {text}")),
    ])
}

fn network_lines(app: &App) -> Vec<Line<'static>> {
    let t = &app.theme;
    let n = &app.net;
    let (ok, summary) = net_summary(app);
    let mut l = vec![
        heading(t, "SUMMARY"),
        Line::from(vec![
            Span::raw("  "),
            marker(ok, t),
            Span::styled(format!(" {summary}"), t.accent),
        ]),
        Line::from(match n.level {
            LinkLevel::Offline => {
                "  No network cable or wireless connection is active. Everything stored on this computer keeps working."
            }
            LinkLevel::LocalLink => {
                "  Connected to a local network, but there is no route to other networks. Local services may work."
            }
            LinkLevel::Routed => {
                "  A route to other networks exists. Press 'p' to test whether the Internet is actually reachable."
            }
        }),
        Line::from(""),
        heading(t, "INTERFACES"),
    ];
    let mut any = false;
    for i in n
        .interfaces
        .iter()
        .filter(|i| i.kind != resilience_core::network::IfKind::Loopback)
    {
        any = true;
        let addrs = if i.addresses.is_empty() {
            "no address".to_string()
        } else {
            i.addresses
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let state = if i.is_connected() {
            "connected"
        } else if i.operstate == "up" {
            "up, no carrier"
        } else {
            "disconnected"
        };
        l.push(Line::from(vec![
            Span::raw("  "),
            marker(Some(i.is_connected()), t),
            Span::styled(
                format!(" {:<10} {:<9} {state}", i.name, i.kind.label()),
                t.base,
            ),
        ]));
        l.push(Line::from(Span::styled(format!("       {addrs}"), t.dim)));
    }
    if !any {
        l.push(Line::from("  No network hardware detected."));
    }
    l.push(Line::from(""));
    l.push(heading(t, "ROUTING & DNS"));
    l.push(kv(
        t,
        "IPv4 gateway",
        match &n.gateway_v4 {
            Some((ifc, gw)) => {
                let seen = match n.gateway_arp {
                    Some(true) => "answered recently (ARP)",
                    _ => "not seen on the wire yet",
                };
                format!("{gw} via {ifc}, {seen}")
            }
            None => "none".into(),
        },
    ));
    l.push(kv(
        t,
        "IPv6 default route",
        n.default_v6_iface
            .clone()
            .map(|i| format!("via {i}"))
            .unwrap_or_else(|| "none".into()),
    ));
    l.push(kv(
        t,
        "DNS servers",
        if n.nameservers.is_empty() {
            "none configured".into()
        } else {
            n.nameservers
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        },
    ));
    l.push(Line::from(""));
    l.push(heading(t, "INTERNET PROBE (ON REQUEST ONLY)"));
    match (&app.probe.running, &app.probe.report) {
        (Some((_, started)), _) => l.push(Line::from(format!(
            "  Probing... ({} s)",
            started.elapsed().as_secs()
        ))),
        (None, Some(r)) => {
            l.push(Line::from(Span::styled(
                format!("  Last probe at {}", clock(r.finished)),
                t.dim,
            )));
            l.push(outcome_line(t, "Internet (TCP)", &r.internet));
            l.push(outcome_line(t, "DNS lookup", &r.dns));
        }
        (None, None) => l.push(Line::from(
            "  Not run. Resilience never contacts the Internet unless you press 'p'.",
        )),
    }
    let targets: Vec<String> = app
        .probe
        .config
        .targets
        .iter()
        .map(|a| a.to_string())
        .collect();
    l.push(Line::from(Span::styled(
        format!(
            "  The probe contacts {} and resolves '{}', each with a {} ms timeout.",
            if targets.is_empty() {
                "nothing".into()
            } else {
                targets.join(", ")
            },
            app.probe.config.dns_name,
            app.probe.config.timeout.as_millis()
        ),
        t.dim,
    )));
    l
}

fn draw_settings(f: &mut Frame, app: &App, area: Rect, list: &mut ListState) {
    let t = app.theme;
    let [themes, info] = Layout::vertical([
        Constraint::Length(ThemeName::ALL.len() as u16 + 2),
        Constraint::Min(0),
    ])
    .areas(area);
    let items: Vec<ListItem> = ThemeName::ALL
        .iter()
        .map(|th| {
            let cur = if *th == app.settings.theme {
                "  (current)"
            } else {
                ""
            };
            ListItem::new(format!("{}{cur}", th.label()))
        })
        .collect();
    draw_list(f, &t, themes, "COLOUR THEME", items, list, "");
    let lines = vec![
        Line::from(vec![
            Span::styled("Security mode: ", t.dim),
            Span::styled(app.mode().label(), t.accent),
        ]),
        Line::from(app.mode().description()),
        Line::from(""),
        Line::from(Span::styled(
            "Your security mode comes from your account, not from a setting, so it cannot be changed \
             here. An administrator can change it; see the handbook page 'Security modes'.",
            t.dim,
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("Settings file: {}", app.settings_path.display()),
            t.dim,
        )),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .style(t.base)
            .wrap(Wrap { trim: true })
            .block(block("ACCOUNT", &t)),
        info,
    );
}

fn draw_search(
    f: &mut Frame,
    app: &App,
    area: Rect,
    query: &str,
    hits: &[SearchHit],
    list: &mut ListState,
) {
    let t = app.theme;
    let [input, results] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(area);
    let cursor = if (app.tick / 2) % 2 == 0 { "█" } else { " " };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(query.to_string(), t.accent),
            Span::styled(cursor, t.base),
        ]))
        .block(block("SEARCH", &t)),
        input,
    );
    let items: Vec<ListItem> = hits
        .iter()
        .map(|h| {
            ListItem::new(match h {
                SearchHit::Category(c) => format!("[MENU] {}", c.title()),
                SearchHit::Entry(e @ Entry::Builtin(_)) => format!("[TOOL] {}", app.entry_name(*e)),
                SearchHit::Entry(e @ Entry::App(_)) => format!("[APP ] {}", app.entry_name(*e)),
                SearchHit::Page(p) => format!("[DOC ] {}", p.title),
            })
        })
        .collect();
    let empty = if query.trim().is_empty() {
        "Type a word: an application, a topic, a setting..."
    } else {
        "No matches."
    };
    draw_list(
        f,
        &t,
        results,
        &format!("RESULTS ({})", hits.len()),
        items,
        list,
        empty,
    );
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_modal(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let Some(m) = &app.modal else { return };
    let (title, mut lines, style) = match m {
        Modal::Message { title, body, error } => {
            let mut l: Vec<Line> = body.lines().map(|s| Line::from(s.to_string())).collect();
            l.push(Line::from(""));
            l.push(Line::from(Span::styled("[ OK ]  (Enter)", t.selected)).centered());
            (title.as_str(), l, if *error { t.error } else { t.base })
        }
        Modal::Confirm {
            title,
            body,
            confirm,
            yes_focused,
            ..
        } => {
            let mut l: Vec<Line> = body.lines().map(|s| Line::from(s.to_string())).collect();
            l.push(Line::from(""));
            let (ys, ns) = if *yes_focused {
                (t.selected, t.base)
            } else {
                (t.base, t.selected)
            };
            l.push(
                Line::from(vec![
                    Span::styled(format!("[ {confirm} (y) ]"), ys),
                    Span::raw("   "),
                    Span::styled("[ Cancel (n) ]", ns),
                ])
                .centered(),
            );
            (title.as_str(), l, t.warn)
        }
        Modal::NewNote { value, error } => {
            let mut l = vec![
                Line::from("Name for the new note:"),
                Line::from(vec![
                    Span::styled(format!("> {value}"), t.accent),
                    Span::styled("█", t.base),
                ]),
                Line::from(""),
            ];
            match error {
                Some(e) => l.push(Line::from(Span::styled(format!("[!!] {e}"), t.error))),
                None => l.push(Line::from(Span::styled(
                    "Letters, digits, spaces, - _ . | Enter create, Esc cancel",
                    t.dim,
                ))),
            }
            ("NEW NOTE", l, t.base)
        }
    };
    let width = 64u16;
    lines = wrap(lines, width as usize - 4);
    let r = centered(area, width, lines.len() as u16 + 2);
    f.render_widget(Clear, r);
    let b = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(style)
        .title(Span::styled(format!(" {title} "), t.title))
        .style(Style::new().patch(t.base));
    f.render_widget(Paragraph::new(lines).style(t.base).block(b), r);
}
