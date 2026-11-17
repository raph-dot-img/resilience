//! The real world overview pack (ADR 0016 §2), drawn as the viewer draws
//! it. Not run by default: it needs the built pack.
//!
//! ```sh
//! nix build .#resilience-world-map
//! RESILIENCE_WORLD_PACK=$(echo result/share/resilience/packs/*.mbtiles) \
//!   cargo test -p resilience-maps --test world -- --ignored --nocapture
//! ```
//!
//! It prints the first screen and a few known places at several scales,
//! to be looked at, and asserts what must hold on any screen size: the
//! attribution, land and sea both drawn, and well-known names.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use resilience_core::config::ThemeName;
use resilience_core::maps::geo::LatLon;
use resilience_maps::app::{MapApp, Start};
use resilience_maps::loader::{Catalog, Worker};
use resilience_maps::ui;

fn screen(
    pack: &Path,
    at: Option<(f64, f64)>,
    scale: Option<f64>,
    w: u16,
    h: u16,
) -> (String, usize) {
    let mut app = MapApp::new(Start {
        packs: vec![pack.to_path_buf()],
        at: at.map(|(lat, lon)| LatLon::wrapped(lat, lon)),
        scale,
        search: None,
        saved: None,
        theme: ThemeName::Green,
        now: Instant::now(),
    });
    let mut worker = Worker::new(Catalog::none());
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    // Open, then draw until no tile is wanted any more.
    for _ in 0..8 {
        for r in worker.handle_all(app.take_requests()) {
            app.on_response(r);
        }
        term.draw(|f| ui::draw(f, &mut app)).expect("draw");
    }
    let buf = term.backend().buffer();
    let blue = buf
        .content()
        .iter()
        .filter(|c| c.fg == Color::Blue || c.bg == Color::Blue)
        .count();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    (out, blue)
}

#[test]
#[ignore = "needs the built world overview pack (RESILIENCE_WORLD_PACK)"]
fn the_world_overview_draws() {
    let pack = PathBuf::from(
        std::env::var_os("RESILIENCE_WORLD_PACK").expect("RESILIENCE_WORLD_PACK=FILE"),
    );
    let (first, blue) = screen(&pack, None, None, 170, 48);
    println!("{first}");
    // The whole world: sea (blue cells) and land (the others).
    let cells = 170 * 48;
    assert!(blue > cells / 4, "sea is drawn ({blue} of {cells} cells)");
    assert!(
        blue < cells * 3 / 4,
        "land is drawn ({blue} of {cells} cells)"
    );
    assert!(first.contains("Natural Earth"), "attribution on screen");
    assert!(first.contains("World overview"));

    for (name, at, scale, wanted) in [
        (
            "Europe",
            (48.0, 8.0),
            5000.0,
            vec!["Paris", "Munich", "Zurich"],
        ),
        ("Luxembourg", (49.61, 6.13), 500.0, vec!["Luxembourg"]),
        ("Japan", (36.0, 138.0), 2000.0, vec!["Tokyo"]),
        ("Brazil, small screen", (-15.8, -47.9), 2000.0, vec!["Bras"]),
    ] {
        let (w, h) = if name.ends_with("small screen") {
            (80, 24)
        } else {
            (170, 48)
        };
        let (s, _) = screen(&pack, Some(at), Some(scale), w, h);
        println!("--- {name}, {scale} m per pixel\n{s}");
        for word in wanted {
            assert!(s.contains(word), "{name}: {word} on screen");
        }
        assert!(s.contains("Natural Earth"));
    }
}
