//! `resilience-admin apply-update DIR` (ADR 0023): install a newer system of
//! this computer's own variant from a signed update bundle on removable
//! media. Nix checks hashes and signatures (release keys from admin.toml
//! only); this code decides what to import and makes the new system the one
//! started next, never the running one.
//!
//! `resilience-admin check-update DIR` (any user, read-only) checks the
//! same bundle without importing anything and reports integrity and
//! authenticity separately (ADR 0023 point 4).

use crate::{Failure, Result, audit, caller, err, hold_interrupts, is_root, lock};
use resilience_core::admin::AdminConfig;
use resilience_core::proc::run_visible;
use resilience_core::update::{self, UpdateSettings};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A bundle has a few thousand files; anything far beyond is not one.
const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 4;

/// Read a small regular file, never through a symbolic link.
fn read_small(path: &Path, limit: u64) -> std::result::Result<String, String> {
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = f.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if meta.len() > limit {
        return Err(format!("{} is too large", path.display()));
    }
    let mut s = String::new();
    f.take(limit + 1)
        .read_to_string(&mut s)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(s)
}

/// Everything in the bundle must be a plain file or folder: Nix reads it
/// as root, so a symbolic link (to a file only root may read) or a FIFO
/// (which would block) is refused before Nix runs. Hard links cannot leave
/// the stick's own file system. Files swapped after this scan are not a
/// concern: only an administrator (password checked by polkit) runs this,
/// and an administrator can already read every file with sudo.
fn scan(dir: &Path) -> std::result::Result<(), String> {
    let mut todo = vec![(dir.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((d, depth)) = todo.pop() {
        for e in fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let e = e.map_err(|e| e.to_string())?;
            seen += 1;
            if seen > MAX_ENTRIES {
                return Err("the folder holds too many files to be an update".into());
            }
            let t = e.file_type().map_err(|e| e.to_string())?;
            if t.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    return Err("the folder is nested too deeply to be an update".into());
                }
                todo.push((e.path(), depth + 1));
            } else if t.is_symlink()
                || t.is_fifo()
                || t.is_socket()
                || t.is_block_device()
                || t.is_char_device()
            {
                return Err(format!(
                    "{} is not a plain file (links and special files are refused)",
                    e.path().display()
                ));
            }
        }
    }
    Ok(())
}

/// The bundle folder a caller named, if it is one the helper accepts.
fn bundle_dir(requested: &str) -> std::result::Result<PathBuf, String> {
    let dir = fs::canonicalize(requested).map_err(|_| "the update folder cannot be found")?;
    if !update::bundle_dir_allowed(&dir) || !dir.is_dir() {
        return Err(format!(
            "updates are installed from a folder on a USB stick or disk mounted under {}",
            update::MEDIA_ROOT
        ));
    }
    Ok(dir)
}

/// Run Nix as the caller (no privilege), output on the terminal. Not in its
/// own process group: Ctrl-C stops the check and Nix together.
fn nix_as_user(argv: &[String]) -> Option<i32> {
    let (program, args) = argv.split_first()?;
    let mut c = Command::new(program);
    c.args(args)
        .env_clear()
        .env("PATH", "/run/current-system/sw/bin");
    if let Ok(h) = std::env::var("HOME") {
        c.env("HOME", h);
    }
    c.stdin(std::process::Stdio::null()).status().ok()?.code()
}

/// `check-update DIR`: read-only, as the caller. Nothing is imported.
pub fn check_update(requested: &str) -> Result<()> {
    let cfg = AdminConfig::load().map_err(err)?;
    let Some(up) = cfg.updates.clone().filter(UpdateSettings::usable) else {
        return Err(Failure::Refused(update::Refusal::Unusable.to_string()));
    };
    let dir = bundle_dir(requested).map_err(Failure::Refused)?;
    scan(&dir).map_err(|e| Failure::Refused(format!("not a valid update: {e}")))?;
    let pointer = read_small(&dir.join(update::POINTER_FILE), update::MAX_TOML)
        .and_then(|t| update::parse_pointer(&t))
        .map_err(|e| {
            Failure::Refused(format!(
                "not a valid update ({}): {e}",
                update::POINTER_FILE
            ))
        })?;
    println!(
        "The stick says: Resilience {} {} (not yet checked).",
        pointer.edition, pointer.version
    );
    println!(
        "This computer runs: Resilience {} {}.",
        up.edition, up.version
    );
    println!();
    println!("Checking every file of the update against its signed hash.");
    println!("This reads the whole update and can take several minutes...");
    let present = nix_as_user(&update::present_argv(&up, &dir, &pointer.manifest)) == Some(0);
    let r = update::CheckResult::from_exit(nix_as_user(&update::verify_argv(&up, &dir)));
    println!();
    let yes_no = |b: bool| if b { "yes" } else { "NO" };
    println!(
        "Intact (no file damaged or changed):      {}",
        yes_no(r.intact && !r.failed)
    );
    println!(
        "Authentic (signed by a release key):      {}",
        yes_no(r.authentic && !r.failed)
    );
    println!(
        "Complete (holds the update's manifest):   {}",
        yes_no(present)
    );
    if !(r.good() && present) {
        return Err(Failure::Refused(
            "this update must not be installed: it is damaged, not signed by a \
             Resilience release key, incomplete, or the stick could not be read"
                .into(),
        ));
    }
    let running = update::Version::parse(&up.version);
    if pointer.edition != up.edition {
        println!();
        println!(
            "It is for the {} edition, not this computer's: it would be refused.",
            pointer.edition
        );
    } else if running.is_some_and(|v| pointer.version <= v) {
        println!();
        println!("It is not newer than the running version: it would be refused.");
    } else {
        println!();
        println!("The update is intact and authentic. It can be installed.");
    }
    Ok(())
}

pub fn apply_update(requested: &str) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let running = cfg
        .updates
        .as_ref()
        .map_or_else(|| "unknown".to_string(), |u| u.version.clone());
    let base = [
        ("op", "apply-update"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("from", running.as_str()),
        ("bundle", requested),
    ];
    let outcome = |extra: &[(&str, &str)]| {
        let mut f = base.to_vec();
        f.extend_from_slice(extra);
        audit(&cfg, &f);
    };
    let refuse = |reason: String| {
        outcome(&[("result", "refused"), ("reason", &reason)]);
        Failure::Refused(reason)
    };
    let Some(up) = cfg.updates.clone().filter(UpdateSettings::usable) else {
        return Err(refuse(update::Refusal::Unusable.to_string()));
    };
    let _lock = lock(&cfg)?;

    // 1. Where the bundle is.
    let dir = bundle_dir(requested).map_err(refuse)?;
    scan(&dir).map_err(|e| refuse(format!("not a valid update: {e}")))?;

    // 2. The unsigned pointer: only says which store path is the manifest.
    let pointer = read_small(&dir.join(update::POINTER_FILE), update::MAX_TOML)
        .and_then(|t| update::parse_pointer(&t))
        .map_err(|e| {
            refuse(format!(
                "not a valid update ({}): {e}",
                update::POINTER_FILE
            ))
        })?;

    // Nothing from the caller's environment reaches Nix.
    let env = [("PATH", "/run/current-system/sw/bin"), ("HOME", "/root")];
    let import = |path: &Path, timeout: Duration| {
        let argv = update::copy_argv(&up, &dir, path);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        run_visible(&argv, &env, timeout) == Some(0)
    };

    // 3. The manifest, checked by Nix (hash and release-key signature).
    println!("Checking the update's signature (release key) and integrity...");
    if !import(&pointer.manifest, Duration::from_secs(600)) {
        return Err(refuse(
            "the update is not signed by a Resilience release key, or it is damaged \
             (see Nix's message above); nothing was installed"
                .into(),
        ));
    }
    let manifest = read_small(&pointer.manifest, update::MAX_TOML)
        .and_then(|t| update::parse_manifest(&t))
        .map_err(|e| refuse(format!("the signed manifest is not valid: {e}")))?;

    // 4. Does it apply to this computer?
    let target = update::plan(&manifest, &up).map_err(|r| refuse(r.to_string()))?;
    let to = manifest.version.to_string();

    // 5. The new system: only what is missing is copied, every path
    // checked; nothing is registered unless it is intact and signed.
    println!(
        "Importing Resilience {to} for this computer ({}): only what is missing is copied.",
        up.variant
    );
    if !import(&target, Duration::from_secs(4 * 3600)) {
        let reason = "importing the new system failed (see above): damaged or unsigned \
                      files, or not enough free space; nothing was installed";
        outcome(&[("to", &to), ("result", "error"), ("reason", reason)]);
        return Err(err(reason));
    }
    // In a system, etc/resilience/system.toml is a chain of symbolic links
    // inside the store: resolve it, and read it only if it stays there.
    let announced = fs::canonicalize(target.join("etc/resilience/system.toml"))
        .ok()
        .filter(|p| p.starts_with("/nix/store/"))
        .and_then(|p| read_small(&p, 1 << 20).ok())
        .and_then(|t| update::announced(&t));
    if announced != Some((manifest.edition.clone(), manifest.version)) {
        let reason = "the new system does not announce the edition and version of its manifest";
        outcome(&[("to", &to), ("result", "refused"), ("reason", reason)]);
        return Err(Failure::Refused(reason.into()));
    }

    // 6. The system started next. From here on the operation must finish
    // (or undo itself) and be audited.
    hold_interrupts();
    let nix_env = cfg.nix_env.to_string_lossy().into_owned();
    let profile = update::SYSTEM_PROFILE;
    let target_s = target.to_string_lossy().into_owned();
    let sw_path = format!("{target_s}/sw/bin");
    let stc_env = [("PATH", sw_path.as_str()), ("HOME", "/root")];
    println!("Making Resilience {to} the system started next...");
    if run_visible(
        &[&nix_env, "--profile", profile, "--set", &target_s],
        &env,
        Duration::from_secs(120),
    ) != Some(0)
    {
        let reason = "the system profile could not be changed; nothing was changed";
        outcome(&[("to", &to), ("result", "error"), ("reason", reason)]);
        return Err(err(reason));
    }
    let stc = format!("{target_s}/bin/switch-to-configuration");
    if run_visible(&[&stc, "boot"], &stc_env, Duration::from_secs(600)) != Some(0) {
        // Back to the previous generation and its boot menu.
        let back = run_visible(
            &[&nix_env, "--profile", profile, "--rollback"],
            &env,
            Duration::from_secs(120),
        ) == Some(0);
        let menu = back && {
            let prev = format!("{profile}/bin/switch-to-configuration");
            let prev_path = format!("{profile}/sw/bin");
            run_visible(
                &[&prev, "boot"],
                &[("PATH", prev_path.as_str()), ("HOME", "/root")],
                Duration::from_secs(600),
            ) == Some(0)
        };
        let reason = format!(
            "the boot menu could not be written for the new system; {}",
            if menu {
                "the previous system stays the default"
            } else {
                "the previous system could NOT be restored as the default: an administrator \
                 should run 'sudo /nix/var/nix/profiles/system/bin/switch-to-configuration \
                 boot' before restarting"
            }
        );
        outcome(&[("to", &to), ("result", "error"), ("reason", &reason)]);
        return Err(err(reason));
    }
    outcome(&[("to", &to), ("result", "ok")]);
    println!();
    println!("Resilience {to} is installed. It starts at the next restart.");
    println!("The running system is unchanged until then. The previous version stays");
    println!("in the boot menu and in Recovery > System generations.");
    println!("The update is recorded in the audit log.");
    Ok(())
}
