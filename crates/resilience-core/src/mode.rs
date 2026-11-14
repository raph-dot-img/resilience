//! Security modes.
//!
//! The mode is **derived from the operating system**, never stored as a UI
//! preference: it reflects the Unix groups of the running process, which are
//! what the kernel, sudo, polkit and the Nix daemon actually enforce.
//!
//! * `wheel` membership          → [`Mode::Advanced`] (may use sudo with a password)
//! * `resilience-stable` member  → [`Mode::Stable`]   (unprivileged shell access)
//! * neither                     → [`Mode::UltraStable`] (guided TUI only)
//!
//! Hiding menu entries is a usability feature, not a security boundary. The
//! boundary is the account's lack of privileges; see `docs/THREAT_MODEL.md`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Group granting the Stable mode (normal unprivileged shell).
pub const GROUP_STABLE: &str = "resilience-stable";
/// Group granting administrative rights (sudo). Advanced mode.
pub const GROUP_ADMIN: &str = "wheel";

/// The three Resilience security modes, ordered by capability.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Guided operations only, no shell, no administration. The fail-safe default.
    #[default]
    UltraStable,
    /// Unprivileged shell access, no system modification.
    Stable,
    /// Administrative capabilities.
    Advanced,
}

impl Mode {
    /// Derive the mode from group names. Unknown groups are ignored.
    pub fn from_groups<I, S>(groups: I) -> Mode
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        groups
            .into_iter()
            .fold(Mode::UltraStable, |mode, g| match g.as_ref() {
                GROUP_ADMIN => mode.max(Mode::Advanced),
                GROUP_STABLE => mode.max(Mode::Stable),
                _ => mode,
            })
    }

    /// Whether an account in this mode may use something requiring `required`.
    pub fn allows(self, required: Mode) -> bool {
        self >= required
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::UltraStable => "ULTRA-STABLE",
            Mode::Stable => "STABLE",
            Mode::Advanced => "ADVANCED",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Mode::UltraStable => {
                "Guided operations only. No shell and no administrative rights. \
                 Your files and the system are protected from accidental damage."
            }
            Mode::Stable => {
                "Normal unprivileged shell access. You can manage your own files \
                 but cannot change the operating system configuration."
            }
            Mode::Advanced => {
                "Administrative rights (sudo, with your password). Mistakes in this \
                 mode CAN damage the installation. Previous system generations remain \
                 available from the boot menu."
            }
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Result of detecting the mode of the current process.
#[derive(Debug, Clone)]
pub struct Detected {
    pub mode: Mode,
    pub user: String,
    pub groups: Vec<String>,
}

/// Map group IDs to names using the contents of an `/etc/group`-style file.
pub fn group_names(gids: &[u32], group_file: &str) -> Vec<String> {
    let mut names: Vec<String> = group_file
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let gid: u32 = fields.nth(1)?.parse().ok()?;
            gids.contains(&gid).then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Look up the login name for a UID in an `/etc/passwd`-style file.
pub fn user_name(uid: u32, passwd_file: &str) -> Option<String> {
    passwd_file.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let id: u32 = fields.nth(1)?.parse().ok()?;
        (id == uid).then(|| name.to_string())
    })
}

/// Supplementary and effective group IDs of this process.
pub fn current_gids() -> Vec<u32> {
    // SAFETY: getgroups(0, NULL) only returns the count; the second call writes
    // at most `n` entries into a buffer of exactly that length.
    let mut gids = unsafe {
        let n = libc::getgroups(0, std::ptr::null_mut());
        if n < 0 {
            Vec::new()
        } else {
            let mut buf = vec![0 as libc::gid_t; n as usize];
            let got = libc::getgroups(n, buf.as_mut_ptr());
            buf.truncate(got.max(0) as usize);
            buf
        }
    };
    // SAFETY: getegid has no preconditions and cannot fail.
    gids.push(unsafe { libc::getegid() });
    gids
}

/// Detect the mode of the running process from the real group database.
/// If anything cannot be read, the least-privileged mode is reported.
pub fn detect() -> Detected {
    let group_file = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let passwd_file = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let groups = group_names(&current_gids(), &group_file);
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let user = user_name(uid, &passwd_file)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| format!("uid {uid}"));
    Detected {
        mode: Mode::from_groups(&groups),
        user,
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUPS: &str = "root:x:0:\nwheel:x:1:alice\nusers:x:100:\nresilience-stable:x:990:bob\n";

    #[test]
    fn default_is_least_privileged() {
        assert_eq!(Mode::default(), Mode::UltraStable);
        assert_eq!(Mode::from_groups(Vec::<String>::new()), Mode::UltraStable);
        assert_eq!(Mode::from_groups(["users", "audio"]), Mode::UltraStable);
    }

    #[test]
    fn groups_map_to_modes() {
        assert_eq!(
            Mode::from_groups(["users", "resilience-stable"]),
            Mode::Stable
        );
        assert_eq!(Mode::from_groups(["wheel"]), Mode::Advanced);
        // Admin rights dominate regardless of order: the mode must never
        // under-report real privileges.
        assert_eq!(
            Mode::from_groups(["wheel", "resilience-stable"]),
            Mode::Advanced
        );
        assert_eq!(
            Mode::from_groups(["resilience-stable", "wheel"]),
            Mode::Advanced
        );
    }

    #[test]
    fn group_names_are_case_sensitive() {
        assert_eq!(
            Mode::from_groups(["Wheel", "RESILIENCE-STABLE"]),
            Mode::UltraStable
        );
    }

    #[test]
    fn mode_ordering() {
        assert!(Mode::Advanced.allows(Mode::Stable));
        assert!(Mode::Stable.allows(Mode::UltraStable));
        assert!(!Mode::UltraStable.allows(Mode::Stable));
        assert!(!Mode::Stable.allows(Mode::Advanced));
    }

    #[test]
    fn resolves_group_names() {
        assert_eq!(
            group_names(&[100, 990], GROUPS),
            vec!["resilience-stable", "users"]
        );
        assert_eq!(group_names(&[1], GROUPS), vec!["wheel"]);
        assert!(group_names(&[4242], GROUPS).is_empty());
        // Malformed lines are skipped rather than panicking.
        assert!(group_names(&[5], "broken\n::\nx:y:notanumber:\n").is_empty());
    }

    #[test]
    fn resolves_user_names() {
        let passwd = "root:x:0:0::/root:/bin/sh\nalice:x:1000:100::/home/alice:/bin/sh\n";
        assert_eq!(user_name(1000, passwd).as_deref(), Some("alice"));
        assert_eq!(user_name(7, passwd), None);
    }

    #[test]
    fn serde_names() {
        #[derive(Deserialize)]
        struct W {
            m: Mode,
        }
        let w: W = toml::from_str("m = \"ultra-stable\"").expect("parse");
        assert_eq!(w.m, Mode::UltraStable);
        assert!(toml::from_str::<W>("m = \"root\"").is_err());
    }
}
