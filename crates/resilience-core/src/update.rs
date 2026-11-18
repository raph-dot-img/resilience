//! Offline updates (ADR 0023): the pure rules.
//!
//! An update bundle is a folder `resilience-update/` holding a `file://`
//! Nix binary cache signed with the release key, and an unsigned pointer
//! `resilience-update.toml` naming the manifest's store path. The manifest
//! is itself a signed store path listing the new system of each installed
//! variant. Nix checks hashes (integrity) and signatures (authenticity);
//! this module decides what to import and whether the update applies.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Folder name of a bundle at the root of a stick.
pub const BUNDLE_DIR: &str = "resilience-update";
/// The pointer file inside the bundle folder.
pub const POINTER_FILE: &str = "resilience-update.toml";
/// Bundle and manifest format understood by this version.
pub const FORMAT: u32 = 1;
/// Pointer and manifest files are small; anything larger is refused.
pub const MAX_TOML: u64 = 16 * 1024;
/// Bundles are applied only from removable media mounted by udisks.
pub const MEDIA_ROOT: &str = "/run/media";
/// The system profile (generations, ADR 0011).
pub const SYSTEM_PROFILE: &str = "/nix/var/nix/profiles/system";

/// Settings shared by system.toml (TUI) and admin.toml (helper).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateSettings {
    /// The `nix` program.
    pub nix: PathBuf,
    /// Nix public keys (`name:base64`) whose signatures are trusted.
    pub trusted_keys: Vec<String>,
    /// The installed configuration this system is, e.g.
    /// `installed-standard-uefi-luks`.
    pub variant: String,
    pub edition: String,
    /// The running release version.
    pub version: String,
}

impl UpdateSettings {
    /// Whether updates can be applied at all (release keys known).
    pub fn usable(&self) -> bool {
        !self.trusted_keys.is_empty() && self.trusted_keys.iter().all(|k| valid_key(k))
    }
}

/// A release version: three numbers, compared numerically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// `MAJOR.MINOR.PATCH`, digits only, no leading zeros.
    pub fn parse(s: &str) -> Option<Version> {
        let mut it = s.split('.');
        let mut num = || -> Option<u32> {
            let p = it.next()?;
            let ok = !p.is_empty()
                && p.len() <= 9
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p == "0" || !p.starts_with('0'));
            if ok { p.parse().ok() } else { None }
        };
        let v = Version(num()?, num()?, num()?);
        if it.next().is_some() {
            return None;
        }
        Some(v)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

const NIX_BASE32: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// A top-level Nix store path: `/nix/store/<32 base-32 chars>-<name>`,
/// nothing below it.
pub fn valid_store_path(p: &str) -> bool {
    let Some(rest) = p.strip_prefix("/nix/store/") else {
        return false;
    };
    let b = rest.as_bytes();
    if b.len() < 34 || b[32] != b'-' {
        return false;
    }
    let name = &b[33..];
    b[..32].iter().all(|c| NIX_BASE32.contains(c))
        && name.len() <= 211
        && name[0] != b'.'
        && name
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"+-._?=".contains(c))
}

/// A Nix public key: `name:base64` of 32 bytes.
pub fn valid_key(k: &str) -> bool {
    let Some((name, key)) = k.split_once(':') else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && key.len() == 44
        && key.ends_with('=')
        && key[..43]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pointer {
    pub edition: String,
    pub version: Version,
    pub manifest: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub edition: String,
    pub version: Version,
    /// Installed variant → its new toplevel.
    pub systems: BTreeMap<String, PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPointer {
    format: u32,
    edition: String,
    version: String,
    manifest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    format: u32,
    edition: String,
    version: String,
    systems: BTreeMap<String, String>,
}

fn word(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn common(format: u32, edition: &str, version: &str) -> Result<Version, String> {
    if format != FORMAT {
        return Err(format!(
            "format {format} is not understood by this version (it reads format {FORMAT})"
        ));
    }
    if !word(edition) {
        return Err("invalid edition".into());
    }
    Version::parse(version).ok_or_else(|| format!("invalid version {version:?}"))
}

/// The unsigned pointer. Only says where the manifest is; nothing in it
/// is trusted.
pub fn parse_pointer(text: &str) -> Result<Pointer, String> {
    let r: RawPointer = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let version = common(r.format, &r.edition, &r.version)?;
    if !valid_store_path(&r.manifest) {
        return Err("the manifest is not a store path".into());
    }
    Ok(Pointer {
        edition: r.edition,
        version,
        manifest: PathBuf::from(r.manifest),
    })
}

/// The manifest, read from the store after Nix checked its signature.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let r: RawManifest = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let version = common(r.format, &r.edition, &r.version)?;
    let mut systems = BTreeMap::new();
    for (k, v) in r.systems {
        if !word(&k) || !valid_store_path(&v) {
            return Err(format!("invalid system entry {k:?}"));
        }
        systems.insert(k, PathBuf::from(v));
    }
    if systems.is_empty() {
        return Err("the manifest lists no system".into());
    }
    Ok(Manifest {
        edition: r.edition,
        version,
        systems,
    })
}

/// Why an authentic update is not applied to this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Unusable,
    WrongEdition { bundle: String, system: String },
    NotNewer { bundle: Version, running: Version },
    NoSystem(String),
    BadRunningVersion(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Unusable => write!(
                f,
                "this system cannot take updates (no release key, or not an installed system)"
            ),
            Refusal::WrongEdition { bundle, system } => write!(
                f,
                "this update is for the {bundle} edition; this computer runs {system}"
            ),
            Refusal::NotNewer { bundle, running } => write!(
                f,
                "this update is version {bundle}, not newer than the running {running} \
                 (to go back, choose an earlier system generation)"
            ),
            Refusal::NoSystem(v) => write!(f, "this update has no system for {v}"),
            Refusal::BadRunningVersion(v) => write!(f, "the running version {v:?} is not valid"),
        }
    }
}

/// The new toplevel for this computer, if the update applies to it.
pub fn plan(m: &Manifest, s: &UpdateSettings) -> Result<PathBuf, Refusal> {
    if !s.usable() {
        return Err(Refusal::Unusable);
    }
    if m.edition != s.edition {
        return Err(Refusal::WrongEdition {
            bundle: m.edition.clone(),
            system: s.edition.clone(),
        });
    }
    let running =
        Version::parse(&s.version).ok_or_else(|| Refusal::BadRunningVersion(s.version.clone()))?;
    if m.version <= running {
        return Err(Refusal::NotNewer {
            bundle: m.version,
            running,
        });
    }
    m.systems
        .get(&s.variant)
        .cloned()
        .ok_or_else(|| Refusal::NoSystem(s.variant.clone()))
}

/// Edition and version a toplevel announces in its
/// `etc/resilience/system.toml`.
pub fn announced(system_toml: &str) -> Option<(String, Version)> {
    #[derive(Deserialize)]
    struct Head {
        edition: String,
        version: String,
    }
    let h: Head = toml::from_str(system_toml).ok()?;
    Some((h.edition, Version::parse(&h.version)?))
}

/// A canonical bundle folder the helper accepts: on removable media
/// mounted by udisks (`/run/media/<user>/<label>/…`), at most a few
/// levels down.
pub fn bundle_dir_allowed(canonical: &Path) -> bool {
    let Ok(rest) = canonical.strip_prefix(MEDIA_ROOT) else {
        return false;
    };
    let n = rest.components().count();
    canonical.is_absolute()
        && (2..=6).contains(&n)
        && rest
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// `file://` URL of a bundle folder for Nix. Bytes outside a safe set are
/// percent-encoded (stick labels may contain spaces).
pub fn file_url(dir: &Path) -> String {
    let mut s = String::from("file://");
    for &b in dir.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"/._~-".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

fn nix_head(nix: &Path) -> Vec<String> {
    vec![
        nix.to_string_lossy().into_owned(),
        "--extra-experimental-features".into(),
        "nix-command".into(),
    ]
}

fn trust(keys: &[String]) -> Vec<String> {
    vec![
        "--option".into(),
        "require-sigs".into(),
        "true".into(),
        "--option".into(),
        "trusted-public-keys".into(),
        keys.join(" "),
    ]
}

/// Import one store path (and whatever of its closure is missing) from
/// the bundle, signatures required.
pub fn copy_argv(s: &UpdateSettings, bundle: &Path, path: &Path) -> Vec<String> {
    let mut v = nix_head(&s.nix);
    v.extend(["copy".into(), "--from".into(), file_url(bundle)]);
    v.extend(trust(&s.trusted_keys));
    v.push(path.to_string_lossy().into_owned());
    v
}

/// Check every path of the bundle (contents and signatures) without
/// importing anything.
pub fn verify_argv(s: &UpdateSettings, bundle: &Path) -> Vec<String> {
    let mut v = nix_head(&s.nix);
    v.extend([
        "store".into(),
        "verify".into(),
        "--store".into(),
        file_url(bundle),
    ]);
    v.extend(trust(&s.trusted_keys));
    v.extend(["--sigs-needed".into(), "1".into(), "--all".into()]);
    v
}

/// Whether `path` is in the bundle at all (`nix store verify --all` on a
/// folder without any path succeeds: the check must also find the
/// manifest).
pub fn present_argv(s: &UpdateSettings, bundle: &Path, path: &Path) -> Vec<String> {
    let mut v = nix_head(&s.nix);
    v.extend([
        "path-info".into(),
        "--store".into(),
        file_url(bundle),
        path.to_string_lossy().into_owned(),
    ]);
    v
}

/// What `nix store verify` found (its exit status is a bit set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckResult {
    /// Every file matches the hash it was signed with.
    pub intact: bool,
    /// Every path is signed by a release key.
    pub authentic: bool,
    /// Nix failed for another reason (unreadable stick, not a bundle).
    pub failed: bool,
}

impl CheckResult {
    pub fn from_exit(code: Option<i32>) -> CheckResult {
        match code {
            Some(c) if (0..8).contains(&c) => CheckResult {
                intact: c & 1 == 0,
                authentic: c & 2 == 0,
                failed: c & 4 != 0,
            },
            _ => CheckResult {
                intact: false,
                authentic: false,
                failed: true,
            },
        }
    }

    pub fn good(self) -> bool {
        self.intact && self.authentic && !self.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "1rvx69m7j5wd3g2cwpdhcxnz7cpjlzk7";
    const KEY: &str = "resilience-1:q7BpySi/HVuhHVoI3ItwWPLezKuu9eI6JTw1s46B6wW=";

    fn sp(name: &str) -> String {
        format!("/nix/store/{H}-{name}")
    }

    fn settings() -> UpdateSettings {
        UpdateSettings {
            nix: "/nix/store/x-nix/bin/nix".into(),
            trusted_keys: vec![KEY.into()],
            variant: "installed-standard-uefi-luks".into(),
            edition: "standard".into(),
            version: "0.3.0".into(),
        }
    }

    fn manifest(edition: &str, version: &str) -> String {
        format!(
            "format = 1\nedition = \"{edition}\"\nversion = \"{version}\"\n[systems]\n\
             installed-standard-uefi-luks = \"{}\"\ninstalled-standard-bios-plain = \"{}\"\n",
            sp("nixos-system-resilience-a"),
            sp("nixos-system-resilience-b")
        )
    }

    #[test]
    fn versions() {
        assert_eq!(Version::parse("0.3.0"), Some(Version(0, 3, 0)));
        assert_eq!(Version::parse("10.20.30"), Some(Version(10, 20, 30)));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.x",
            "1.2.-3",
            " 1.2.3",
            "1.2.3-rc1",
            "1..3",
            "9999999999.0.0",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
    }

    #[test]
    fn store_paths() {
        assert!(valid_store_path(&sp("resilience-update-standard-0.4.0")));
        assert!(!valid_store_path(&format!("{}/etc", sp("x"))));
        assert!(!valid_store_path(&sp(".hidden")));
        assert!(!valid_store_path(&sp("")));
        assert!(!valid_store_path(
            "/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-x"
        )); // 'e' is not base-32
        assert!(!valid_store_path("/nix/store/short-x"));
        assert!(!valid_store_path(&format!("/tmp/{H}-x")));
        assert!(!valid_store_path(&sp("a b")));
        assert!(!valid_store_path(&sp("../../etc")));
    }

    #[test]
    fn keys() {
        assert!(valid_key(KEY));
        assert!(!valid_key("resilience-1"));
        assert!(!valid_key(":q7BpySi/HVuhHVoI3ItwWPLezKuu9eI6JTw1s46B6wW="));
        assert!(!valid_key(
            "r e:q7BpySi/HVuhHVoI3ItwWPLezKuu9eI6JTw1s46B6wW="
        ));
        assert!(!valid_key("r:short="));
        let mut s = settings();
        assert!(s.usable());
        s.trusted_keys.clear();
        assert!(!s.usable());
        s.trusted_keys = vec!["nonsense".into()];
        assert!(!s.usable());
    }

    #[test]
    fn pointer() {
        let text = format!(
            "format = 1\nedition = \"standard\"\nversion = \"0.4.0\"\nmanifest = \"{}\"\n",
            sp("resilience-update-standard-0.4.0")
        );
        let p = parse_pointer(&text).expect("valid");
        assert_eq!(p.version, Version(0, 4, 0));
        assert!(
            parse_pointer(&text.replace("format = 1", "format = 2"))
                .expect_err("refused")
                .contains("format 2")
        );
        assert!(parse_pointer(&text.replace(H, "zzz")).is_err());
        assert!(parse_pointer(&format!("{text}extra = 1\n")).is_err());
        assert!(parse_pointer(&text.replace("standard", "Standard!")).is_err());
    }

    #[test]
    fn manifest_parsing() {
        let m = parse_manifest(&manifest("standard", "0.4.0")).expect("valid");
        assert_eq!(m.systems.len(), 2);
        assert!(
            parse_manifest("format = 1\nedition = \"standard\"\nversion = \"0.4.0\"\n[systems]\n")
                .is_err()
        );
        let bad = manifest("standard", "0.4.0").replace("installed-standard-bios-plain", "../x");
        assert!(parse_manifest(&bad).is_err());
    }

    #[test]
    fn plan_rules() {
        let s = settings();
        let m = parse_manifest(&manifest("standard", "0.4.0")).expect("valid");
        assert_eq!(
            plan(&m, &s).expect("valid"),
            PathBuf::from(sp("nixos-system-resilience-a"))
        );
        let same = parse_manifest(&manifest("standard", "0.3.0")).expect("valid");
        assert!(matches!(plan(&same, &s), Err(Refusal::NotNewer { .. })));
        let older = parse_manifest(&manifest("standard", "0.2.9")).expect("valid");
        assert!(
            plan(&older, &s)
                .expect_err("refused")
                .to_string()
                .contains("earlier system generation")
        );
        let minimal = parse_manifest(&manifest("minimal", "0.4.0")).expect("valid");
        assert!(matches!(
            plan(&minimal, &s),
            Err(Refusal::WrongEdition { .. })
        ));
        let mut other = s.clone();
        other.variant = "installed-standard-bios-luks".into();
        assert_eq!(
            plan(&m, &other),
            Err(Refusal::NoSystem("installed-standard-bios-luks".into()))
        );
        let mut nokey = s.clone();
        nokey.trusted_keys.clear();
        assert_eq!(plan(&m, &nokey), Err(Refusal::Unusable));
        let mut badver = s;
        badver.version = "dev".into();
        assert!(matches!(
            plan(&m, &badver),
            Err(Refusal::BadRunningVersion(_))
        ));
    }

    #[test]
    fn announced_versions() {
        assert_eq!(
            announced("edition = \"standard\"\nversion = \"0.4.0\"\ncatalog = \"/x\"\n"),
            Some(("standard".into(), Version(0, 4, 0)))
        );
        assert_eq!(announced("edition = \"standard\"\n"), None);
    }

    #[test]
    fn bundle_locations() {
        let ok = |p: &str| bundle_dir_allowed(Path::new(p));
        assert!(ok("/run/media/user/STICK/resilience-update"));
        assert!(ok("/run/media/user/My Stick/updates/resilience-update"));
        assert!(!ok("/run/media/user"));
        assert!(!ok("/home/user/resilience-update"));
        assert!(!ok("/run/media/../../etc/x"));
        assert!(!ok("/run/mediax/user/STICK/resilience-update"));
        assert!(!ok("/run/media/u/a/b/c/d/e/f/g"));
    }

    #[test]
    fn urls_and_argv() {
        assert_eq!(
            file_url(Path::new("/run/media/u/My Stick/resilience-update")),
            "file:///run/media/u/My%20Stick/resilience-update"
        );
        assert_eq!(file_url(Path::new("/a/é")), "file:///a/%C3%A9");
        let s = settings();
        let b = Path::new("/run/media/u/S/resilience-update");
        let c = copy_argv(&s, b, Path::new(&sp("m")));
        assert_eq!(
            c[3..6],
            ["copy", "--from", "file:///run/media/u/S/resilience-update"]
        );
        assert!(
            c.windows(3)
                .any(|w| w == ["--option", "require-sigs", "true"])
        );
        assert!(c.windows(2).any(|w| w == ["trusted-public-keys", KEY]));
        assert_eq!(c.last().expect("valid"), &sp("m"));
        let p = present_argv(&s, b, Path::new(&sp("m")));
        assert_eq!(
            p[3..],
            [
                "path-info",
                "--store",
                "file:///run/media/u/S/resilience-update",
                &sp("m")
            ]
        );
        let v = verify_argv(&s, b);
        assert_eq!(v[3..6], ["store", "verify", "--store"]);
        assert!(v.ends_with(&["--sigs-needed".into(), "1".into(), "--all".into()]));
    }

    #[test]
    fn check_results() {
        assert!(CheckResult::from_exit(Some(0)).good());
        let c = CheckResult::from_exit(Some(1));
        assert!(!c.intact && c.authentic && !c.failed);
        let u = CheckResult::from_exit(Some(2));
        assert!(u.intact && !u.authentic);
        assert!(CheckResult::from_exit(Some(4)).failed);
        assert!(CheckResult::from_exit(None).failed);
        assert!(CheckResult::from_exit(Some(127)).failed);
    }
}
