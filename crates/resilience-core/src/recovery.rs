//! Read-only recovery information: system generations, failed services,
//! recent errors, and the diagnostic report users can hand to a helper.
//!
//! Nothing here needs privileges or changes the system. System rollback
//! (choosing an earlier generation in the boot menu) and user-data restore
//! are deliberately separate features (MASTER-PROMPT §20).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::proc::run_full;

/// Where NixOS keeps the system profile and its generations.
pub const PROFILES_DIR: &str = "/nix/var/nix/profiles";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    pub number: u64,
    /// Store path of the system closure.
    pub target: PathBuf,
    pub created: Option<SystemTime>,
    pub nixos_version: Option<String>,
    pub kernel: Option<String>,
    /// The default for the next boot.
    pub default: bool,
    /// The generation the machine booted.
    pub booted: bool,
    /// The generation currently active (differs from booted after a switch).
    pub running: bool,
}

/// `system-12-link` → 12.
pub fn parse_generation_link(name: &str) -> Option<u64> {
    name.strip_prefix("system-")?
        .strip_suffix("-link")?
        .parse()
        .ok()
}

/// `/nix/store/<hash>-linux-6.12.58/bzImage` → `6.12.58`.
pub fn kernel_version_from_path(path: &str) -> Option<String> {
    let dir = Path::new(path).parent()?.file_name()?.to_str()?;
    let (_, rest) = dir.split_once('-')?;
    let v = rest.strip_prefix("linux-")?;
    (!v.is_empty()).then(|| v.to_string())
}

fn canonical(p: &Path) -> Option<PathBuf> {
    fs::canonicalize(p).ok()
}

/// List system generations, newest first. Missing directories yield an
/// empty list (live ISO, test VMs).
pub fn generations(profiles: &Path, booted: &Path, running: &Path) -> Vec<Generation> {
    let default_name = fs::read_link(profiles.join("system"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let booted = canonical(booted);
    let running = canonical(running);
    let mut out: Vec<Generation> = fs::read_dir(profiles)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let number = parse_generation_link(&name)?;
            let link = e.path();
            let target = canonical(&link)?;
            Some(Generation {
                number,
                created: fs::symlink_metadata(&link).and_then(|m| m.modified()).ok(),
                nixos_version: fs::read_to_string(target.join("nixos-version"))
                    .ok()
                    .map(|s| s.trim().to_string()),
                kernel: fs::read_link(target.join("kernel"))
                    .ok()
                    .and_then(|k| kernel_version_from_path(&k.to_string_lossy())),
                default: default_name.as_deref() == Some(name.as_str()),
                booted: booted.as_ref() == Some(&target),
                running: running.as_ref() == Some(&target),
                target,
            })
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.number));
    out
}

/// Parse `systemctl show` output (`Key=Value` lines).
pub fn parse_show(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitFailure {
    pub unit: String,
    pub description: String,
    pub result: String,
    pub since: String,
}

/// Details about failed units (unprivileged `systemctl show`).
pub fn failure_details(units: &[String]) -> Vec<UnitFailure> {
    units
        .iter()
        .filter(|u| {
            !u.is_empty()
                && u.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "@._-:\\".contains(c))
        })
        .map(|u| {
            let props = run_full(
                &[
                    "systemctl",
                    "show",
                    "--no-pager",
                    "-p",
                    "Description,Result,InactiveEnterTimestamp",
                    "--",
                    u,
                ],
                Duration::from_secs(3),
            )
            .map(|o| parse_show(&o.stdout))
            .unwrap_or_default();
            let get = |k: &str| props.get(k).cloned().unwrap_or_default();
            UnitFailure {
                unit: u.clone(),
                description: get("Description"),
                result: get("Result"),
                since: get("InactiveEnterTimestamp"),
            }
        })
        .collect()
}

/// Recent error messages of this boot that this account may read. Members
/// of `wheel` see the whole system journal; others only their own entries.
pub fn recent_errors(max: usize) -> Vec<String> {
    let n = max.to_string();
    run_full(
        &[
            "journalctl",
            "--boot",
            "--priority",
            "err",
            "--no-pager",
            "--quiet",
            "--output",
            "short",
            "--lines",
            &n,
        ],
        Duration::from_secs(5),
    )
    .map(|o| {
        o.stdout
            .lines()
            .map(|l| l.chars().filter(|c| !c.is_control()).collect())
            .collect()
    })
    .unwrap_or_default()
}

/// `resilience-report-20261114-103000.txt` for a UTC timestamp.
pub fn report_file_name(rfc3339: &str) -> String {
    let compact: String = rfc3339
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'T')
        .collect();
    format!("resilience-report-{}.txt", compact.replace('T', "-"))
}

/// Directory for exported reports: `~/Diagnostics` (mode 0700).
pub fn reports_dir(home: &Path) -> PathBuf {
    home.join("Diagnostics")
}

/// Write `text` to a new file (0600) in `dir` (created 0700). Never
/// overwrites an existing file.
pub fn write_report(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let path = dir.join(name);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    Ok(path)
}

/// Last `n` lines of a text file (for including the interface log).
pub fn tail(path: &Path, n: usize) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn generation_links() {
        assert_eq!(parse_generation_link("system-12-link"), Some(12));
        assert_eq!(parse_generation_link("system-link"), None);
        assert_eq!(parse_generation_link("system-x-link"), None);
        assert_eq!(parse_generation_link("per-user"), None);
        assert_eq!(parse_generation_link("system"), None);
    }

    #[test]
    fn kernel_versions() {
        assert_eq!(
            kernel_version_from_path("/nix/store/abc123-linux-6.12.58/bzImage").as_deref(),
            Some("6.12.58")
        );
        assert_eq!(kernel_version_from_path("/nix/store/abc-foo/bzImage"), None);
        assert_eq!(kernel_version_from_path("bzImage"), None);
    }

    #[test]
    fn lists_generations_newest_first() {
        let d = std::env::temp_dir().join(format!("resilience-gen-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let profiles = d.join("profiles");
        fs::create_dir_all(&profiles).expect("mkdir");
        for n in [1u64, 2, 10] {
            let sys = d.join(format!("sys{n}"));
            fs::create_dir_all(&sys).expect("mkdir");
            fs::write(sys.join("nixos-version"), format!("26.05.{n}\n")).expect("write");
            symlink(&sys, profiles.join(format!("system-{n}-link"))).expect("symlink");
        }
        symlink("system-2-link", profiles.join("system")).expect("symlink");
        symlink(d.join("sys10"), d.join("booted")).expect("symlink");
        let g = generations(&profiles, &d.join("booted"), &d.join("missing"));
        assert_eq!(
            g.iter().map(|g| g.number).collect::<Vec<_>>(),
            vec![10, 2, 1]
        );
        assert!(g[0].booted && !g[0].default && !g[0].running);
        assert!(g[1].default && !g[1].booted);
        assert_eq!(g[2].nixos_version.as_deref(), Some("26.05.1"));
        assert!(generations(&d.join("nope"), &d, &d).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn parses_systemctl_show() {
        let p = parse_show("Result=exit-code\nDescription=My = Service\n\n");
        assert_eq!(p["Result"], "exit-code");
        assert_eq!(p["Description"], "My = Service");
    }

    #[test]
    fn report_names_and_files() {
        assert_eq!(
            report_file_name("2026-11-14T10:30:00Z"),
            "resilience-report-20261114-103000.txt"
        );
        let d = std::env::temp_dir().join(format!("resilience-rep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let dir = d.join("Diagnostics");
        let p = write_report(&dir, "r.txt", "hello\n").expect("write");
        assert_eq!(fs::read_to_string(&p).expect("read"), "hello\n");
        let mode = |p: &Path| fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&p), 0o600);
        assert_eq!(mode(&dir), 0o700);
        assert!(
            write_report(&dir, "r.txt", "again").is_err(),
            "never overwrites"
        );
        fs::write(d.join("log"), "a\nb\nc\n").expect("write");
        assert_eq!(tail(&d.join("log"), 2), vec!["b", "c"]);
        assert_eq!(tail(&d.join("missing"), 2), Vec::<String>::new());
        let _ = fs::remove_dir_all(&d);
    }
}
