//! The `view` command line (ADR 0016 §7), parsed strictly before anything
//! is opened:
//!
//! ```text
//! resilience-maps view [--pack FILE]... [--at LAT,LON] [--scale M] [--search WORDS]
//! ```
//!
//! - `--pack FILE`: an absolute path to a `.mbtiles` file, at most
//!   [`MAX_PACKS`] of them, none given twice. Nothing else is checked here:
//!   the reader checks the file itself.
//! - `--at LAT,LON`: any form [`parse_coordinates`] accepts (decimal or
//!   degrees-minutes-seconds), as one argument.
//! - `--scale M`: metres per pixel, a number within the zoom ladder's range;
//!   it is snapped to the nearest step of the ladder.
//! - `--search WORDS`: kept for the search of a later slice (ADR §9 item
//!   10); at most [`MAX_SEARCH`] characters, no control characters.
//!
//! Each option at most once except `--pack`. Arguments are never passed to
//! a shell or another program.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use resilience_core::maps::geo::{
    CoordError, LatLon, MAX_SCALE, MIN_SCALE, ZOOM_LADDER, parse_coordinates,
};

/// At most this many packs are open at once (ADR §3).
pub const MAX_PACKS: usize = 8;
/// Longest `--search` text, in characters.
pub const MAX_SEARCH: usize = 200;
/// Longest pack path accepted, in bytes (PATH_MAX).
pub const MAX_PATH: usize = 4096;

/// What `view` was asked to show.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewArgs {
    pub packs: Vec<PathBuf>,
    pub at: Option<LatLon>,
    /// A step of [`ZOOM_LADDER`].
    pub scale: Option<f64>,
    /// For the later search slice; unused by this viewer yet.
    pub search: Option<String>,
}

/// Why the command line was refused. `Display` is a sentence for people.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgError {
    Unknown(String),
    MissingValue(&'static str),
    Repeated(&'static str),
    TooManyPacks,
    NotAbsolute(String),
    NotMbtiles(String),
    PathTooLong,
    SamePackTwice(String),
    Coordinates(CoordError),
    Scale(String),
    Search(&'static str),
    NotUnicode(&'static str),
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgError::Unknown(a) => write!(f, "unknown argument \"{a}\""),
            ArgError::MissingValue(o) => write!(f, "{o} needs a value"),
            ArgError::Repeated(o) => write!(f, "{o} is given more than once"),
            ArgError::TooManyPacks => write!(f, "at most {MAX_PACKS} map packs can be open"),
            ArgError::NotAbsolute(p) => write!(f, "{p}: the path of a map pack must be absolute"),
            ArgError::NotMbtiles(p) => write!(f, "{p}: a map pack's name ends in .mbtiles"),
            ArgError::PathTooLong => write!(f, "the path of a map pack is too long"),
            ArgError::SamePackTwice(p) => write!(f, "{p} is given twice"),
            ArgError::Coordinates(e) => write!(f, "--at: {e}"),
            ArgError::Scale(v) => write!(
                f,
                "--scale \"{v}\": give metres per pixel between {MIN_SCALE} and {MAX_SCALE}"
            ),
            ArgError::Search(why) => write!(f, "--search: {why}"),
            ArgError::NotUnicode(o) => write!(f, "the value of {o} is not valid text"),
        }
    }
}

impl std::error::Error for ArgError {}

/// A path or argument shown in a message: cleaned and cut.
fn shown(s: &OsStr) -> String {
    crate::text::clean_short(&s.to_string_lossy(), 120)
}

fn utf8<'a>(v: &'a OsStr, option: &'static str) -> Result<&'a str, ArgError> {
    v.to_str().ok_or(ArgError::NotUnicode(option))
}

/// The ladder step nearest to `m` (on a log scale).
pub fn snap_scale(m: f64) -> f64 {
    let mut best = ZOOM_LADDER[0];
    for step in ZOOM_LADDER {
        if (step.ln() - m.ln()).abs() < (best.ln() - m.ln()).abs() {
            best = step;
        }
    }
    best
}

fn parse_scale(v: &str) -> Result<f64, ArgError> {
    let ok = |c: char| c.is_ascii_digit() || c == '.';
    if v.is_empty() || v.len() > 12 || !v.chars().all(ok) {
        return Err(ArgError::Scale(crate::text::clean_short(v, 40)));
    }
    match v.parse::<f64>() {
        Ok(m) if m.is_finite() && (MIN_SCALE..=MAX_SCALE).contains(&m) => Ok(snap_scale(m)),
        _ => Err(ArgError::Scale(crate::text::clean_short(v, 40))),
    }
}

fn check_search(v: &str) -> Result<String, ArgError> {
    if v.chars()
        .any(|c| c.is_control() || crate::text::is_unsafe(c))
    {
        return Err(ArgError::Search("control characters are not allowed"));
    }
    if v.chars().count() > MAX_SEARCH {
        return Err(ArgError::Search("the text is too long"));
    }
    let t = v.trim();
    if t.is_empty() {
        return Err(ArgError::Search("the text is empty"));
    }
    Ok(t.to_string())
}

fn check_pack(v: &OsStr) -> Result<PathBuf, ArgError> {
    if v.len() > MAX_PATH {
        return Err(ArgError::PathTooLong);
    }
    let p = PathBuf::from(v);
    if !p.is_absolute() {
        return Err(ArgError::NotAbsolute(shown(v)));
    }
    let name = p.file_name().map(OsStr::as_bytes).unwrap_or(b"");
    let lower = name.to_ascii_lowercase();
    // A name that is only the extension (".mbtiles") is not a pack name.
    if name.len() <= b".mbtiles".len() || !lower.ends_with(b".mbtiles") {
        return Err(ArgError::NotMbtiles(shown(v)));
    }
    Ok(p)
}

/// Parse the arguments after `view`.
pub fn parse_view(args: &[OsString]) -> Result<ViewArgs, ArgError> {
    let mut out = ViewArgs {
        packs: Vec::new(),
        at: None,
        scale: None,
        search: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let option: &'static str = match a.as_bytes() {
            b"--pack" => "--pack",
            b"--at" => "--at",
            b"--scale" => "--scale",
            b"--search" => "--search",
            _ => return Err(ArgError::Unknown(shown(a))),
        };
        let v = it.next().ok_or(ArgError::MissingValue(option))?;
        match option {
            "--pack" => {
                let p = check_pack(v)?;
                if out.packs.contains(&p) {
                    return Err(ArgError::SamePackTwice(shown(v)));
                }
                if out.packs.len() == MAX_PACKS {
                    return Err(ArgError::TooManyPacks);
                }
                out.packs.push(p);
            }
            "--at" => {
                if out.at.is_some() {
                    return Err(ArgError::Repeated(option));
                }
                let s = utf8(v, option)?;
                out.at = Some(parse_coordinates(s).map_err(ArgError::Coordinates)?);
            }
            "--scale" => {
                if out.scale.is_some() {
                    return Err(ArgError::Repeated(option));
                }
                out.scale = Some(parse_scale(utf8(v, option)?)?);
            }
            _ => {
                if out.search.is_some() {
                    return Err(ArgError::Repeated(option));
                }
                out.search = Some(check_search(utf8(v, option)?)?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn ok(v: &[&str]) -> ViewArgs {
        parse_view(&args(v)).unwrap_or_else(|e| panic!("{v:?}: {e}"))
    }

    fn err(v: &[&str]) -> ArgError {
        match parse_view(&args(v)) {
            Ok(a) => panic!("{v:?} accepted: {a:?}"),
            Err(e) => e,
        }
    }

    #[test]
    fn accepted_command_lines() {
        let a = ok(&[]);
        assert!(a.packs.is_empty() && a.at.is_none() && a.scale.is_none());
        let a = ok(&[
            "--pack",
            "/run/media/ann/STICK/map-osm-luxembourg_2026-01.mbtiles",
            "--pack",
            "/var/lib/resilience/packs/World.MBTILES",
            "--at",
            "49.6116, 6.1319",
            "--scale",
            "50",
        ]);
        assert_eq!(a.packs.len(), 2);
        let at = a.at.expect("at");
        assert!((at.lat() - 49.6116).abs() < 1e-9 && (at.lon() - 6.1319).abs() < 1e-9);
        assert_eq!(a.scale, Some(50.0));
        let a = ok(&["--at", "49d36'41.8\" N 6d07'54.8\" E"]);
        assert!((a.at.expect("dms").lat() - 49.61161).abs() < 1e-4);
        // Scales snap to the nearest ladder step.
        for (given, step) in [
            ("5", 5.0),
            ("6", 5.0),
            ("8", 10.0),
            ("70", 50.0),
            ("75", 100.0),
            ("200000", 200_000.0),
            ("0.5e1", 0.0),
        ] {
            if step == 0.0 {
                assert!(matches!(err(&["--scale", given]), ArgError::Scale(_)));
            } else {
                assert_eq!(ok(&["--scale", given]).scale, Some(step), "{given}");
            }
        }
        assert_eq!(
            ok(&["--search", "  rue de l'essai "]).search.as_deref(),
            Some("rue de l'essai")
        );
        let eight: Vec<String> = (0..8).map(|i| format!("/m/p{i}.mbtiles")).collect();
        let mut v = Vec::new();
        for p in &eight {
            v.push("--pack");
            v.push(p.as_str());
        }
        assert_eq!(ok(&v).packs.len(), 8);
    }

    #[test]
    fn refused_command_lines() {
        let nine: Vec<String> = (0..9).map(|i| format!("/m/p{i}.mbtiles")).collect();
        let mut v = Vec::new();
        for p in &nine {
            v.push("--pack");
            v.push(p.as_str());
        }
        assert_eq!(err(&v), ArgError::TooManyPacks);
        type Case = (&'static [&'static str], fn(&ArgError) -> bool);
        let cases: &[Case] = &[
            (&["--pack"], |e| {
                matches!(e, ArgError::MissingValue("--pack"))
            }),
            (&["--pack", "map.mbtiles"], |e| {
                matches!(e, ArgError::NotAbsolute(_))
            }),
            (&["--pack", "./map.mbtiles"], |e| {
                matches!(e, ArgError::NotAbsolute(_))
            }),
            (&["--pack", "/m/map.zim"], |e| {
                matches!(e, ArgError::NotMbtiles(_))
            }),
            (&["--pack", "/m/.mbtiles"], |e| {
                matches!(e, ArgError::NotMbtiles(_))
            }),
            (&["--pack", "/m/map.mbtiles-wal"], |e| {
                matches!(e, ArgError::NotMbtiles(_))
            }),
            (&["--pack", "/"], |e| matches!(e, ArgError::NotMbtiles(_))),
            (&["--pack", "/m/a.mbtiles", "--pack", "/m/a.mbtiles"], |e| {
                matches!(e, ArgError::SamePackTwice(_))
            }),
            (&["--at"], |e| matches!(e, ArgError::MissingValue("--at"))),
            (&["--at", "91,0"], |e| {
                matches!(e, ArgError::Coordinates(CoordError::LatitudeOutOfRange))
            }),
            (&["--at", "nowhere"], |e| {
                matches!(e, ArgError::Coordinates(_))
            }),
            (&["--at", "1,2", "--at", "3,4"], |e| {
                matches!(e, ArgError::Repeated("--at"))
            }),
            (&["--scale", "4"], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", "200001"], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", "-5"], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", "inf"], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", "NaN"], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", ""], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", "1..2"], |e| matches!(e, ArgError::Scale(_))),
            (&["--scale", "50", "--scale", "50"], |e| {
                matches!(e, ArgError::Repeated("--scale"))
            }),
            (&["--search", ""], |e| matches!(e, ArgError::Search(_))),
            (&["--search", "a\x1b[2Jb"], |e| {
                matches!(e, ArgError::Search(_))
            }),
            (&["--search", "a\u{202e}b"], |e| {
                matches!(e, ArgError::Search(_))
            }),
            (&["--pack=/m/a.mbtiles"], |e| {
                matches!(e, ArgError::Unknown(_))
            }),
            (&["-p", "/m/a.mbtiles"], |e| {
                matches!(e, ArgError::Unknown(_))
            }),
            (&["/m/a.mbtiles"], |e| matches!(e, ArgError::Unknown(_))),
            (&["--", "--pack"], |e| matches!(e, ArgError::Unknown(_))),
            (&["--help"], |e| matches!(e, ArgError::Unknown(_))),
        ];
        for (v, check) in cases {
            let e = err(v);
            assert!(check(&e), "{v:?} gave {e:?}");
            // Every message is one printable line.
            assert!(!e.to_string().chars().any(char::is_control), "{e}");
        }
        let long = "x".repeat(MAX_SEARCH + 1);
        assert!(matches!(err(&["--search", &long]), ArgError::Search(_)));
        let path = format!("/{}.mbtiles", "a".repeat(MAX_PATH));
        assert_eq!(err(&["--pack", &path]), ArgError::PathTooLong);
    }

    #[test]
    fn non_utf8_paths_are_accepted_but_not_values() {
        let p = OsString::from_vec(b"/m/caf\xe9.mbtiles".to_vec());
        let a = parse_view(&[OsString::from("--pack"), p.clone()]).expect("path bytes");
        assert_eq!(a.packs, vec![PathBuf::from(p)]);
        let bad = OsString::from_vec(b"49\xff,6".to_vec());
        assert_eq!(
            parse_view(&[OsString::from("--at"), bad]),
            Err(ArgError::NotUnicode("--at"))
        );
        // An escape sequence in a refused path does not reach the message.
        let e = err(&["--pack", "rel\x1b[2J.mbtiles"]);
        assert!(!e.to_string().contains('\x1b'), "{e}");
    }
}
