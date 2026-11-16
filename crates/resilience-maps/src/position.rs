//! The last map position (ADR 0016 §7): `~/.local/state/resilience/maps.toml`
//! (`$XDG_STATE_HOME` if set), holding the centre and the scale and nothing
//! else (no history).
//!
//! ```toml
//! # Last map position of resilience-maps
//! lat = 49.611600
//! lon = 6.131900
//! scale = 50
//! ```
//!
//! - **Reading** ignores anything unexpected: a file that is not a regular
//!   file (a symlink is not followed), larger than [`MAX_FILE`], not UTF-8,
//!   with an unknown or repeated key, a missing key, a value that is not a
//!   plain number or out of range. The viewer then starts as if there were
//!   no file. Nothing from the file is ever printed.
//! - **Writing** is atomic: a new file `.maps.toml.PID.tmp` created with
//!   `O_EXCL | O_NOFOLLOW` and mode 0600, written, synced, then renamed
//!   over `maps.toml` (a symlink there is replaced, never followed). The
//!   directory is created with mode 0700.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use resilience_core::maps::geo::{LatLon, MAX_SCALE, MIN_SCALE, VIEW_MAX_LAT, View};

/// The file is never read beyond this many bytes.
pub const MAX_FILE: u64 = 4096;

/// `STATE_HOME/resilience/maps.toml`.
pub fn path(state_home: &Path) -> PathBuf {
    state_home.join("resilience").join("maps.toml")
}

/// The text written for a view.
pub fn render(view: &View) -> String {
    let c = view.center();
    format!(
        "# Last map position of resilience-maps\nlat = {:.6}\nlon = {:.6}\nscale = {}\n",
        c.lat(),
        c.lon(),
        view.scale()
    )
}

fn number(v: &str) -> Option<f64> {
    let v = v.trim();
    let ok = |c: char| c.is_ascii_digit() || c == '.' || c == '-';
    if v.is_empty() || v.len() > 20 || !v.chars().all(ok) {
        return None;
    }
    v.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// The view in a file's text, or `None` if anything is wrong with it.
pub fn parse(text: &str) -> Option<View> {
    let (mut lat, mut lon, mut scale) = (None, None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=')?;
        let slot = match k.trim() {
            "lat" => &mut lat,
            "lon" => &mut lon,
            "scale" => &mut scale,
            _ => return None,
        };
        if slot.is_some() {
            return None;
        }
        *slot = Some(number(v)?);
    }
    let (lat, lon, scale) = (lat?, lon?, scale?);
    if lat.abs() > VIEW_MAX_LAT || !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
        return None;
    }
    let center = LatLon::new(lat, lon)?;
    Some(View::new(center, scale))
}

/// The saved view, or `None` (no file, or one to ignore).
pub fn load(path: &Path) -> Option<View> {
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    let meta = f.metadata().ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut f).take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE {
        return None;
    }
    parse(std::str::from_utf8(&bytes).ok()?)
}

/// Save `view` at `path` atomically with mode 0600.
pub fn save(path: &Path, view: &View) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("no directory"))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("no file name"))?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    // A stale temporary file of an earlier run with the same PID.
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)?;
        f.write_all(render(view).as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TempDir;
    use std::os::unix::fs::PermissionsExt;

    fn view(lat: f64, lon: f64, scale: f64) -> View {
        View::new(LatLon::new(lat, lon).expect("valid"), scale)
    }

    #[test]
    fn round_trip_with_mode_0600() {
        let dir = TempDir::new("position");
        let p = path(&dir.path().join("state"));
        assert_eq!(load(&p), None, "no file yet");
        let v = view(49.6116, 6.1319, 50.0);
        save(&p, &v).expect("save");
        let back = load(&p).expect("load");
        assert!((back.center().lat() - 49.6116).abs() < 1e-6);
        assert!((back.center().lon() - 6.1319).abs() < 1e-6);
        assert_eq!(back.scale(), 50.0);
        let mode = fs::metadata(&p).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = fs::metadata(p.parent().expect("dir"))
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700);
        // Saving again replaces it; no temporary file is left behind.
        save(&p, &view(-33.8688, 151.2093, 200_000.0)).expect("save again");
        assert_eq!(load(&p).expect("load").scale(), 200_000.0);
        let names: Vec<_> = fs::read_dir(p.parent().expect("dir"))
            .expect("list")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("maps.toml")]);
        assert!(
            fs::read_to_string(&p)
                .expect("read")
                .contains("scale = 200000")
        );
    }

    #[test]
    fn corrupt_or_hostile_files_are_ignored() {
        let bad = [
            "",
            "lat = 1\nlon = 2\n",
            "lat = 1\nlon = 2\nscale = 50\nscale = 50\n",
            "lat = 1\nlon = 2\nscale = 50\nextra = 1\n",
            "lat = 95\nlon = 2\nscale = 50\n",
            "lat = 86\nlon = 2\nscale = 50\n",
            "lat = 1\nlon = 200\nscale = 50\n",
            "lat = 1\nlon = 2\nscale = 1\n",
            "lat = 1\nlon = 2\nscale = 1e9\n",
            "lat = nan\nlon = 2\nscale = 50\n",
            "lat = inf\nlon = 2\nscale = 50\n",
            "lat = \"1\"\nlon = 2\nscale = 50\n",
            "lat 1\nlon = 2\nscale = 50\n",
            "lat = 1\x1b[2J\nlon = 2\nscale = 50\n",
            "[view]\nlat = 1\nlon = 2\nscale = 50\n",
        ];
        for text in bad {
            assert_eq!(parse(text), None, "{text:?}");
        }
        let good = parse("# c\n\n lat = -1.5 \nlon=2\nscale = 75\n").expect("good");
        assert_eq!(good.scale(), 75.0, "kept as given within the range");

        let dir = TempDir::new("position-hostile");
        let p = path(dir.path());
        fs::create_dir_all(p.parent().expect("dir")).expect("mkdir");
        // Too large, even if it starts well.
        let mut big = render(&view(1.0, 2.0, 50.0));
        big.push_str(&"#".repeat(MAX_FILE as usize));
        fs::write(&p, big).expect("write");
        assert_eq!(load(&p), None);
        // Not UTF-8.
        fs::write(&p, b"lat = 1\nlon = 2\nscale = 50\n\xff").expect("write");
        assert_eq!(load(&p), None);
        // A symlink to a valid file is not followed.
        let target = dir.file("elsewhere.toml");
        fs::write(&target, render(&view(1.0, 2.0, 50.0))).expect("write");
        fs::remove_file(&p).expect("rm");
        std::os::unix::fs::symlink(&target, &p).expect("symlink");
        assert_eq!(load(&p), None);
        // Saving replaces the link itself, the target is untouched.
        save(&p, &view(3.0, 4.0, 100.0)).expect("save over link");
        assert!(
            !fs::symlink_metadata(&p)
                .expect("stat")
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::read_to_string(&target)
                .expect("read")
                .contains("lat = 1.0")
        );
        assert_eq!(load(&p).expect("load").scale(), 100.0);
        // A directory in its place.
        fs::remove_file(&p).expect("rm");
        fs::create_dir(&p).expect("mkdir");
        assert_eq!(load(&p), None);
        assert!(save(&p, &view(3.0, 4.0, 100.0)).is_err());
    }
}
