//! `resilience-maps info` as the TUI will run it: the real binary, which
//! lowers its address-space limit to 1 GiB before opening the file.

#[path = "../src/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

use std::process::{Command, Output};

use fixtures::*;

const BIN: &str = env!("CARGO_BIN_EXE_resilience-maps");

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("run resilience-maps")
}

fn info(path: &std::path::Path, check: bool) -> Output {
    let dir = path.parent().expect("parent");
    unchanged(dir, || {
        let mut args = vec![std::ffi::OsStr::new("info")];
        if check {
            args.push("--check".as_ref());
        }
        args.push(path.as_os_str());
        run(&args)
    })
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("UTF-8 output")
}

/// No control character but the line ends, nothing that reorders text.
fn assert_printable(s: &str) {
    for c in s.chars() {
        assert!(
            c == '\n' || !resilience_maps::text::is_unsafe(c),
            "{c:?} in {s:?}"
        );
    }
}

fn field<'a>(out: &'a str, key: &str) -> &'a str {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")))
        .unwrap_or_else(|| panic!("no {key} in {out}"))
}

#[test]
fn info_describes_a_valid_pack() {
    let dir = TempDir::new("cli-valid");
    let path = dir.file("map-osm-testland_2026-01.mbtiles");
    write_pack(&path);
    let o = info(&path, true);
    let out = stdout(&o);
    assert!(
        o.status.success(),
        "{out}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_printable(&out);
    let keys: Vec<&str> = out
        .lines()
        .map(|l| l.split(": ").next().expect("should succeed"))
        .collect();
    assert_eq!(
        keys,
        [
            "file",
            "name",
            "profile",
            "data-date",
            "source",
            "coverage",
            "license",
            "attribution",
            "builder",
            "format",
            "zoom",
            "bounds",
            "center",
            "size",
            "tiles",
            "coverage-area",
            "search",
            "points-of-interest",
            "integrity"
        ]
    );
    assert_eq!(field(&out, "name"), "Testland");
    assert_eq!(field(&out, "profile"), "resilience-1");
    assert_eq!(field(&out, "data-date"), "2026-01-01");
    assert_eq!(field(&out, "license"), "ODbL-1.0");
    assert_eq!(field(&out, "attribution"), "(c) OpenStreetMap contributors");
    assert_eq!(field(&out, "builder"), "unknown");
    assert_eq!(field(&out, "zoom"), "0-2");
    assert_eq!(field(&out, "bounds"), "5.70000,49.40000,6.60000,50.20000");
    assert_eq!(field(&out, "center"), "49.61160,6.13190,2");
    assert_eq!(field(&out, "tiles"), "4");
    assert_eq!(field(&out, "coverage-area"), "1 rings, 4 points");
    assert_eq!(field(&out, "search"), "available");
    assert_eq!(field(&out, "points-of-interest"), "available");
    assert_eq!(field(&out, "integrity"), "ok");
    // Without --check the file is not checked.
    let o = info(&path, false);
    assert_eq!(field(&stdout(&o), "integrity"), "not checked");
}

#[test]
fn info_cleans_hostile_strings() {
    let dir = TempDir::new("cli-hostile");
    // The file name itself carries an escape sequence.
    let path = dir.file("evil\x1b[2J.mbtiles");
    write_hostile_strings_pack(&path);
    let o = info(&path, false);
    let out = stdout(&o);
    assert!(o.status.success(), "{out}");
    assert_printable(&out);
    assert!(!out.contains('\x1b') && !out.contains('\r') && !out.contains('\u{9b}'));
    // The forged line stayed inside the name's value.
    assert_eq!(
        out.lines().filter(|l| l.starts_with("profile: ")).count(),
        1
    );
    assert!(field(&out, "name").starts_with("Evil?]0;pwned"));
    assert!(field(&out, "file").ends_with("evil?[2J.mbtiles"));
}

#[test]
fn refusals_and_usage_errors_have_their_own_exit_codes() {
    let dir = TempDir::new("cli-refused");
    let path = dir.file("random.mbtiles");
    std::fs::write(&path, vec![0x5au8; 8192]).expect("should succeed");
    let o = info(&path, false);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.starts_with("Refused: ") && err.contains("not an SQLite file"),
        "{err}"
    );

    let wal = dir.file("wal.mbtiles");
    write_pack_with(&wal, |c| {
        c.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))
            .expect("should succeed");
    });
    let o = info(&wal, false);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("WAL"));

    let o = info(&dir.file("missing.mbtiles"), false);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("Error: "));

    for args in [&["info"][..], &["view"], &["info", "a", "b"], &[]] {
        let a: Vec<&std::ffi::OsStr> = args.iter().map(|s| s.as_ref()).collect();
        assert_eq!(run(&a).status.code(), Some(64), "{args:?}");
    }
    assert!(run(&["--version".as_ref()]).status.success());
}
