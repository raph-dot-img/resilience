//! Decode and draw time of a dense view (ADR 0016 §9 item 6: "measure
//! decode and draw time"). Not run by default (timings depend on the
//! machine and the build profile):
//!
//! ```sh
//! cargo test --release -p resilience-maps --test perf -- --ignored --nocapture
//! ```
//!
//! Four tiles of zoom 14 meet at the centre of a 170x48 screen at 5 m per
//! pixel, each about 400 KiB of MVT with tens of thousands of line
//! vertices: what the ADR's Luxembourg experiment saw for a full screen,
//! four times over.

#[path = "../src/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

use std::time::{Duration, Instant};

use fixtures::{Attr, PackSpec, TempDir, TileMaker};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use resilience_core::config::ThemeName;
use resilience_core::maps::geo::{LatLon, TileId, World, unproject};
use resilience_core::maps::mvt;
use resilience_maps::app::{MapApp, Start};
use resilience_maps::loader::{Catalog, Worker};

/// A small deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> i32 {
        (self.next() % n) as i32
    }
}

const ROADS: [&str; 6] = [
    "residential",
    "residential",
    "service",
    "path",
    "tertiary",
    "primary",
];

fn dense_tile(id: TileId, seed: u64) -> TileMaker {
    let mut r = Rng(seed | 1);
    let mut t = TileMaker::new(id);
    // Streets: random walks of 12-20 vertices.
    for i in 0..4600 {
        let mut x = r.below(4096);
        let mut y = r.below(4096);
        let n = 12 + r.below(9);
        let mut pts = Vec::new();
        for _ in 0..n {
            pts.push((x, y));
            x = (x + r.below(161) - 80).clamp(-64, 4160);
            y = (y + r.below(161) - 80).clamp(-64, 4160);
        }
        let class = ROADS[i % ROADS.len()];
        let name = format!("Rue numero {i}");
        t.line_xy(
            "roads",
            &[("class", Attr::S(class)), ("name", Attr::S(&name))],
            &pts,
        );
    }
    for _ in 0..100 {
        let (x, y) = (r.below(4096), r.below(4096));
        t.line_xy(
            "rail",
            &[("class", Attr::S("rail"))],
            &[(x, y), (x + 300, y + 40), (x + 600, y + 120)],
        );
    }
    // Woods and lakes: 24-gons.
    for i in 0..400 {
        let (cx, cy) = (r.below(4096), r.below(4096));
        let rad = 20 + r.below(120);
        let ring: Vec<(i32, i32)> = (0..24)
            .map(|k| {
                let a = f64::from(k) / 24.0 * std::f64::consts::TAU;
                let wob = f64::from(rad + r.below(20));
                (cx + (a.cos() * wob) as i32, cy + (a.sin() * wob) as i32)
            })
            .collect();
        let (layer, class) = if i % 3 == 0 {
            ("water", "lake")
        } else {
            ("land", "forest")
        };
        t.polygon_xy(layer, &[("class", Attr::S(class))], &ring);
    }
    for i in 0..800 {
        let (x, y) = (r.below(4096), r.below(4096));
        let name = format!("Lieu {i}");
        t.raw(
            "pois",
            1,
            &[("category", Attr::S("food")), ("name", Attr::S(&name))],
            &resilience_core::maps::mvt_encode::points(&[(x, y)]),
        );
    }
    t
}

fn vm_hwm_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

#[test]
#[ignore = "timing measurement, run on demand in release mode"]
fn dense_view_decode_and_draw_times() {
    let dir = TempDir::new("perf");
    let path = dir.file("dense.mbtiles");
    // The four z14 tiles around a corner near Luxembourg.
    let corner = TileId::containing(LatLon::wrapped(49.6116, 6.1319), 14);
    let ids: Vec<TileId> = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .iter()
        .filter_map(|(dx, dy)| TileId::new(14, corner.x() + dx, corner.y() + dy))
        .collect();
    let mut spec = PackSpec::detailed("Dense", (5.7, 49.4, 6.6, 50.2));
    let mut sizes = Vec::new();
    let mut vertices = Vec::new();
    for (n, id) in ids.iter().enumerate() {
        let t = dense_tile(*id, 0x9E37_79B9_7F4A_7C15 ^ n as u64);
        let bytes = t.encode();
        let decoded = mvt::decode(&bytes).expect("decodes");
        vertices.push(
            decoded
                .layers()
                .iter()
                .map(|l| l.vertex_count())
                .sum::<usize>(),
        );
        sizes.push(bytes.len());
        spec = spec.tile(t);
    }
    // Zooms 13 and 12 around the corner too, for 10 and 20 m per pixel.
    for z in [13u8, 12] {
        let c = TileId::containing(LatLon::wrapped(49.6116, 6.1319), z);
        for dy in 0..2 {
            for dx in 0..2 {
                if let Some(id) = TileId::new(z, c.x() + dx, c.y() + dy) {
                    spec = spec.tile(dense_tile(id, u64::from(z) * 31 + u64::from(dx * 2 + dy)));
                }
            }
        }
    }
    spec.write(&path);
    // RESILIENCE_PERF_KEEP=FILE keeps a copy, e.g. to run the real viewer
    // on it in a terminal and read its memory use from /proc.
    if let Some(keep) = std::env::var_os("RESILIENCE_PERF_KEEP") {
        std::fs::copy(&path, keep).expect("keep a copy");
    }
    println!(
        "tiles: {} KiB of MVT each (decompressed)",
        sizes
            .iter()
            .map(|s| s / 1024)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("vertices per tile: {vertices:?}");

    // Decoding alone.
    let raw: Vec<Vec<u8>> = spec.tiles.iter().take(4).map(|(_, b)| b.clone()).collect();
    let t0 = Instant::now();
    let rounds = 20;
    for _ in 0..rounds {
        for b in &raw {
            std::hint::black_box(mvt::decode(b).expect("decodes"));
        }
    }
    println!(
        "decode only: {:.1} ms per tile",
        t0.elapsed().as_secs_f64() * 1e3 / f64::from(rounds * 4)
    );

    // The shared corner at the centre of the screen.
    let n = f64::from(1u32 << 14);
    let at = unproject(World {
        x: f64::from(corner.x() + 1) / n,
        y: f64::from(corner.y() + 1) / n,
    });
    for scale in [5.0, 10.0, 20.0] {
        let start = Instant::now();
        let mut app = MapApp::new(Start {
            packs: vec![path.clone()],
            at: Some(at),
            scale: Some(scale),
            search: None,
            saved: None,
            theme: ThemeName::Green,
            now: start,
        });
        // The catalog-checked path: no quick_check in the measurement.
        let mut worker = Worker::new(Catalog::none());
        for r in worker.handle_all(app.take_requests()) {
            app.on_response(r);
        }
        let opened = start.elapsed();
        let t_cold = Instant::now();
        let _ = app.map_canvas(170, 45);
        let reqs = app.take_requests();
        let t_read = Instant::now();
        for r in worker.handle_all(reqs) {
            app.on_response(r);
        }
        let read = t_read.elapsed();
        let canvas = app.map_canvas(170, 45);
        let cold = t_cold.elapsed();
        std::hint::black_box(canvas);
        // Warm: pan one cell at a time, tiles cached.
        let frames = 30;
        let t_warm = Instant::now();
        let mut worst = Duration::ZERO;
        for i in 0..frames {
            let code = if i % 2 == 0 {
                KeyCode::Right
            } else {
                KeyCode::Left
            };
            app.handle_key(KeyEvent::new(code, KeyModifiers::SHIFT));
            let f0 = Instant::now();
            std::hint::black_box(app.map_canvas(170, 45));
            worst = worst.max(f0.elapsed());
        }
        let warm = t_warm.elapsed() / frames;
        assert!(app.take_requests().is_empty(), "warm frames read nothing");
        println!(
            "{scale} m/px: open+check {:.0} ms; cold frame {:.1} ms (of which read+inflate+decode {:.1} ms, {} tiles cached, {} KiB); warm frame {:.1} ms (worst {:.1} ms)",
            opened.as_secs_f64() * 1e3,
            cold.as_secs_f64() * 1e3,
            read.as_secs_f64() * 1e3,
            app.cache().len(),
            app.cache().bytes() / 1024,
            warm.as_secs_f64() * 1e3,
            worst.as_secs_f64() * 1e3,
        );
    }
    println!(
        "peak memory of this test process (VmHWM): {} MiB",
        vm_hwm_kib() / 1024
    );
}
