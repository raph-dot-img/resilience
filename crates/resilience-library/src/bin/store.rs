//! `resilience-pack-store` — writes the shared packs directory (ADR 0013).
//!
//! Started by `resilience-library` through
//! `pkexec --user resilience-packs resilience-pack-store change …` after an
//! administrator's password. It runs as the unprivileged system account
//! `resilience-packs`, never as root, and refuses to run as anyone else.
//!
//! ```text
//! resilience-pack-store change [--remove FILE]... [--get ID]...
//! ```
//!
//! Removals come first. Then, for the packs to get, it writes one plan line
//! per pack on stdout (`send ID OFFSET` or `skip ID REASON`, then `ready`),
//! reads each pack's bytes from stdin in chunks, and answers `done ID …`.
//! It trusts only `/etc/resilience/packs.toml` (root-owned): the environment
//! and the caller's files are ignored. The bytes are never interpreted, only
//! hashed; a pack is installed only if its SHA-256 matches the catalog.

use std::ffi::CString;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::ExitCode;

use resilience_core::packs::{self, Outcome, PackConfig, PlanLine};

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;
const MAX_ITEMS: usize = 64;

const USAGE: &str = "Usage: resilience-pack-store change [--remove FILE]... [--get ID]...";

fn main() -> ExitCode {
    // SAFETY: umask, signal and getrlimit/setrlimit have no preconditions.
    // Installed packs are readable by all. Limits and ignored signals pass
    // through pkexec from the caller: undo those that could stop a write
    // half-way.
    unsafe {
        libc::umask(0o022);
        libc::signal(libc::SIGXFSZ, libc::SIG_DFL);
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_FSIZE, &mut lim) == 0 {
            lim.rlim_cur = lim.rlim_max;
            libc::setrlimit(libc::RLIMIT_FSIZE, &lim);
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(EXIT_ERROR),
        Err((code, msg)) => {
            eprintln!("resilience-pack-store: {msg}");
            if code != EXIT_USAGE {
                log(&format!("op=change result=refused reason={msg:?}"));
            }
            ExitCode::from(code)
        }
    }
}

struct Request {
    remove: Vec<String>,
    get: Vec<String>,
}

fn parse(args: &[String]) -> Result<Request, String> {
    let Some((op, rest)) = args.split_first() else {
        return Err(USAGE.into());
    };
    if op != "change" || rest.len() % 2 != 0 || rest.len() / 2 > MAX_ITEMS {
        return Err(USAGE.into());
    }
    let mut req = Request {
        remove: Vec::new(),
        get: Vec::new(),
    };
    for pair in rest.chunks(2) {
        match pair[0].as_str() {
            "--remove" if packs::valid_file_name(&pair[1]) => req.remove.push(pair[1].clone()),
            "--get" if packs::valid_id(&pair[1]) => req.get.push(pair[1].clone()),
            _ => return Err(format!("invalid argument {:?} {:?}", pair[0], pair[1])),
        }
    }
    if req.remove.is_empty() && req.get.is_empty() {
        return Err(USAGE.into());
    }
    Ok(req)
}

fn run(args: &[String]) -> Result<bool, (u8, String)> {
    let req = parse(args).map_err(|m| (EXIT_USAGE, m))?;
    let cfg = PackConfig::load(Path::new(packs::PACKS_CONFIG_PATH))
        .map_err(|e| (EXIT_ERROR, e.to_string()))?;
    check_identity(&cfg).map_err(|m| (EXIT_REFUSED, m))?;
    let caller = caller();
    let _lock = lock(&cfg).map_err(|m| (EXIT_REFUSED, m))?;

    let mut ok = true;
    for file in &req.remove {
        match packs::remove_file(&cfg, file) {
            Ok(removed) => {
                log(&format!(
                    "op=remove caller={caller} file={file} result={}",
                    if removed { "ok" } else { "absent" }
                ));
                eprintln!(
                    "{file}: {}",
                    if removed {
                        "removed."
                    } else {
                        "was not installed."
                    }
                );
            }
            Err(m) => {
                ok = false;
                log(&format!(
                    "op=remove caller={caller} file={file} result=error reason={m:?}"
                ));
                eprintln!("{file}: cannot remove: {m}");
            }
        }
    }
    if req.get.is_empty() {
        return Ok(ok);
    }

    let plan = packs::plan_receive(&cfg, &req.get, || packs::free_space(&cfg.dir))
        .map_err(|m| (EXIT_REFUSED, m))?;
    let mut out = BufWriter::new(io::stdout().lock());
    for line in &plan {
        if let PlanLine::Skip { id, reason } = line {
            log(&format!(
                "op=get caller={caller} id={id} result=skipped reason={reason:?}"
            ));
        }
        let _ = out.write_all(line.render().as_bytes());
    }
    let _ = out.flush();
    let mut input = io::stdin().lock();
    for line in &plan {
        let PlanLine::Send { id, offset } = line else {
            continue;
        };
        let Some(p) = cfg.pack(id) else { continue };
        log(&format!(
            "op=get caller={caller} id={id} file={} from={offset} result=started",
            p.file
        ));
        let (outcome, stop) = match packs::receive_pack(&cfg, p, *offset, &mut input) {
            Ok(o) => (o, false),
            Err(e) => (Outcome::Failed(e.to_string()), true),
        };
        let result = match &outcome {
            Outcome::Installed => "ok".to_string(),
            Outcome::Incomplete(n) => format!("incomplete bytes={n}"),
            Outcome::Mismatch => "mismatch".into(),
            Outcome::Failed(m) => format!("error reason={m:?}"),
        };
        log(&format!(
            "op=get caller={caller} id={id} file={} result={result}",
            p.file
        ));
        ok &= outcome == Outcome::Installed;
        let _ = out.write_all(outcome.render(id).as_bytes());
        let _ = out.flush();
        if stop {
            // The input is no longer in step with the plan.
            break;
        }
    }
    Ok(ok)
}

/// Runs only as the store account, which must own the packs directory.
fn check_identity(cfg: &PackConfig) -> Result<(), String> {
    let name = CString::new(cfg.store_user.as_str()).map_err(|_| "bad store user".to_string())?;
    // SAFETY: getpwnam with a valid C string; the result is read at once.
    let pw = unsafe { libc::getpwnam(name.as_ptr()) };
    if pw.is_null() {
        return Err(format!("the account {} does not exist", cfg.store_user));
    }
    // SAFETY: pw is non-null and points to a valid passwd entry.
    let uid = unsafe { (*pw).pw_uid };
    // SAFETY: getuid/geteuid have no preconditions.
    let (real, effective) = unsafe { (libc::getuid(), libc::geteuid()) };
    if uid == 0 || real != uid || effective != uid {
        return Err(format!(
            "must run as {} (pkexec --user {}), not uid {effective}",
            cfg.store_user, cfg.store_user
        ));
    }
    for d in [cfg.dir.clone(), cfg.staging()] {
        let m = fs::symlink_metadata(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        if !m.file_type().is_dir() || m.uid() != uid {
            return Err(format!(
                "{} is not a directory owned by {}",
                d.display(),
                cfg.store_user
            ));
        }
    }
    Ok(())
}

fn lock(cfg: &PackConfig) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = cfg.staging().join("lock");
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: flock on a file descriptor we own; non-blocking.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("another installation of packs is running; try again when it ends".into());
    }
    Ok(f)
}

/// The account that asked (pkexec sets PKEXEC_UID; its environment is
/// otherwise cleared).
fn caller() -> String {
    std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map(|u| format!("uid{u}"))
        .unwrap_or_else(|| "unknown".into())
}

fn log(msg: &str) {
    let Ok(msg) = CString::new(msg.replace('\0', "")) else {
        return;
    };
    // SAFETY: openlog/syslog/closelog with valid C strings; "%s" keeps the
    // message from being read as a format string.
    unsafe {
        libc::openlog(
            c"resilience-pack-store".as_ptr(),
            libc::LOG_PID,
            libc::LOG_AUTHPRIV,
        );
        libc::syslog(libc::LOG_NOTICE, c"%s".as_ptr(), msg.as_ptr());
        libc::closelog();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn arguments_are_strict() {
        let r = parse(&s(&["change", "--remove", "a.zim", "--get", "wiki-en"])).expect("ok");
        assert_eq!(r.remove, vec!["a.zim"]);
        assert_eq!(r.get, vec!["wiki-en"]);
        for bad in [
            &["change"][..],
            &["remove", "a.zim"],
            &["change", "--get"],
            &["change", "--get", "../x"],
            &["change", "--remove", "/etc/passwd"],
            &["change", "--remove", "x.txt"],
            &["change", "--other", "x"],
        ] {
            assert!(parse(&s(bad)).is_err(), "{bad:?}");
        }
        // Map packs (ADR 0016): the same rules, a second extension.
        let r = parse(&s(&[
            "change",
            "--remove",
            "map-osm-luxembourg_2025-07.mbtiles",
            "--get",
            "map-osm-luxembourg",
        ]))
        .expect("map names");
        assert_eq!(r.remove, vec!["map-osm-luxembourg_2025-07.mbtiles"]);
        assert_eq!(r.get, vec!["map-osm-luxembourg"]);
        for bad in [
            ".mbtiles",
            "../x.mbtiles",
            "/var/lib/resilience/packs/x.mbtiles",
            "x.mbtiles.part",
            "x.mbtiles-wal",
            "x.mbtiles-journal",
            "x..mbtiles",
            "x y.mbtiles",
            "lock",
        ] {
            assert!(parse(&s(&["change", "--remove", bad])).is_err(), "{bad}");
        }
        let many: Vec<String> = std::iter::once("change".to_string())
            .chain((0..65).flat_map(|i| ["--get".to_string(), format!("p{i}")]))
            .collect();
        assert!(parse(&many).is_err());
    }
}
