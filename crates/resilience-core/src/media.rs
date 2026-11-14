//! Removable media (USB sticks, SD cards) through udisks2.
//!
//! Listing uses `lsblk` (unprivileged). Mounting and ejecting go through
//! `udisksctl --no-user-interaction`: udisks2 and its polkit policy decide.
//! For an active local session the defaults allow removable media without a
//! password and require an administrator for internal ("system") disks;
//! `--no-user-interaction` turns any password request into a refusal, so the
//! interface never mounts an internal disk. The interface also offers only
//! devices that sit on a removable or hot-plugged drive (ADR 0008).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::proc::run_full;

/// Columns requested from `lsblk -P -b`.
pub const LSBLK_COLUMNS: &str =
    "NAME,PATH,PKNAME,TYPE,RM,HOTPLUG,RO,SIZE,LABEL,FSTYPE,MOUNTPOINT,FSAVAIL,TRAN,MODEL";

/// Where udisks2 mounts removable media.
pub const MEDIA_ROOT: &str = "/run/media/";

/// File systems the interface offers for backups.
const SUPPORTED_FS: [&str; 9] = [
    "vfat", "exfat", "ntfs", "ntfs3", "ext2", "ext3", "ext4", "btrfs", "xfs",
];

/// Decode lsblk's `\xHH` escapes.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1] == b'x'
            && b[i + 2].is_ascii_hexdigit()
            && b[i + 3].is_ascii_hexdigit()
        {
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(b[i + 2]) * 16 + hex(b[i + 3]));
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `lsblk -P` output: one `KEY="value" ...` line per device.
pub fn parse_pairs(text: &str) -> Vec<BTreeMap<String, String>> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut row = BTreeMap::new();
            let mut rest = line.trim_start();
            while let Some(eq) = rest.find("=\"") {
                let key = rest[..eq].trim().to_string();
                let after = &rest[eq + 2..];
                let Some(end) = after.find('"') else { break };
                row.insert(key, unescape(&after[..end]));
                rest = after[end + 1..].trim_start();
            }
            row
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// Block device of the file system, e.g. `/dev/sda1`.
    pub device: String,
    /// Whole drive, e.g. `/dev/sda` (powered off on eject).
    pub drive: String,
    pub label: String,
    pub fstype: String,
    pub size: u64,
    pub model: String,
    pub mountpoint: Option<PathBuf>,
    /// Free space, when mounted.
    pub available: Option<u64>,
    pub read_only: bool,
    /// Why this volume cannot be used, if it cannot.
    pub problem: Option<String>,
}

impl Volume {
    /// Name shown to the user: the label, else model and device.
    pub fn display_name(&self) -> String {
        if !self.label.is_empty() {
            self.label.clone()
        } else if !self.model.is_empty() {
            format!("{} ({})", self.model, self.device)
        } else {
            self.device.clone()
        }
    }

    pub fn usable(&self) -> bool {
        self.problem.is_none()
    }
}

fn flag(row: &BTreeMap<String, String>, k: &str) -> bool {
    row.get(k).is_some_and(|v| v == "1")
}

fn get(row: &BTreeMap<String, String>, k: &str) -> String {
    row.get(k).cloned().unwrap_or_default()
}

/// File systems on removable or hot-plugged drives, from parsed lsblk rows.
/// Internal disks are never returned.
pub fn removable_volumes(rows: &[BTreeMap<String, String>]) -> Vec<Volume> {
    let by_name: BTreeMap<String, &BTreeMap<String, String>> =
        rows.iter().map(|r| (get(r, "NAME"), r)).collect();
    let mut out = Vec::new();
    for row in rows {
        let fstype = get(row, "FSTYPE");
        let kind = get(row, "TYPE");
        if fstype.is_empty() || !(kind == "part" || kind == "disk") {
            continue;
        }
        // Walk up to the whole drive; the volume is removable if any device
        // on the way is (lsblk reports RM/HOTPLUG per device).
        let mut removable = false;
        let mut top = row;
        let mut seen = 0;
        loop {
            removable |= flag(top, "RM") || flag(top, "HOTPLUG");
            let parent = get(top, "PKNAME");
            match by_name.get(&parent) {
                Some(p) if !parent.is_empty() && seen < 8 => {
                    top = p;
                    seen += 1;
                }
                _ => break,
            }
        }
        if !removable {
            continue;
        }
        let device = get(row, "PATH");
        if !valid_device(&device) {
            continue;
        }
        let mountpoint = Some(get(row, "MOUNTPOINT"))
            .filter(|m| !m.is_empty())
            .map(PathBuf::from);
        let read_only = flag(row, "RO");
        let problem = if fstype == "crypto_LUKS" {
            Some("encrypted stick: not supported yet".to_string())
        } else if !SUPPORTED_FS.contains(&fstype.as_str()) {
            Some(format!("file system '{fstype}' is not supported"))
        } else if mountpoint
            .as_ref()
            .is_some_and(|m| !m.to_string_lossy().starts_with(MEDIA_ROOT))
        {
            Some("in use by the system".to_string())
        } else {
            None
        };
        out.push(Volume {
            drive: get(top, "PATH"),
            label: get(row, "LABEL"),
            size: get(row, "SIZE").parse().unwrap_or(0),
            model: get(top, "MODEL").trim().to_string(),
            available: get(row, "FSAVAIL").parse().ok(),
            device,
            fstype,
            mountpoint,
            read_only,
            problem,
        });
    }
    out
}

/// `/dev/` followed by a plain device name.
pub fn valid_device(p: &str) -> bool {
    p.strip_prefix("/dev/").is_some_and(|n| {
        !n.is_empty()
            && !n.starts_with('.')
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    })
}

/// Removable volumes currently attached.
pub fn list(lsblk: &str) -> Result<Vec<Volume>, String> {
    let o = run_full(
        &[lsblk, "-P", "-b", "-o", LSBLK_COLUMNS],
        Duration::from_secs(5),
    )
    .ok_or_else(|| "lsblk could not be run".to_string())?;
    if !o.success() {
        return Err(format!("lsblk failed: {}", o.stderr.trim()));
    }
    Ok(removable_volumes(&parse_pairs(&o.stdout)))
}

fn udisks(udisksctl: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut argv = vec![udisksctl];
    argv.extend_from_slice(args);
    match run_full(&argv, timeout) {
        Some(o) if o.success() => Ok(o.stdout),
        Some(o) => Err(clean_udisks_error(&o.stderr)),
        None => Err("udisks did not answer in time".to_string()),
    }
}

/// Turn udisksctl's error into one readable sentence.
pub fn clean_udisks_error(stderr: &str) -> String {
    let text = stderr.trim();
    let msg = text
        .rsplit_once(": ")
        .map(|(_, m)| m)
        .filter(|m| !m.is_empty())
        .unwrap_or(text);
    if text.contains("NotAuthorized") || text.contains("not authorized") {
        "the system did not allow this (only removable media can be used here)".to_string()
    } else if text.contains("target is busy") || text.contains("DeviceBusy") {
        "the stick is still in use; close any program using it and try again".to_string()
    } else if msg.is_empty() {
        "udisks reported an error".to_string()
    } else {
        msg.chars().filter(|c| !c.is_control()).collect()
    }
}

/// Mount a removable volume and return its mount point.
pub fn mount(tools_lsblk: &str, udisksctl: &str, v: &Volume) -> Result<PathBuf, String> {
    if let Some(m) = &v.mountpoint {
        return Ok(m.clone());
    }
    if !valid_device(&v.device) {
        return Err("invalid device".into());
    }
    udisks(
        udisksctl,
        &[
            "mount",
            "--no-user-interaction",
            "--block-device",
            &v.device,
        ],
        Duration::from_secs(30),
    )?;
    list(tools_lsblk)?
        .into_iter()
        .find(|x| x.device == v.device)
        .and_then(|x| x.mountpoint)
        .ok_or_else(|| "the stick was mounted but cannot be found".to_string())
}

/// What happened on eject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ejected {
    /// Unmounted and powered off.
    PoweredOff,
    /// Unmounted; the drive could not be powered off (still safe to unplug).
    Unmounted(String),
}

/// Unmount (all data is written out) and power the drive off.
pub fn eject(udisksctl: &str, v: &Volume) -> Result<Ejected, String> {
    if !valid_device(&v.device) || !valid_device(&v.drive) {
        return Err("invalid device".into());
    }
    if v.mountpoint.is_some() {
        udisks(
            udisksctl,
            &[
                "unmount",
                "--no-user-interaction",
                "--block-device",
                &v.device,
            ],
            Duration::from_secs(60),
        )?;
    }
    match udisks(
        udisksctl,
        &[
            "power-off",
            "--no-user-interaction",
            "--block-device",
            &v.drive,
        ],
        Duration::from_secs(30),
    ) {
        Ok(_) => Ok(Ejected::PoweredOff),
        Err(e) => Ok(Ejected::Unmounted(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from a VM with a virtio root disk, a USB stick with one FAT
    // partition, the live ISO stick, and an encrypted SD card.
    const SAMPLE: &str = r#"NAME="vda" PATH="/dev/vda" PKNAME="" TYPE="disk" RM="0" HOTPLUG="0" RO="0" SIZE="4294967296" LABEL="" FSTYPE="" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
NAME="vda1" PATH="/dev/vda1" PKNAME="vda" TYPE="part" RM="0" HOTPLUG="0" RO="0" SIZE="4293918720" LABEL="nixos" FSTYPE="ext4" MOUNTPOINT="/" FSAVAIL="3000000000" TRAN="" MODEL=""
NAME="vdb" PATH="/dev/vdb" PKNAME="" TYPE="disk" RM="0" HOTPLUG="0" RO="0" SIZE="67108864" LABEL="" FSTYPE="ext4" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
NAME="sda" PATH="/dev/sda" PKNAME="" TYPE="disk" RM="1" HOTPLUG="1" RO="0" SIZE="134217728" LABEL="" FSTYPE="" MOUNTPOINT="" FSAVAIL="" TRAN="usb" MODEL="QEMU HARDDISK   "
NAME="sda1" PATH="/dev/sda1" PKNAME="sda" TYPE="part" RM="0" HOTPLUG="0" RO="0" SIZE="133169152" LABEL="MY\x20STICK" FSTYPE="vfat" MOUNTPOINT="/run/media/alice/MY STICK" FSAVAIL="120000000" TRAN="" MODEL=""
NAME="sdb" PATH="/dev/sdb" PKNAME="" TYPE="disk" RM="1" HOTPLUG="1" RO="1" SIZE="1500000000" LABEL="RESILIENCE" FSTYPE="iso9660" MOUNTPOINT="/iso" FSAVAIL="0" TRAN="usb" MODEL="Live"
NAME="mmcblk0" PATH="/dev/mmcblk0" PKNAME="" TYPE="disk" RM="0" HOTPLUG="1" RO="0" SIZE="32000000000" LABEL="" FSTYPE="crypto_LUKS" MOUNTPOINT="" FSAVAIL="" TRAN="" MODEL=""
NAME="loop0" PATH="/dev/loop0" PKNAME="" TYPE="loop" RM="0" HOTPLUG="0" RO="1" SIZE="1000" LABEL="" FSTYPE="squashfs" MOUNTPOINT="/nix/.ro-store" FSAVAIL="0" TRAN="" MODEL=""
"#;

    #[test]
    fn parses_lsblk_pairs_with_escapes() {
        let rows = parse_pairs(SAMPLE);
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[4]["LABEL"], "MY STICK");
        assert_eq!(rows[4]["MOUNTPOINT"], "/run/media/alice/MY STICK");
        assert_eq!(rows[0]["PKNAME"], "");
        assert_eq!(unescape(r"a\x22b\x5c"), "a\"b\\");
        assert_eq!(unescape(r"trailing\x2"), r"trailing\x2");
    }

    #[test]
    fn only_removable_drives_are_offered() {
        let v = removable_volumes(&parse_pairs(SAMPLE));
        let devs: Vec<&str> = v.iter().map(|v| v.device.as_str()).collect();
        assert_eq!(devs, vec!["/dev/sda1", "/dev/sdb", "/dev/mmcblk0"]);
        let stick = &v[0];
        assert_eq!(stick.drive, "/dev/sda");
        assert_eq!(stick.display_name(), "MY STICK");
        assert_eq!(stick.model, "QEMU HARDDISK");
        assert_eq!(stick.available, Some(120000000));
        assert!(stick.usable() && !stick.read_only);
        assert_eq!(
            v[1].problem.as_deref(),
            Some("file system 'iso9660' is not supported")
        );
        assert_eq!(
            v[2].problem.as_deref(),
            Some("encrypted stick: not supported yet")
        );
    }

    #[test]
    fn system_mounts_are_not_usable() {
        let text = r#"NAME="sdc" PATH="/dev/sdc" PKNAME="" TYPE="disk" RM="1" HOTPLUG="1" RO="0" SIZE="1" LABEL="" FSTYPE="ext4" MOUNTPOINT="/mnt" FSAVAIL="1" TRAN="usb" MODEL="X""#;
        let v = removable_volumes(&parse_pairs(text));
        assert_eq!(v[0].problem.as_deref(), Some("in use by the system"));
        assert_eq!(v[0].display_name(), "X (/dev/sdc)");
    }

    #[test]
    fn device_names_are_validated() {
        assert!(valid_device("/dev/sda1"));
        assert!(valid_device("/dev/mmcblk0p1"));
        assert!(!valid_device("/dev/"));
        assert!(!valid_device("/dev/../etc/passwd"));
        assert!(!valid_device("/dev/sda 1"));
        assert!(!valid_device("/dev/mapper/x"));
        assert!(!valid_device("sda"));
        // A parsed row with a bad path is ignored, not offered.
        let text = r#"NAME="x" PATH="/dev/../x" PKNAME="" TYPE="disk" RM="1" HOTPLUG="1" RO="0" SIZE="1" LABEL="" FSTYPE="vfat" MOUNTPOINT="" FSAVAIL="" TRAN="usb" MODEL="""#;
        assert!(removable_volumes(&parse_pairs(text)).is_empty());
    }

    #[test]
    fn udisks_errors_are_readable() {
        assert_eq!(
            clean_udisks_error(
                "Error mounting /dev/vdb: GDBus.Error:org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: Not authorized to perform operation\n"
            ),
            "the system did not allow this (only removable media can be used here)"
        );
        assert!(
            clean_udisks_error("Error unmounting /dev/sda1: target is busy").contains("in use")
        );
        assert_eq!(
            clean_udisks_error("Error: Object is not a drive"),
            "Object is not a drive"
        );
        assert_eq!(clean_udisks_error(""), "udisks reported an error");
    }
}
