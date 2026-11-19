//! Drawing the viewer's screens with ratatui (ADR 0016 §3). Like the main
//! interface (`resilience-tui/src/ui.rs`): the 8 basic colours only, the
//! user's theme for everything but the map itself, key hints on the bottom
//! line, a plain notice below 40x12. Every character drawn is printable
//! ASCII or one of the console glyphs: text from map files is folded to
//! ASCII before it gets here (`label::fold_to`).
//!
//! The map screen, from the top:
//! - the map, with the crosshair in its middle cell;
//! - the readout: coordinates of the crosshair (`d`: decimal or degrees,
//!   minutes and seconds), the scale bar, the pack under the crosshair and
//!   the attribution of what is in view, packed into as few lines as the
//!   width allows (the attribution is never left out);
//! - the status line (the most important message, or nothing);
//! - the key hints.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use resilience_core::config::ThemeName;
use resilience_core::maps::canvas::{Canvas, Colour};
use resilience_core::maps::geo::{format_decimal, format_dms, scale_bar};
use resilience_core::maps::label::fold_to;
use resilience_core::maps::style::{self, scale_text};

use crate::app::{Level, MIN_COLS, MIN_ROWS, MapApp, OpenPack, Screen, Slot};
use crate::loader::Integrity;
use crate::render::Role;

/// The sentence ADR 0016 §3 requires on the information screen.
pub const OSM_WARNING: &str = "OpenStreetMap is made by volunteers. Roads, places and names can \
     be missing, wrong or out of date. Something absent from the map may exist.";

/// The map screen's key hints in the order shown, each with its rank:
/// when the line is too narrow, the lowest ranks are left out first, so
/// `q Quit` and `? Help` are always there.
pub const MAP_HINTS: [(&str, u8); 7] = [
    ("Arrows Move", 4),
    ("+/- Zoom", 5),
    ("d Degrees", 1),
    ("i Info", 3),
    ("L Legend", 2),
    ("? Help", 6),
    ("q Quit", 7),
];

/// The hints that fit in `width`, two spaces apart.
pub fn hints(width: usize) -> String {
    let mut keep: Vec<bool> = vec![true; MAP_HINTS.len()];
    let len = |keep: &[bool]| {
        let parts: Vec<&str> = MAP_HINTS
            .iter()
            .zip(keep)
            .filter(|(_, k)| **k)
            .map(|((h, _), _)| *h)
            .collect();
        parts.join("  ")
    };
    let mut ranks: Vec<(u8, usize)> = MAP_HINTS
        .iter()
        .enumerate()
        .map(|(i, (_, r))| (*r, i))
        .collect();
    ranks.sort();
    for (_, i) in ranks {
        if len(&keep).len() <= width {
            break;
        }
        keep[i] = false;
    }
    len(&keep)
}

/// Styles of the interface around the map, from the user's theme (the
/// same choices as the main interface's `theme.rs`).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub base: Style,
    pub title: Style,
    pub accent: Style,
    pub dim: Style,
    pub warn: Style,
    pub error: Style,
}

impl Theme {
    pub fn new(name: ThemeName) -> Theme {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        if name == ThemeName::Mono {
            return Theme {
                base: Style::new(),
                title: bold,
                accent: bold,
                dim: Style::new(),
                warn: bold,
                error: bold.add_modifier(Modifier::UNDERLINED),
            };
        }
        let fg = match name {
            ThemeName::Amber => Color::Yellow,
            ThemeName::Cyan => Color::Cyan,
            _ => Color::Green,
        };
        Theme {
            base: Style::new().fg(fg),
            title: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            accent: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            dim: Style::new().fg(fg).add_modifier(Modifier::DIM),
            warn: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        }
    }
}

/// A canvas colour as one of the 8 basic ANSI colours. White is ANSI
/// colour 7 (`Gray` in ratatui), which the console has as a background
/// too; ratatui's `White` is the bright variant.
pub fn colour(c: Option<Colour>) -> Color {
    match c {
        None => Color::Reset,
        Some(Colour::Black) => Color::Black,
        Some(Colour::Red) => Color::Red,
        Some(Colour::Green) => Color::Green,
        Some(Colour::Yellow) => Color::Yellow,
        Some(Colour::Blue) => Color::Blue,
        Some(Colour::Magenta) => Color::Magenta,
        Some(Colour::Cyan) => Color::Cyan,
        Some(Colour::White) => Color::Gray,
    }
}

/// Copy a canvas into the buffer at `area`'s top left.
fn blit(buf: &mut Buffer, area: Rect, canvas: &Canvas) {
    for row in 0..canvas.rows().min(usize::from(area.height)) {
        for col in 0..canvas.cols().min(usize::from(area.width)) {
            let c = canvas.cell(col, row);
            let mut st = Style::new().fg(colour(c.fg)).bg(colour(c.bg));
            if c.reverse {
                st = st.add_modifier(Modifier::REVERSED);
            }
            let pos = (area.x + col as u16, area.y + row as u16);
            if let Some(cell) = buf.cell_mut(pos) {
                cell.set_char(c.ch).set_style(st);
            }
        }
    }
}

/// Word-wrap ASCII text to `width`, continuation lines indented by
/// `indent` spaces. Words longer than a line are cut.
pub fn wrap(text: &str, width: usize, indent: usize) -> Vec<String> {
    let width = width.max(indent + 8);
    if text.chars().count() <= width {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let mut word = word.to_string();
        loop {
            let room = width - line.chars().count();
            let sep = usize::from(!line.trim().is_empty());
            if word.chars().count() + sep <= room {
                if sep == 1 {
                    line.push(' ');
                }
                line.push_str(&word);
                break;
            }
            if line.trim().is_empty() {
                // The word alone is too long: cut it.
                let cut: String = word.chars().take(room).collect();
                word = word.chars().skip(room).collect();
                line.push_str(&cut);
            }
            out.push(std::mem::take(&mut line));
            line = " ".repeat(indent);
            if word.is_empty() {
                break;
            }
        }
    }
    if !line.trim().is_empty() {
        out.push(line);
    }
    out
}

/// Pack segments into lines of `width`, two spaces apart; a segment wider
/// than a line is cut.
pub fn pack_segments(segments: &[String], width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in segments.iter().filter(|s| !s.is_empty()) {
        let s: String = s.chars().take(width).collect();
        match out.last_mut() {
            Some(l) if l.chars().count() + 2 + s.chars().count() <= width => {
                l.push_str("  ");
                l.push_str(&s);
            }
            _ => out.push(s),
        }
    }
    out
}

/// The scale bar text for the current scale, `└────┘ 500 m`, at most
/// `room` characters wide (empty if no length fits).
pub fn scale_bar_text(scale: f64, room: usize) -> String {
    let max_cells = room.saturating_sub(8).clamp(0, 20) as u32;
    match scale_bar(scale, 8.min(max_cells.max(1)), max_cells.max(1)) {
        Some(b) if b.cells >= 2 => format!("└{}┘ {}", "─".repeat(b.cells as usize - 2), b.label),
        _ => format!("{} per pixel", scale_text(scale)),
    }
}

/// The readout lines (coordinates, scale bar, pack, attribution).
pub fn readout(app: &MapApp, width: u16) -> Vec<String> {
    let w = usize::from(width);
    let c = app.view().center();
    let coords = if app.dms {
        format_dms(c)
    } else {
        format_decimal(c)
    };
    let room = if coords.len() + 2 + 30 <= w {
        30
    } else {
        w.saturating_sub(coords.len() + 2).max(w.min(30))
    };
    let bar = scale_bar_text(app.view().scale(), room);
    let pack = match app.active_pack() {
        Some(p) => format!("Map: {}", p.title),
        None => "No map here".to_string(),
    };
    let credit = match app.attribution() {
        a if a.is_empty() => "No map data in view".to_string(),
        a => a,
    };
    pack_segments(&[coords, bar, pack, credit], w)
}

fn level_style(t: &Theme, l: Level) -> Style {
    match l {
        Level::Info => t.base,
        Level::Warn => t.warn,
        Level::Error => t.error,
    }
}

pub fn draw(f: &mut Frame, app: &mut MapApp) {
    let area = f.area();
    let t = Theme::new(app.theme);
    f.render_widget(Block::new().style(t.base), area);
    if area.width < MIN_COLS || area.height < MIN_ROWS {
        let msg = format!(
            "Terminal too small ({}x{}). The map needs at least {MIN_COLS}x{MIN_ROWS}; 80x24 \
             or more is better. q Quit",
            area.width, area.height
        );
        let lines: Vec<Line> = wrap(&msg, usize::from(area.width.max(1)), 0)
            .into_iter()
            .map(|l| Line::from(Span::styled(l, t.warn)))
            .collect();
        f.render_widget(Paragraph::new(lines), area);
        return;
    }
    match app.screen {
        Screen::Map => {
            if app.slots().is_empty() {
                no_map_page(f, app, &t, area);
            } else if app.nothing_to_show() {
                failures_page(f, app, &t, area);
            } else {
                map_screen(f, app, &t, area);
            }
        }
        Screen::Info { scroll } => {
            let lines = info_lines(app, usize::from(area.width.saturating_sub(4)));
            page(
                f,
                app,
                &t,
                area,
                "MAP INFORMATION",
                lines,
                scroll,
                "Up/Down Scroll  Esc Back",
            );
        }
        Screen::Help { scroll } => {
            let lines = help_lines(usize::from(area.width.saturating_sub(4)));
            page(
                f,
                app,
                &t,
                area,
                "MAP HELP",
                lines,
                scroll,
                "Up/Down Scroll  Esc Back",
            );
        }
        Screen::Search => search_page(f, app, &t, area),
        Screen::Legend { all, scroll } => {
            let (title, lines) = legend_lines(app, all, &t);
            let hint = if all {
                "a This scale only  Up/Down Scroll  Esc Back"
            } else {
                "a All scales  Up/Down Scroll  Esc Back"
            };
            page(f, app, &t, area, &title, lines, scroll, hint);
        }
    }
}

fn map_screen(f: &mut Frame, app: &mut MapApp, t: &Theme, area: Rect) {
    let info = readout(app, area.width);
    let below = info.len() as u16 + 2;
    let map_rows = area.height.saturating_sub(below).max(1);
    let canvas = app.map_canvas(area.width, map_rows);
    blit(
        f.buffer_mut(),
        Rect {
            height: map_rows,
            ..area
        },
        &canvas,
    );
    let mut y = area.y + map_rows;
    for l in info {
        f.render_widget(
            Paragraph::new(Span::styled(l, t.accent)),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
    }
    let status = match app.status() {
        Some((lvl, text)) => {
            let text: String = text.chars().take(usize::from(area.width)).collect();
            Span::styled(text, level_style(t, lvl))
        }
        None => Span::raw(""),
    };
    f.render_widget(Paragraph::new(status), Rect::new(area.x, y, area.width, 1));
    f.render_widget(
        Paragraph::new(Span::styled(hints(usize::from(area.width)), t.accent)),
        Rect::new(area.x, y + 1, area.width, 1),
    );
}

fn block<'a>(title: &str, t: &Theme) -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(t.base)
        .title(Span::styled(format!(" {title} "), t.title))
}

#[allow(clippy::too_many_arguments)]
fn page(
    f: &mut Frame,
    app: &mut MapApp,
    t: &Theme,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    scroll: u16,
    hint: &str,
) {
    let body = Rect {
        height: area.height - 1,
        ..area
    };
    let b = block(title, t);
    let inner = b.inner(body);
    f.render_widget(b, body);
    let max = (lines.len() as u16).saturating_sub(inner.height);
    app.clamp_scroll(max);
    let scroll = scroll.min(max);
    f.render_widget(
        Paragraph::new(lines).style(t.base).scroll((scroll, 0)),
        Rect {
            x: inner.x + 1,
            width: inner.width.saturating_sub(1),
            ..inner
        },
    );
    let more = if max > 0 {
        format!("  ({}/{})", scroll + 1, max + 1)
    } else {
        String::new()
    };
    f.render_widget(
        Paragraph::new(Span::styled(format!("{hint}{more}"), t.accent)),
        Rect::new(area.x, area.y + area.height - 1, area.width, 1),
    );
}

fn text_lines(text: &str, width: usize, indent: usize, st: Style) -> Vec<Line<'static>> {
    wrap(text, width, indent)
        .into_iter()
        .map(|l| Line::from(Span::styled(l, st)))
        .collect()
}

fn no_map_page(f: &mut Frame, app: &mut MapApp, t: &Theme, area: Rect) {
    let w = usize::from(area.width.saturating_sub(4));
    let mut lines = text_lines("No map is installed or open.", w, 0, t.warn);
    lines.push(Line::raw(""));
    lines.extend(text_lines(
        "To get a map: in the Resilience menu, open Maps & navigation, then Map packs, and \
         install a map pack from a USB stick or a download (installing asks for an \
         administrator's password). A map file on a USB stick opens from Maps & navigation, \
         Maps on a USB stick.",
        w,
        0,
        t.base,
    ));
    page(f, app, t, area, "MAPS", lines, 0, "? Help  q Quit");
}

fn failures_page(f: &mut Frame, app: &mut MapApp, t: &Theme, area: Rect) {
    let w = usize::from(area.width.saturating_sub(4));
    let mut lines = text_lines("The map cannot be shown.", w, 0, t.error);
    lines.push(Line::raw(""));
    for s in app.slots() {
        let (name, msg) = match s {
            Slot::Failed { path, message } => (display_path(path), message.clone()),
            Slot::Open(p) => match &p.summary.integrity {
                Integrity::Damaged(m) => (p.title.clone(), m.clone()),
                _ => continue,
            },
            Slot::Opening { .. } => continue,
        };
        lines.extend(text_lines(
            &format!("{name}: {}", fold(&msg, 400)),
            w,
            2,
            t.base,
        ));
    }
    lines.push(Line::raw(""));
    lines.extend(text_lines(
        "If the file came from a USB stick, copy it again or ask for a new copy. A map \
         installed from Map packs can be installed again there.",
        w,
        0,
        t.base,
    ));
    let scroll = match app.screen {
        Screen::Info { scroll } => scroll,
        _ => 0,
    };
    page(
        f,
        app,
        t,
        area,
        "MAPS",
        lines,
        scroll,
        "i Information  q Quit",
    );
}

/// The search screen (`/`): the words, what happened, the results.
fn search_page(f: &mut Frame, app: &mut MapApp, t: &Theme, area: Rect) {
    use crate::app::FindStatus;
    use resilience_core::maps::search::{Kind, where_from};
    let w = usize::from(area.width.saturating_sub(4)).max(20);
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled("Find: ", t.title),
            Span::styled(fold(&app.find.input, 80), t.accent),
            Span::styled("_", t.base),
        ]),
        Line::raw(""),
    ];
    let status = match &app.find.status {
        FindStatus::Typing => "Type a place, a street or a point of interest, then Enter. \
             Only the open maps are searched."
            .to_string(),
        FindStatus::Searching { .. } => "Searching...".into(),
        FindStatus::Done {
            searched, failed, ..
        } => {
            let names = if searched.is_empty() {
                "no open map can be searched".to_string()
            } else {
                searched.join(", ")
            };
            let mut s = if app.find.results.is_empty() {
                format!("Nothing found in: {names}. Places outside these maps cannot be found.")
            } else {
                format!(
                    "{} found in: {names}. Enter goes there.",
                    app.find.results.len()
                )
            };
            if !failed.is_empty() {
                s.push_str(&format!(" Could not be searched: {}.", failed.join(", ")));
            }
            s
        }
    };
    lines.extend(text_lines(&status, w, 0, t.dim));
    lines.push(Line::raw(""));
    let centre = app.view().center();
    let first = lines.len();
    for (i, h) in app.find.results.iter().enumerate() {
        let kind = match h.kind {
            Kind::Settlement | Kind::Natural => fold(&h.class, 20),
            Kind::Street => "street".into(),
            Kind::Poi => fold(&h.class, 20).replace('_', " "),
        };
        let near = h
            .near
            .as_deref()
            .and_then(|n| fold_to(n, 30))
            .map(|n| format!(", near {n}"))
            .unwrap_or_default();
        let text = format!(
            "{} ({kind}{near})  {}",
            fold(&h.name, 40),
            where_from(centre, h.at)
        );
        let text: String = text.chars().take(w).collect();
        let style = if i == app.find.sel {
            t.accent.add_modifier(Modifier::REVERSED)
        } else {
            t.base
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    // Keep the chosen result in sight.
    let visible = usize::from(area.height.saturating_sub(4));
    let want = first + app.find.sel;
    let scroll = want.saturating_sub(visible.saturating_sub(1)) as u16;
    page(
        f,
        app,
        t,
        area,
        "FIND A PLACE",
        lines,
        scroll,
        "Type  Enter Search / Go there  Up/Down Choose  Esc Back",
    );
}

/// Fold any text for the console, or say it could not be shown.
fn fold(s: &str, max: usize) -> String {
    fold_to(s, max).unwrap_or_else(|| "(cannot be shown)".into())
}

fn display_path(p: &std::path::Path) -> String {
    fold(&p.to_string_lossy(), 200)
}

fn integrity_text(i: &Integrity) -> String {
    match i {
        Integrity::Catalog => "checked against the catalog of this system".into(),
        Integrity::Passed => {
            "not checked against the catalog (not installed from it); the file passed its \
             integrity check"
                .into()
        }
        Integrity::NotChecked => {
            "not checked: not from the catalog of this system, and the file was not checked \
             (press c on the map to check it)"
                .into()
        }
        Integrity::Checking => "not checked against the catalog; checking the file now...".into(),
        Integrity::Damaged(m) => format!("DAMAGED, not shown: {}", fold(m, 300)),
    }
}

fn area_text(p: &OpenPack) -> String {
    let b = p.summary.metadata.bounds;
    let rect = format!(
        "{:.2} to {:.2} E, {:.2} to {:.2} N",
        b.west, b.east, b.south, b.north
    );
    match p.role {
        Role::Overview => "the whole world, at overview detail only".into(),
        Role::Detailed => {
            let rings = p.summary.coverage.as_ref().map_or(0, |c| c.rings.len());
            format!("stated by the file ({rings} outline(s)); outside it the map is shaded")
        }
        Role::Approximate if p.summary.metadata.bounds_given => format!(
            "not stated by the file: drawn within its outer limits ({rect}), and it may be \
             incomplete anywhere inside them"
        ),
        Role::Approximate => {
            "not stated by the file, nor its limits: it never counts as a detailed map".into()
        }
    }
}

/// The information screen: every pack, then the OpenStreetMap warning.
pub fn info_lines(app: &MapApp, width: usize) -> Vec<Line<'static>> {
    let t = Theme::new(app.theme);
    let mut out = Vec::new();
    let field = |out: &mut Vec<Line<'static>>, k: &str, v: &str| {
        for (i, l) in wrap(v, width.saturating_sub(14), 0).into_iter().enumerate() {
            let k = if i == 0 { k } else { "" };
            out.push(Line::from(Span::styled(format!("{k:<13} {l}"), t.base)));
        }
    };
    for (n, s) in app.slots().iter().enumerate() {
        match s {
            Slot::Open(p) => {
                out.push(Line::from(Span::styled(
                    format!("{}. {}", n + 1, p.title),
                    t.title,
                )));
                let m = &p.summary.metadata;
                field(&mut out, "Checked:", &integrity_text(&p.summary.integrity));
                field(
                    &mut out,
                    "Covers:",
                    &m.coverage
                        .as_deref()
                        .map_or_else(|| "not stated".into(), |c| fold(c, 1000)),
                );
                field(&mut out, "Area:", &area_text(p));
                if let Some(problem) = &p.summary.coverage_problem {
                    field(
                        &mut out,
                        "",
                        &format!("(its coverage table is unusable: {})", fold(problem, 200)),
                    );
                }
                field(
                    &mut out,
                    "Data date:",
                    &m.data_date
                        .as_deref()
                        .map_or_else(|| "unknown".into(), |d| fold(d, 40)),
                );
                field(
                    &mut out,
                    "Source:",
                    &m.source
                        .as_deref()
                        .map_or_else(|| "unknown".into(), |s| fold(s, 400)),
                );
                field(
                    &mut out,
                    "Licence:",
                    &m.license
                        .as_deref()
                        .map_or_else(|| "not stated".into(), |l| fold(l, 100)),
                );
                field(&mut out, "Attribution:", &p.attribution);
                field(
                    &mut out,
                    "Detail:",
                    &format!(
                        "{}, tile zooms {} to {}",
                        match p.role {
                            Role::Overview => "world overview",
                            Role::Detailed => "detailed map",
                            Role::Approximate => "map without a stated area",
                        },
                        m.zoom.min(),
                        m.zoom.max()
                    ),
                );
                if !MapApp::is_ours(p) {
                    field(
                        &mut out,
                        "Note:",
                        "not a Resilience map pack: only data in the Resilience layout is drawn, \
                         and there is no search",
                    );
                }
                field(&mut out, "File:", &display_path(&p.summary.path));
            }
            Slot::Failed { path, message } => {
                out.push(Line::from(Span::styled(
                    format!("{}. {}", n + 1, display_path(path)),
                    t.title,
                )));
                field(&mut out, "Not shown:", &fold(message, 400));
            }
            Slot::Opening { path } => {
                out.push(Line::from(Span::styled(
                    format!("{}. {}", n + 1, display_path(path)),
                    t.title,
                )));
                field(&mut out, "", "opening...");
            }
        }
        out.push(Line::raw(""));
    }
    if app.slots().is_empty() {
        out.extend(text_lines("No map is open.", width, 0, t.base));
        out.push(Line::raw(""));
    }
    out.extend(text_lines(OSM_WARNING, width, 0, t.warn));
    out
}

/// The legend screen's title and lines.
pub fn legend_lines(app: &MapApp, all: bool, t: &Theme) -> (String, Vec<Line<'static>>) {
    let scale = app.view().scale();
    let entries = style::legend(if all { None } else { Some(scale) });
    let title = if all {
        "LEGEND, ALL SCALES".to_string()
    } else {
        format!("LEGEND AT {} PER PIXEL", scale_text(scale).to_uppercase())
    };
    let lines = entries
        .into_iter()
        .map(|e| {
            let mut spans: Vec<Span<'static>> = e
                .sample
                .iter()
                .map(|c| {
                    let mut st = Style::new().fg(colour(c.fg)).bg(colour(c.bg));
                    if c.reverse {
                        st = st.add_modifier(Modifier::REVERSED);
                    }
                    Span::styled(c.ch.to_string(), st)
                })
                .collect();
            spans.push(Span::styled(format!("  {}", e.text), t.base));
            if all {
                spans.push(Span::styled(format!(" ({})", e.scales_text()), t.dim));
            }
            Line::from(spans)
        })
        .collect();
    (title, lines)
}

/// The help screen.
pub fn help_lines(width: usize) -> Vec<Line<'static>> {
    const KEYS: &[&str] = &[
        "== MOVING ==",
        "  Arrows or h j k l   Move by a quarter of the screen",
        "  Shift + arrows      Move by one character",
        "  + and -             Zoom in and out",
        "",
        "== READING THE MAP ==",
        "  The + in the middle is the crosshair: the line below the map gives its",
        "  coordinates, then the scale bar (true at the crosshair), the map in use",
        "  there and who made the map data.",
        "  /                   Find a place, a street or a point of interest in the",
        "                      open maps (Enter goes to the chosen result)",
        "  d                   Coordinates in degrees, minutes and seconds, or decimal",
        "  L                   Legend: what the colours and signs mean (a: all scales)",
        "  i                   Information about each map: what it covers, how old the",
        "                      data is, its licence and whether the file was checked",
        "  c                   Check a large map file that was not checked yet",
        "  ? or F1             This help",
        "  q or Esc            Leave the map (it opens here again next time)",
        "",
        "== WHEN PARTS ARE MISSING ==",
        "  Shaded areas have no detailed map: only the world overview is shown there.",
        "  At large scales a dotted line shows where a detailed map ends.",
    ];
    let t = Theme::new(ThemeName::Mono);
    let mut out = Vec::new();
    for k in KEYS {
        out.extend(text_lines(k, width, 22, t.base));
    }
    out.push(Line::raw(""));
    out.extend(text_lines(OSM_WARNING, width, 0, t.base));
    out
}
