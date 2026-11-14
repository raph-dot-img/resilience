//! `resilience-admin` — the only privileged entry point of Resilience OS.
//!
//! It exposes a fixed set of typed operations (ADR 0007); it never runs a
//! shell or a program chosen by the caller. Privileged operations must run as
//! root, which ordinary users reach only through `pkexec`, with the polkit
//! action `org.resilience.admin.set-mode` (`auth_admin`: an administrator's
//! password every time). Every argument is validated here, server-side,
//! whatever the TUI already checked.
//!
//! ```text
//! resilience-admin list                  any user, read-only
//! resilience-admin set-mode USER MODE    root (pkexec / sudo)
//! resilience-admin apply-state           root (system activation)
//! ```

use resilience_core::admin::{
    self, AdminConfig, Change, ModeState, Source, effective_mode, group_ops, mode_id,
    plan_set_mode, stale_overrides,
};
use resilience_core::mode::{Mode, user_name};
use resilience_core::proc::run_full;
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;
const EXIT_PERMISSION: u8 = 77;

const USAGE: &str = "\
Usage:
  resilience-admin list                  Show managed accounts and their modes
  resilience-admin set-mode USER MODE    Change a security mode (administrator)
  resilience-admin apply-state           Re-apply recorded modes (system use)
MODE is one of: ultra-stable, stable, advanced";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match argv.as_slice() {
        ["list"] => list(),
        ["set-mode", user, mode] => set_mode(user, mode),
        ["apply-state"] => apply_state(),
        ["--version"] => {
            println!("resilience-admin {}", resilience_core::VERSION);
            Ok(())
        }
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(Failure::Usage),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(EXIT_USAGE)
        }
        Err(Failure::Permission) => {
            eprintln!(
                "resilience-admin: this operation needs administrator authorisation.\n\
                 Use it from the Resilience interface (Users and modes), which asks for\n\
                 an administrator password through polkit."
            );
            ExitCode::from(EXIT_PERMISSION)
        }
        Err(Failure::Refused(msg)) => {
            eprintln!("Refused: {msg}.");
            ExitCode::from(EXIT_REFUSED)
        }
        Err(Failure::Error(msg)) => {
            eprintln!("resilience-admin: {msg}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

enum Failure {
    Usage,
    Permission,
    Refused(String),
    Error(String),
}

type Result<T> = std::result::Result<T, Failure>;

fn err(msg: impl Into<String>) -> Failure {
    Failure::Error(msg.into())
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// The account that asked for the operation. pkexec and sudo record the
/// invoking UID; both are only reachable after authentication, and the
/// helper only trusts these variables when it really runs as root.
fn caller() -> (u32, String) {
    let uid = ["PKEXEC_UID", "SUDO_UID"]
        .iter()
        .find_map(|v| std::env::var(v).ok()?.parse::<u32>().ok())
        // SAFETY: getuid has no preconditions and cannot fail.
        .unwrap_or_else(|| unsafe { libc::getuid() });
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let name = user_name(uid, &passwd).unwrap_or_else(|| format!("uid{uid}"));
    (uid, name)
}

fn load_state(cfg: &AdminConfig) -> Result<ModeState> {
    match fs::read_to_string(&cfg.state_file) {
        Ok(text) => ModeState::parse(&text).map_err(|e| {
            err(format!(
                "{} is corrupt ({e}); refusing to change anything. Inspect or remove it as an administrator.",
                cfg.state_file.display()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ModeState::default()),
        Err(e) => Err(err(format!("{}: {e}", cfg.state_file.display()))),
    }
}

/// Write the state atomically: temporary file, fsync, rename, fsync dir.
fn save_state(cfg: &AdminConfig, state: &ModeState) -> Result<()> {
    let path = &cfg.state_file;
    let dir = path
        .parent()
        .ok_or_else(|| err("state file has no directory"))?;
    fs::create_dir_all(dir).map_err(|e| err(format!("{}: {e}", dir.display())))?;
    let tmp = path.with_extension("toml.tmp");
    let io = |e: std::io::Error| err(format!("{}: {e}", tmp.display()));
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&tmp)
            .map_err(io)?;
        f.set_permissions(fs::Permissions::from_mode(0o644))
            .map_err(io)?;
        f.write_all(state.to_toml().as_bytes()).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    fs::rename(&tmp, path).map_err(io)?;
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Append one entry to the audit log (mode 0640, root:wheel via tmpfiles)
/// and send it to the system journal (authpriv facility).
fn audit(cfg: &AdminConfig, fields: &[(&str, &str)]) {
    let line = admin::audit_line(&admin::now_rfc3339(), fields);
    let written = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o640)
        .open(&cfg.audit_log)
        .and_then(|mut f| {
            f.write_all(line.as_bytes())?;
            f.sync_all()
        });
    if let Err(e) = written {
        eprintln!("resilience-admin: WARNING: could not write the audit log: {e}");
    }
    syslog(line.trim_end());
}

fn syslog(msg: &str) {
    let ident = c"resilience-admin";
    let Ok(msg) = CString::new(msg.replace('\0', "")) else {
        return;
    };
    // SAFETY: `ident` is a 'static C string (openlog keeps the pointer); the
    // message is passed as an argument to a constant "%s" format, never as
    // the format itself.
    unsafe {
        libc::openlog(ident.as_ptr(), libc::LOG_PID, libc::LOG_AUTHPRIV);
        libc::syslog(libc::LOG_NOTICE, c"%s".as_ptr(), msg.as_ptr());
        libc::closelog();
    }
}

/// Exclusive lock so two invocations never interleave state updates.
fn lock(cfg: &AdminConfig) -> Result<fs::File> {
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&cfg.lock_file)
        .map_err(|e| err(format!("{}: {e}", cfg.lock_file.display())))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // SAFETY: flock on a file descriptor we own; non-blocking.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(f);
        }
        if Instant::now() >= deadline {
            return Err(err(
                "another administrative operation is running; try again",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn group_file() -> Result<String> {
    fs::read_to_string("/etc/group").map_err(|e| err(format!("/etc/group: {e}")))
}

/// Make the managed groups of `user` match `mode`, using gpasswd only.
fn apply_groups(cfg: &AdminConfig, user: &str, mode: Mode) -> Result<usize> {
    let ops = group_ops(&group_file()?, user, mode);
    let gpasswd = cfg.gpasswd.to_string_lossy();
    for op in &ops {
        let a = op.gpasswd_args();
        let out = run_full(
            &[gpasswd.as_ref(), a[0], a[1], a[2]],
            Duration::from_secs(10),
        )
        .ok_or_else(|| err("gpasswd did not finish"))?;
        if !out.success() {
            return Err(err(format!(
                "gpasswd {}: {}",
                a.join(" "),
                out.stderr.trim()
            )));
        }
    }
    Ok(ops.len())
}

fn list() -> Result<()> {
    let cfg = AdminConfig::load().map_err(err)?;
    let state = load_state(&cfg)?;
    let groups = fs::read_to_string("/etc/group").unwrap_or_default();
    println!(
        "{:<16} {:<13} {:<13} {:<13} NOTE",
        "USER", "DECLARED", "EFFECTIVE", "ACTIVE GROUPS"
    );
    for r in admin::overview(&cfg, &state, &groups) {
        let mut notes = Vec::new();
        if let (Source::Override, Some(o)) = (r.source, state.users.get(&r.name)) {
            notes.push(format!("changed {} by {}", o.changed, o.by));
        }
        if r.active != r.effective {
            notes.push("group database differs; re-applied at next boot".into());
        }
        if r.autologin {
            notes.push("autologin".into());
        }
        println!(
            "{:<16} {:<13} {:<13} {:<13} {}",
            r.name,
            mode_id(r.declared),
            mode_id(r.effective),
            mode_id(r.active),
            notes.join("; ")
        );
    }
    Ok(())
}

fn set_mode(user: &str, mode: &str) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let _lock = lock(&cfg)?;
    let refuse = |reason: String| {
        audit(
            &cfg,
            &[
                ("op", "set-mode"),
                ("caller", &by),
                ("caller_uid", &uid_s),
                // As requested (quoted and escaped by audit_line if unusual).
                ("target", user),
                ("to", mode),
                ("result", "refused"),
                ("reason", &reason),
            ],
        );
        Failure::Refused(reason)
    };
    let Some(to) = admin::parse_mode(mode) else {
        return Err(refuse(format!("unknown mode {mode:?}")));
    };
    let mut state = load_state(&cfg)?;
    let change: Change =
        plan_set_mode(&cfg, &state, user, to).map_err(|r| refuse(r.to_string()))?;

    admin::record(&mut state, &cfg, &change, &by, &admin::now_rfc3339());
    save_state(&cfg, &state)?;
    let applied = apply_groups(&cfg, &change.user, change.to);
    let fields = [
        ("op", "set-mode"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("target", change.user.as_str()),
        ("from", mode_id(change.from)),
        ("to", mode_id(change.to)),
    ];
    if let Err(Failure::Error(e)) = &applied {
        let mut f = fields.to_vec();
        f.extend([("result", "error"), ("reason", e.as_str())]);
        audit(&cfg, &f);
        return Err(err(format!(
            "the new mode was recorded but applying it failed: {e}. It will be re-applied at next boot."
        )));
    }
    let mut f = fields.to_vec();
    f.push(("result", "ok"));
    audit(&cfg, &f);

    println!(
        "{} is now in {} mode (was {}).",
        change.user, change.to, change.from
    );
    println!("The change is recorded in the audit log and kept across reboots.");
    if change.is_demotion() {
        println!(
            "Privileges were reduced, so all sessions of {} are being closed now.",
            change.user
        );
        let _ = std::io::stdout().flush();
        let loginctl = cfg.loginctl.to_string_lossy();
        // Last step: this may end the session this helper runs in.
        let _ = run_full(
            &[loginctl.as_ref(), "terminate-user", &change.user],
            Duration::from_secs(10),
        );
    } else {
        println!("{} receives the new rights at the next login.", change.user);
    }
    Ok(())
}

fn apply_state() -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let _lock = lock(&cfg)?;
    let mut state = match load_state(&cfg) {
        Ok(s) => s,
        Err(e) => {
            // Keep the declared modes (from the system activation) and leave
            // the corrupt file for an administrator to inspect.
            if let Failure::Error(msg) = &e {
                audit(
                    &cfg,
                    &[("op", "apply-state"), ("result", "error"), ("reason", msg)],
                );
            }
            return Err(e);
        }
    };
    let stale = stale_overrides(&cfg, &state);
    for name in &stale {
        state.users.remove(name);
        audit(
            &cfg,
            &[
                ("op", "apply-state"),
                ("target", name),
                ("result", "override-dropped"),
                (
                    "reason",
                    "declared mode changed in the system configuration",
                ),
            ],
        );
    }
    if !stale.is_empty() {
        save_state(&cfg, &state)?;
    }
    let mut changed = 0;
    for name in state.users.keys() {
        if let Some((mode, Source::Override)) = effective_mode(&cfg, &state, name) {
            let n = apply_groups(&cfg, name, mode)?;
            changed += n;
            if n > 0 {
                audit(
                    &cfg,
                    &[
                        ("op", "apply-state"),
                        ("target", name),
                        ("to", mode_id(mode)),
                        ("result", "ok"),
                    ],
                );
            }
        }
    }
    println!(
        "resilience-admin: {} override(s) applied, {} group change(s), {} stale override(s) dropped",
        state.users.len(),
        changed,
        stale.len()
    );
    Ok(())
}
