//! Colour themes.
//!
//! Only the 8 basic ANSI colours are used so every theme renders correctly on
//! the Linux virtual console (which has no 256-colour or RGB support). Every
//! state that matters also carries a text marker (`[OK]`, `[!!]`, `[--]`), so
//! the Monochrome theme loses no information.

use ratatui::style::{Color, Modifier, Style};
use resilience_core::config::ThemeName;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub base: Style,
    pub border: Style,
    pub title: Style,
    pub accent: Style,
    pub dim: Style,
    pub selected: Style,
    pub ok: Style,
    pub warn: Style,
    pub error: Style,
}

impl Theme {
    pub fn new(name: ThemeName) -> Theme {
        let mono = |m: Modifier| Style::new().add_modifier(m);
        match name {
            ThemeName::Mono => Theme {
                base: Style::new(),
                border: Style::new(),
                title: mono(Modifier::BOLD),
                accent: mono(Modifier::BOLD),
                dim: Style::new(),
                selected: mono(Modifier::REVERSED | Modifier::BOLD),
                ok: Style::new(),
                warn: mono(Modifier::BOLD),
                error: mono(Modifier::BOLD | Modifier::UNDERLINED),
            },
            _ => {
                let (fg, accent) = match name {
                    ThemeName::Amber => (Color::Yellow, Color::White),
                    ThemeName::Cyan => (Color::Cyan, Color::White),
                    _ => (Color::Green, Color::White),
                };
                Theme {
                    base: Style::new().fg(fg),
                    border: Style::new().fg(fg),
                    title: Style::new().fg(accent).add_modifier(Modifier::BOLD),
                    accent: Style::new().fg(accent).add_modifier(Modifier::BOLD),
                    dim: Style::new().fg(fg).add_modifier(Modifier::DIM),
                    selected: Style::new()
                        .fg(Color::Black)
                        .bg(fg)
                        .add_modifier(Modifier::BOLD),
                    ok: Style::new().fg(fg).add_modifier(Modifier::BOLD),
                    warn: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    error: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
                }
            }
        }
    }
}
