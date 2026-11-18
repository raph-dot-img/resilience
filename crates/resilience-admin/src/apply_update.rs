//! `resilience-admin apply-update DIR` (ADR 0023): install a newer system of
//! this computer's own variant from a signed update bundle on removable
//! media. Nix checks hashes and signatures (release keys from admin.toml
//! only); this code decides what to import and makes the new system the one
//! started next, never the running one.

use crate::{Failure, Result, audit, caller, err, hold_interrupts, is_root, lock};
use resilience_core::admin::AdminConfig;
use resilience_core::proc::run_visible;
use resilience_core::update::{self, UpdateSettings};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
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
    let dir: PathBuf = fs::canonicalize(requested)
        .map_err(|_| refuse("the update folder cannot be found".into()))?;
    if !update::bundle_dir_allowed(&dir) || !dir.is_dir() {
        return Err(refuse(format!(
            "updates are installed from a folder on a USB stick or disk mounted under {}",
            update::MEDIA_ROOT
        )));
    }
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
    let announced = read_small(&target.join("etc/resilience/system.toml"), 1 << 20)
        .ok()
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
