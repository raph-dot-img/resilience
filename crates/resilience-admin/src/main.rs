//! `resilience-admin` — the only privileged entry point of Resilience OS.
//!
//! It exposes a fixed set of typed operations (ADR 0007); it never runs a
//! shell or a program chosen by the caller. Privileged operations must run as
//! root, which ordinary users reach only through `pkexec`, with the polkit
//! action per operation (`org.resilience.admin.set-mode`,
//! `org.resilience.admin.set-time`,
//! `org.resilience.admin.set-default-generation`; `auth_admin`: an
//! administrator's password every time; and `org.resilience.admin.set-keymap`,
//! allowed without a password to the active local session, ADR 0017). Every argument is validated here, server-side, whatever the
//! TUI already checked.
//!
//! ```text
//! resilience-admin list                  any user, read-only
//! resilience-admin set-mode USER MODE    root (pkexec / sudo)
//! resilience-admin set-time UTC-TIME     root (pkexec / sudo), ADR 0009
//! resilience-admin set-default-generation N   root (pkexec / sudo), ADR 0011
//! resilience-admin set-keymap LAYOUT     root (pkexec, no password), ADR 0017
//! resilience-admin set-mesh on|off       root (pkexec, no password), ADR 0021
//! resilience-admin record-licence SHA256  root (pkexec, no password), ADR 0024
//! resilience-admin check-update DIR      any user, read-only, ADR 0023
//! resilience-admin apply-update DIR      root (pkexec / sudo), ADR 0023
//! resilience-admin apply-state           root (system activation)
//! ```

mod apply_update;

use resilience_core::admin::{
    self, AdminConfig, Change, ModeState, Source, effective_mode, group_ops, mode_id,
    plan_set_mode, stale_overrides,
};
use resilience_core::clock;
use resilience_core::mode::{Mode, user_name};
use resilience_core::proc::{run_full, run_visible};
use resilience_core::recovery::{self, DefaultPlan};
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
  resilience-admin set-time TIME         Set the clock; TIME is YYYY-MM-DDTHH:MM:SSZ (UTC)
  resilience-admin set-default-generation N
                                         Start system generation N at every boot
  resilience-admin set-keymap LAYOUT     Load a console keyboard layout of this system
  resilience-admin set-mesh on|off       Turn the Wi-Fi mesh on or off (ADR 0021)
  resilience-admin record-licence SHA256 Record that you read the licence (ADR 0024)
  resilience-admin check-update DIR      Check the update in DIR without installing it
  resilience-admin apply-update DIR      Install the update in DIR (USB stick, ADR 0023)
  resilience-admin apply-state           Re-apply recorded modes (system use)
MODE is one of: ultra-stable, stable, advanced";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match argv.as_slice() {
        ["list"] => list(),
        ["set-mode", user, mode] => set_mode(user, mode),
        ["set-time", time] => set_time(time),
        ["set-default-generation", n] => set_default_generation(n),
        ["set-keymap", id] => set_keymap(id),
        ["set-mesh", state] => set_mesh(state),
        ["record-licence", hash] => record_licence(hash),
        ["check-update", dir] => apply_update::check_update(dir),
        ["apply-update", dir] => apply_update::apply_update(dir),
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

    hold_interrupts();
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

/// Load one of the system's console keyboard layouts (ADR 0017). The
/// console has one keymap for all its consoles; it is not saved here (the
/// TUI loads each account's choice at login, and the system's layout is
/// loaded again before every login prompt).
/// ADR 0024: append "this account certified it read this licence text,
/// at this time" to the root-owned record. The hash must be the one of the
/// text this system shows, so a record always names the exact text.
fn record_licence(requested: &str) -> Result<()> {
    use resilience_core::licence;
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let base = [
        ("op", "record-licence"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("sha256", requested),
    ];
    let outcome = |extra: &[(&str, &str)]| {
        let mut f = base.to_vec();
        f.extend_from_slice(extra);
        audit(&cfg, &f);
    };
    let refuse = |reason: &str| {
        outcome(&[("result", "refused"), ("reason", reason)]);
        Failure::Refused(reason.into())
    };
    let Some(l) = cfg.licence.clone() else {
        return Err(refuse("this system has no licence notice"));
    };
    if !licence::valid_sha256(requested) {
        return Err(refuse("not a SHA-256 hash"));
    }
    if requested != l.sha256 {
        return Err(refuse(
            "this is not the licence text of this system (it may have been updated); \
             read it again",
        ));
    }
    let _lock = lock(&cfg)?;
    let now = admin::now_rfc3339();
    let line = admin::audit_line(&now, &licence::record_fields(&by, &uid_s, &l));
    let written = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&l.record)
        .and_then(|mut f| {
            f.write_all(line.as_bytes())?;
            f.sync_all()
        });
    if let Err(e) = written {
        let reason = format!("{}: {e}", l.record.display());
        outcome(&[("result", "error"), ("reason", &reason)]);
        return Err(err(reason));
    }
    outcome(&[("result", "ok")]);
    println!(
        "Recorded: {by} read the {} on {now} (this computer's clock).",
        l.title()
    );
    Ok(())
}

fn set_keymap(requested: &str) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let _lock = lock(&cfg)?;
    let base = [
        ("op", "set-keymap"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("to", requested),
    ];
    let outcome = |extra: &[(&str, &str)]| {
        let mut f = base.to_vec();
        f.extend_from_slice(extra);
        audit(&cfg, &f);
    };
    let argv = match admin::plan_set_keymap(&cfg, requested) {
        Ok(a) => a,
        Err(r) => {
            let reason = r.to_string();
            outcome(&[("result", "refused"), ("reason", &reason)]);
            return Err(Failure::Refused(reason));
        }
    };
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    match run_full(&args, Duration::from_secs(10)) {
        Some(o) if o.success() => {
            outcome(&[("result", "ok")]);
            println!("Keyboard layout {requested} loaded.");
            Ok(())
        }
        Some(o) => {
            let reason = o.stderr.trim().to_string();
            outcome(&[("result", "error"), ("reason", &reason)]);
            Err(Failure::Error(format!("loadkeys failed: {reason}")))
        }
        None => {
            outcome(&[("result", "error"), ("reason", "loadkeys did not finish")]);
            Err(Failure::Error("loadkeys did not finish".into()))
        }
    }
}

/// Turn the system's mesh network on or off (ADR 0021): start or stop its
/// unit, nothing else. Allowed to the person at the console without a
/// password (polkit), like joining a Wi-Fi network.
fn set_mesh(requested: &str) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let _lock = lock(&cfg)?;
    let base = [
        ("op", "set-mesh"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("to", requested),
    ];
    let outcome = |extra: &[(&str, &str)]| {
        let mut f = base.to_vec();
        f.extend_from_slice(extra);
        audit(&cfg, &f);
    };
    let argv = match admin::plan_set_mesh(&cfg, requested) {
        Ok(a) => a,
        Err(r) => {
            let reason = r.to_string();
            outcome(&[("result", "refused"), ("reason", &reason)]);
            return Err(Failure::Refused(reason));
        }
    };
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    match run_full(&args, Duration::from_secs(90)) {
        Some(o) if o.success() => {
            outcome(&[("result", "ok")]);
            println!(
                "The mesh network is {}.",
                if requested == "on" { "on" } else { "off" }
            );
            Ok(())
        }
        Some(o) => {
            let reason = o.stderr.trim().to_string();
            outcome(&[("result", "error"), ("reason", &reason)]);
            Err(Failure::Error(format!(
                "the mesh could not be changed: {reason}"
            )))
        }
        None => {
            outcome(&[("result", "error"), ("reason", "systemctl did not finish")]);
            Err(Failure::Error("systemctl did not finish".into()))
        }
    }
}

/// Set the system clock and the hardware clock to an exact UTC time.
/// The time is validated here (strict format, 2025 <= t < 2100), then
/// handed to systemd's timedatectl as `@SECONDS`; timedated writes the
/// RTC in the configured mode (UTC or local) and refuses while NTP is on.
fn set_time(requested: &str) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let _lock = lock(&cfg)?;
    let before = admin::now_rfc3339();
    let base = [
        ("op", "set-time"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("from", before.as_str()),
    ];
    let outcome = |extra: &[(&str, &str)]| {
        let mut f = base.to_vec();
        f.extend_from_slice(extra);
        audit(&cfg, &f);
    };
    let t = match clock::parse_utc(requested) {
        Ok(t) => t,
        Err(r) => {
            let reason = r.to_string();
            // As requested (quoted and escaped by audit_line if unusual).
            outcome(&[
                ("to", requested),
                ("result", "refused"),
                ("reason", &reason),
            ]);
            return Err(Failure::Refused(reason));
        }
    };
    let to = admin::rfc3339_utc(t);
    hold_interrupts();
    let timedatectl = cfg.timedatectl.to_string_lossy();
    let at = format!("@{t}");
    let out = run_full(
        &[timedatectl.as_ref(), "--no-ask-password", "set-time", &at],
        Duration::from_secs(20),
    );
    match out {
        Some(o) if o.success() => {
            outcome(&[("to", &to), ("result", "ok")]);
            println!("The clock is now set to {to} (UTC).");
            println!("The hardware clock was updated too, so the time is kept after a restart.");
            println!("The change is recorded in the audit log.");
            Ok(())
        }
        Some(o) => {
            let reason = o.stderr.trim().to_string();
            outcome(&[("to", &to), ("result", "error"), ("reason", &reason)]);
            let hint = if reason.contains("NTP") || reason.contains("synchroni") {
                "\nAutomatic time synchronisation (NTP) is on; it sets the clock itself."
            } else {
                ""
            };
            Err(err(format!("timedatectl refused: {reason}{hint}")))
        }
        None => {
            outcome(&[
                ("to", &to),
                ("result", "error"),
                ("reason", "timedatectl did not finish"),
            ]);
            Err(err(
                "timedatectl did not finish; the clock may be unchanged",
            ))
        }
    }
}

/// Make generation `requested` the one started at every boot, without
/// changing the running system: point the system profile at it (nix-env
/// --switch-generation, as `nixos-rebuild --rollback` does), then run that
/// generation's own `switch-to-configuration boot`, which writes the boot
/// menu. If writing the boot menu fails, the profile is switched back.
fn set_default_generation(requested: &str) -> Result<()> {
    if !is_root() {
        return Err(Failure::Permission);
    }
    let cfg = AdminConfig::load().map_err(err)?;
    let (uid, by) = caller();
    let uid_s = uid.to_string();
    let _lock = lock(&cfg)?;
    let profiles = std::path::Path::new(recovery::PROFILES_DIR);
    let gens = recovery::generations(
        profiles,
        std::path::Path::new("/run/booted-system"),
        std::path::Path::new("/run/current-system"),
    );
    let before = gens
        .iter()
        .find(|g| g.default)
        .map_or_else(|| "none".to_string(), |g| g.number.to_string());
    let base = [
        ("op", "set-default-generation"),
        ("caller", by.as_str()),
        ("caller_uid", uid_s.as_str()),
        ("from", before.as_str()),
    ];
    let outcome = |extra: &[(&str, &str)]| {
        let mut f = base.to_vec();
        f.extend_from_slice(extra);
        audit(&cfg, &f);
    };
    let plan: DefaultPlan =
        match recovery::plan_set_default(&gens, requested, std::path::Path::new("/nix/store")) {
            Ok(p) => p,
            Err(r) => {
                let reason = r.to_string();
                // As requested (quoted and escaped by audit_line if unusual).
                outcome(&[
                    ("to", requested),
                    ("result", "refused"),
                    ("reason", &reason),
                ]);
                return Err(Failure::Refused(reason));
            }
        };
    let to = plan.number.to_string();
    let profile = recovery::system_profile(profiles);
    let profile_s = profile.to_string_lossy();
    let nix_env = cfg.nix_env.to_string_lossy();
    // Nothing from the caller's environment reaches these programs.
    let sw_path = format!("{}/sw/bin", plan.target.display());
    let env = [("PATH", sw_path.as_str()), ("HOME", "/root")];
    let switch_profile = |n: &str| {
        run_visible(
            &[
                nix_env.as_ref(),
                "--profile",
                profile_s.as_ref(),
                "--switch-generation",
                n,
            ],
            &env,
            Duration::from_secs(60),
        ) == Some(0)
    };

    // From here on the operation must finish (or undo itself) and be
    // audited: Ctrl-C or a closed terminal must not stop it half-way.
    hold_interrupts();
    println!("Making generation {to} the default for every start...");
    let _ = std::io::stdout().flush();
    let expected_link = format!("system-{}-link", plan.number);
    let profile_is_ours =
        || fs::read_link(&profile).is_ok_and(|l| l.as_os_str() == expected_link.as_str());
    if !switch_profile(&to) {
        outcome(&[
            ("to", &to),
            ("result", "error"),
            ("reason", "nix-env --switch-generation failed"),
        ]);
        return Err(err(
            "the system profile could not be switched; nothing was changed",
        ));
    }
    let stc = plan.target.join("bin/switch-to-configuration");
    let stc = stc.to_string_lossy();
    let installed = run_visible(&[stc.as_ref(), "boot"], &env, Duration::from_secs(300));
    if installed != Some(0) {
        // Never undo someone else's change (a nixos-rebuild at the same time).
        let undone = profile_is_ours()
            && plan
                .previous
                .is_some_and(|p| switch_profile(&p.to_string()));
        let reason = format!(
            "switch-to-configuration boot failed ({}); profile {}",
            installed.map_or_else(|| "killed or timed out".into(), |c| format!("exit {c}")),
            if undone {
                "switched back"
            } else {
                "NOT switched back"
            }
        );
        outcome(&[("to", &to), ("result", "error"), ("reason", &reason)]);
        // The profile's own switch-to-configuration writes the boot menu for
        // whatever the profile now points to.
        return Err(err(if undone {
            "the boot menu could not be written; the previous default was restored. \
             The boot menu may need repair: an administrator can run \
             'sudo /nix/var/nix/profiles/system/bin/switch-to-configuration boot'."
        } else {
            "the boot menu could not be written and the previous default was not \
             restored. An administrator should run \
             'sudo /nix/var/nix/profiles/system/bin/switch-to-configuration boot' before restarting."
        }));
    }
    if !profile_is_ours() {
        let reason = "the system profile changed during the operation (another system change?)";
        outcome(&[("to", &to), ("result", "error"), ("reason", reason)]);
        return Err(err(format!(
            "{reason}. Check the boot menu before restarting: an administrator can run \
             'sudo /nix/var/nix/profiles/system/bin/switch-to-configuration boot'."
        )));
    }
    outcome(&[("to", &to), ("result", "ok")]);
    println!();
    println!("Generation {to} is now started by default (it was {before}).");
    println!("The running system is unchanged until the next restart.");
    println!("Newer generations stay available in the boot menu.");
    println!("The change is recorded in the audit log.");
    Ok(())
}

/// Ignore Ctrl-C, Ctrl-\\, Ctrl-Z and hang-up from now on, in this process
/// and (inherited across exec) in the programs it starts. Called once a
/// request has been validated, so a change is never left half-done or
/// unaudited; the request can still be cancelled at the password prompt.
fn hold_interrupts() {
    for sig in [libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP, libc::SIGHUP] {
        // SAFETY: installing SIG_IGN has no preconditions; no handler runs.
        unsafe {
            libc::signal(sig, libc::SIG_IGN);
        }
    }
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
        let raised = state
            .users
            .remove(name)
            .is_some_and(|o| o.mode != Mode::UltraStable);
        let reason = if cfg.ultra_stable_only && raised {
            "the system is configured for Ultra-Stable accounts only"
        } else {
            "declared mode changed in the system configuration"
        };
        audit(
            &cfg,
            &[
                ("op", "apply-state"),
                ("target", name),
                ("result", "override-dropped"),
                ("reason", reason),
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
