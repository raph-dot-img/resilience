//! System status: battery, storage, memory, uptime, systemd state.
//!
//! All readers are cheap (procfs/sysfs reads, one `statvfs`) except
//! [`system_state`] and [`failed_units`], which call `systemctl` with a hard
//! timeout and should be refreshed less often.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::proc::run_capture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    pub name: String,
    /// Charge in percent, if the firmware reports it.
    pub capacity: Option<u8>,
    /// Kernel status string: Charging, Discharging, Full, Not charging, Unknown.
    pub status: String,
}

/// Read every battery under a `power_supply` class directory
/// (normally `/sys/class/power_supply`).
pub fn batteries_in(root: &Path) -> Vec<Battery> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Battery> = entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let kind = read_trim(&dir.join("type"))?;
            if kind != "Battery" {
                return None;
            }
            Some(Battery {
                name: e.file_name().to_string_lossy().into_owned(),
                capacity: read_trim(&dir.join("capacity"))
                    .and_then(|c| c.parse::<u16>().ok())
                    .map(|c| c.min(100) as u8),
                status: read_trim(&dir.join("status")).unwrap_or_else(|| "Unknown".into()),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Whether any mains adapter reports `online = 1`.
pub fn on_ac_power(root: &Path) -> Option<bool> {
    let entries = fs::read_dir(root).ok()?;
    let mut seen = false;
    for e in entries.flatten() {
        let dir = e.path();
        if read_trim(&dir.join("type")).as_deref() == Some("Mains") {
            seen = true;
            if read_trim(&dir.join("online")).as_deref() == Some("1") {
                return Some(true);
            }
        }
    }
    seen.then_some(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Storage {
    pub path: PathBuf,
    pub total: u64,
    /// Bytes available to unprivileged users.
    pub available: u64,
}

/// Filesystem capacity for the filesystem containing `path`.
pub fn storage(path: &Path) -> Option<Storage> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a valid NUL-terminated string and `st` is a properly
    // sized, writable statvfs struct that the call fills in.
    let st = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return None;
        }
        st
    };
    let frsize = st.f_frsize as u64;
    Some(Storage {
        path: path.to_path_buf(),
        total: st.f_blocks as u64 * frsize,
        available: st.f_bavail as u64 * frsize,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub total_kib: u64,
    pub available_kib: u64,
}

pub fn parse_meminfo(text: &str) -> Option<Memory> {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            let rest = l.strip_prefix(name)?.strip_prefix(':')?;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    Some(Memory {
        total_kib: field("MemTotal")?,
        available_kib: field("MemAvailable")?,
    })
}

pub fn parse_uptime(text: &str) -> Option<Duration> {
    let secs: f64 = text.split_whitespace().next()?.parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs(secs as u64))
}

pub fn parse_loadavg(text: &str) -> Option<[f32; 3]> {
    let mut it = text.split_whitespace().map(|v| v.parse::<f32>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

/// Extract `PRETTY_NAME` from os-release content.
pub fn parse_os_release(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.strip_prefix("PRETTY_NAME=")?;
        Some(v.trim_matches('"').to_string())
    })
}

pub fn format_uptime(d: Duration) -> String {
    let s = d.as_secs();
    let (days, hours, mins) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if days > 0 {
        format!("{days}d {hours:02}h {mins:02}m")
    } else {
        format!("{hours:02}h {mins:02}m")
    }
}

/// Overall state reported by systemd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemState {
    Running,
    Degraded,
    Starting,
    Maintenance,
    Stopping,
    /// systemd could not be queried (e.g. development host without systemd).
    Unknown(String),
}

impl SystemState {
    pub fn parse(text: &str) -> SystemState {
        match text.trim() {
            "running" => SystemState::Running,
            "degraded" => SystemState::Degraded,
            "initializing" | "starting" => SystemState::Starting,
            "maintenance" => SystemState::Maintenance,
            "stopping" => SystemState::Stopping,
            "" => SystemState::Unknown("no answer".into()),
            other => SystemState::Unknown(other.into()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            SystemState::Running => "OPERATIONAL".into(),
            SystemState::Degraded => "DEGRADED".into(),
            SystemState::Starting => "STARTING".into(),
            SystemState::Maintenance => "MAINTENANCE".into(),
            SystemState::Stopping => "STOPPING".into(),
            SystemState::Unknown(s) => format!("UNKNOWN ({s})"),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, SystemState::Running)
    }
}

const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(3);

pub fn system_state() -> SystemState {
    match run_capture(&["systemctl", "is-system-running"], SYSTEMCTL_TIMEOUT) {
        Some(out) => SystemState::parse(&out),
        None => SystemState::Unknown("systemctl unavailable".into()),
    }
}

/// Names of failed systemd units.
pub fn failed_units() -> Vec<String> {
    run_capture(
        &[
            "systemctl",
            "list-units",
            "--state=failed",
            "--plain",
            "--no-legend",
            "--no-pager",
        ],
        SYSTEMCTL_TIMEOUT,
    )
    .map(|o| parse_failed_units(&o))
    .unwrap_or_default()
}

pub fn parse_failed_units(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|u| u.contains('.'))
        .map(str::to_string)
        .collect()
}

/// A cheap snapshot of the machine, refreshed every few seconds by the TUI.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub hostname: String,
    pub kernel: String,
    pub os_name: String,
    pub system_generation: Option<String>,
    pub batteries: Vec<Battery>,
    pub ac_power: Option<bool>,
    pub root: Option<Storage>,
    pub home: Option<Storage>,
    pub memory: Option<Memory>,
    pub uptime: Option<Duration>,
    pub load: Option<[f32; 3]>,
}

fn read_trim(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

pub fn snapshot(home: &Path) -> Snapshot {
    let ps = Path::new("/sys/class/power_supply");
    Snapshot {
        hostname: read_trim(Path::new("/proc/sys/kernel/hostname"))
            .unwrap_or_else(|| "unknown".into()),
        kernel: read_trim(Path::new("/proc/sys/kernel/osrelease"))
            .unwrap_or_else(|| "unknown".into()),
        os_name: fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|t| parse_os_release(&t))
            .unwrap_or_else(|| "unknown".into()),
        system_generation: fs::read_link("/run/current-system")
            .ok()
            .map(|p| p.display().to_string()),
        batteries: batteries_in(ps),
        ac_power: on_ac_power(ps),
        root: storage(Path::new("/")),
        home: storage(home),
        memory: fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|t| parse_meminfo(&t)),
        uptime: fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|t| parse_uptime(&t)),
        load: fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|t| parse_loadavg(&t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo() {
        let t =
            "MemTotal:        2014128 kB\nMemFree:          100 kB\nMemAvailable:    1500000 kB\n";
        assert_eq!(
            parse_meminfo(t),
            Some(Memory {
                total_kib: 2014128,
                available_kib: 1500000
            })
        );
        assert_eq!(parse_meminfo("MemTotal: 5 kB\n"), None);
        // Prefix collisions must not match ("MemTotalX").
        assert_eq!(parse_meminfo("MemTotalX: 1 kB\nMemAvailable: 1 kB\n"), None);
    }

    #[test]
    fn uptime_and_load() {
        assert_eq!(
            parse_uptime("3723.45 1000.0\n"),
            Some(Duration::from_secs(3723))
        );
        assert_eq!(parse_uptime("garbage"), None);
        assert_eq!(parse_uptime("-5 0"), None);
        assert_eq!(format_uptime(Duration::from_secs(3723)), "01h 02m");
        assert_eq!(format_uptime(Duration::from_secs(90_061)), "1d 01h 01m");
        assert_eq!(
            parse_loadavg("0.10 0.20 0.30 1/100 42\n"),
            Some([0.1, 0.2, 0.3])
        );
        assert_eq!(parse_loadavg("0.10"), None);
    }

    #[test]
    fn systemd_states() {
        assert_eq!(SystemState::parse("running\n"), SystemState::Running);
        assert_eq!(SystemState::parse("degraded"), SystemState::Degraded);
        assert!(matches!(
            SystemState::parse("offline"),
            SystemState::Unknown(_)
        ));
        assert!(!SystemState::Degraded.is_ok());
    }

    #[test]
    fn failed_unit_list() {
        let t = "foo.service loaded failed failed Foo\nbar.mount loaded failed failed Bar\n\n";
        assert_eq!(parse_failed_units(t), vec!["foo.service", "bar.mount"]);
        assert!(parse_failed_units("").is_empty());
    }

    #[test]
    fn os_release() {
        let t = "NAME=NixOS\nPRETTY_NAME=\"Resilience OS 0.1\"\n";
        assert_eq!(parse_os_release(t).as_deref(), Some("Resilience OS 0.1"));
    }

    #[test]
    fn reads_fake_power_supply_tree() {
        let dir = std::env::temp_dir().join(format!("resilience-ps-{}", std::process::id()));
        let bat = dir.join("BAT0");
        let ac = dir.join("AC");
        fs::create_dir_all(&bat).expect("mkdir");
        fs::create_dir_all(&ac).expect("mkdir");
        fs::write(bat.join("type"), "Battery\n").expect("w");
        fs::write(bat.join("capacity"), "87\n").expect("w");
        fs::write(bat.join("status"), "Discharging\n").expect("w");
        fs::write(ac.join("type"), "Mains\n").expect("w");
        fs::write(ac.join("online"), "0\n").expect("w");
        let b = batteries_in(&dir);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].capacity, Some(87));
        assert_eq!(b[0].status, "Discharging");
        assert_eq!(on_ac_power(&dir), Some(false));
        fs::remove_dir_all(&dir).expect("cleanup");
        assert!(batteries_in(&dir).is_empty());
    }

    #[test]
    fn statvfs_root() {
        let s = storage(Path::new("/")).expect("statvfs /");
        assert!(s.total > 0 && s.available <= s.total);
        assert!(storage(Path::new("/definitely/not/here")).is_none());
    }
}
