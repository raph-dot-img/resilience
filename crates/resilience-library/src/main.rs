//! `resilience-library` — the offline library, as the user (ADR 0013).
//!
//! ```text
//! resilience-library list                       catalog and what is installed
//! resilience-library change [--from DIR] [--remove FILE]... [--get ID]...
//!                                               install or remove packs (one
//!                                               administrator password)
//! resilience-library open FILE [--search WORDS] read a pack in the text browser
//! resilience-library check FILE                 compare a file (e.g. on a stick)
//!                                               with the catalog's fingerprint
//! ```
//!
//! `change` downloads with `curl` (HTTPS only), or reads the files from a
//! mounted stick (`--from`), as the user, and streams the bytes to
//! `resilience-pack-store`, which runs as the `resilience-packs` account and
//! installs each pack only if its SHA-256 matches the catalog. An interrupted
//! download continues where it stopped next time.
//!
//! `open` starts `kiwix-serve` on 127.0.0.1 for this one file, runs the text
//! browser on it, and stops the server when the browser exits.

use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use resilience_core::human_bytes;
use resilience_core::launch::{self, PolkitAgent};
use resilience_core::packs::{self, Outcome, Pack, PackConfig, PackState, PlanLine};

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;
/// Bytes per chunk sent to the store.
const CHUNK: usize = 256 * 1024;
/// Download attempts per pack (each continues where the previous stopped).
const ATTEMPTS: usize = 4;

const USAGE: &str = "\
Usage:
  resilience-library list
  resilience-library change [--from DIR] [--remove FILE]... [--get ID]...
  resilience-library open FILE [--search WORDS]
  resilience-library check FILE";

enum Failure {
    Usage,
    Refused(String),
    Error(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") if args.len() == 1 => config().and_then(|c| list(&c)),
        Some("change") => config().and_then(|c| change(&c, &args[1..])),
        Some("open") => config().and_then(|c| open(&c, &args[1..])),
        Some("check") if args.len() == 2 => config().and_then(|c| check(&c, Path::new(&args[1]))),
        // Used at build time on the generated catalog.
        Some("check-config") if args.len() == 2 => PackConfig::load(Path::new(&args[1]))
            .map(|c| println!("{} packs", c.packs.len()))
            .map_err(|e| Failure::Error(e.to_string())),
        Some("--version") => {
            println!("resilience-library {}", resilience_core::VERSION);
            Ok(())
        }
        Some("--help" | "-h") => {
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
        Err(Failure::Refused(m)) => {
            eprintln!("\n{m}");
            ExitCode::from(EXIT_REFUSED)
        }
        Err(Failure::Error(m)) => {
            eprintln!("\n{m}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

/// The catalog. `RESILIENCE_PACKS_CONFIG` overrides the path for development
/// (this program has no privileges; the store ignores it).
fn config() -> Result<PackConfig, Failure> {
    let path = std::env::var_os("RESILIENCE_PACKS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| packs::PACKS_CONFIG_PATH.into());
    PackConfig::load(&path).map_err(|e| Failure::Error(e.to_string()))
}

fn list(cfg: &PackConfig) -> Result<(), Failure> {
    let inv = packs::inventory(cfg);
    for (p, st) in cfg.packs.iter().zip(&inv.states) {
        let state = match st {
            PackState::Installed => "installed".to_string(),
            PackState::Partial(n) => format!("partial({n})"),
            PackState::Absent => "absent".into(),
        };
        println!("{}\t{}\t{}\t{state}\t{}", p.id, p.language, p.size, p.title);
    }
    for o in &inv.others {
        println!("-\t-\t{}\tother\t{}", o.size, o.file);
    }
    Ok(())
}

// ---------------------------------------------------------------- change

struct ChangeArgs {
    from: Option<PathBuf>,
    remove: Vec<String>,
    get: Vec<String>,
}

fn parse_change(args: &[String]) -> Result<ChangeArgs, Failure> {
    if args.len() % 2 != 0 {
        return Err(Failure::Usage);
    }
    let mut c = ChangeArgs {
        from: None,
        remove: Vec::new(),
        get: Vec::new(),
    };
    for pair in args.chunks(2) {
        match pair[0].as_str() {
            "--from" if c.from.is_none() => c.from = Some(PathBuf::from(&pair[1])),
            "--remove" => c.remove.push(pair[1].clone()),
            "--get" => c.get.push(pair[1].clone()),
            _ => return Err(Failure::Usage),
        }
    }
    if c.remove.is_empty() && c.get.is_empty() {
        return Err(Failure::Usage);
    }
    Ok(c)
}

/// Where the bytes of one pack come from.
enum Source {
    File(PathBuf),
    Download,
}

fn change(cfg: &PackConfig, args: &[String]) -> Result<(), Failure> {
    let req = parse_change(args)?;
    let mut sources = Vec::new();
    for id in &req.get {
        let p = cfg
            .pack(id)
            .ok_or_else(|| Failure::Refused(format!("{id:?} is not in the catalog.")))?;
        let source = match &req.from {
            Some(dir) => {
                let found = packs::find_on_volume(cfg, dir);
                let idx = cfg.packs.iter().position(|q| q.id == p.id);
                match found.into_iter().find(|(i, _)| Some(*i) == idx) {
                    Some((_, path)) => Source::File(path),
                    None => {
                        return Err(Failure::Refused(format!(
                            "{} ({}) is not on {}, or its size is wrong.",
                            p.title,
                            p.file,
                            dir.display()
                        )));
                    }
                }
            }
            None if p.urls.is_empty() => {
                return Err(Failure::Refused(format!(
                    "{} has no download address; copy it from a USB stick.",
                    p.title
                )));
            }
            None => Source::Download,
        };
        sources.push((p, source));
    }
    for f in &req.remove {
        if !packs::valid_file_name(f) {
            return Err(Failure::Refused(format!("{f:?} is not a pack file name.")));
        }
    }

    println!("Resource packs\n");
    for f in &req.remove {
        let title = cfg
            .packs
            .iter()
            .find(|p| &p.file == f)
            .map(|p| p.title.as_str())
            .unwrap_or(f.as_str());
        println!("  remove   {title}");
    }
    for (p, s) in &sources {
        let how = match s {
            Source::File(_) => "copy    ",
            Source::Download => "download",
        };
        println!("  {how} {}  ({})", p.title, human_bytes(p.size));
    }
    println!("\nAn administrator's password is needed (packs are shared by every account).");
    println!("Ctrl-C stops; an unfinished download continues next time.\n");

    let _agent = PolkitAgent::start(&cfg.pkttyagent)
        .map_err(|e| Failure::Error(format!("Cannot ask for the password: {e}")))?;
    let mut argv = vec![
        "--user".to_string(),
        cfg.store_user.clone(),
        cfg.store.clone(),
        "change".into(),
    ];
    for f in &req.remove {
        argv.extend(["--remove".to_string(), f.clone()]);
    }
    for (p, _) in &sources {
        argv.extend(["--get".to_string(), p.id.clone()]);
    }
    let mut child = Command::new(&cfg.pkexec)
        .args(&argv)
        .env("SHELL", &cfg.session_shell)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| Failure::Error(format!("Cannot start pkexec: {e}")))?;
    let mut to_store = child
        .stdin
        .take()
        .ok_or_else(|| Failure::Error("no pipe".into()))?;
    let mut from_store = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| Failure::Error("no pipe".into()))?,
    );

    // The plan: one line per pack, then `ready`.
    let mut plan = Vec::new();
    if !sources.is_empty() {
        loop {
            let mut line = String::new();
            match from_store.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => match PlanLine::parse(&line) {
                    Some(PlanLine::Ready) => {
                        plan.push(PlanLine::Ready);
                        break;
                    }
                    Some(l) => plan.push(l),
                    None => {}
                },
            }
        }
    }
    let mut all_ok = true;
    if !sources.is_empty() && plan.last() != Some(&PlanLine::Ready) {
        drop(to_store);
        let status = child.wait().ok();
        return Err(refusal_from(status));
    }
    for line in &plan {
        match line {
            PlanLine::Skip { id, reason } => {
                let title = cfg.pack(id).map(|p| p.title.as_str()).unwrap_or(id);
                if reason != "already installed" {
                    all_ok = false;
                }
                println!("{title}: {reason}.");
            }
            PlanLine::Send { id, offset } => {
                let Some((p, source)) = sources.iter().find(|(p, _)| &p.id == id) else {
                    continue;
                };
                if *offset >= p.size {
                    drop(to_store);
                    let _ = child.wait();
                    return Err(Failure::Error(format!(
                        "The installer asked for {} from an impossible offset.",
                        p.title
                    )));
                }
                println!("\n{}", p.title);
                if *offset > 0 {
                    println!("  continuing from {}", human_bytes(*offset));
                }
                let sent = match source {
                    Source::File(path) => send_file(&mut to_store, path, p, *offset),
                    Source::Download => send_download(cfg, &mut to_store, p, *offset),
                };
                if let Err(e) = &sent {
                    eprintln!("\n  {e}");
                }
                let ended = packs::write_end(&mut to_store).is_ok();
                let outcome = if ended {
                    read_outcome(&mut from_store, id)
                } else {
                    None
                };
                eprint!("\r{:72}\r", "");
                match outcome {
                    Some(o) => {
                        all_ok &= o == Outcome::Installed;
                        println!("  {}", o.describe(&p.title));
                    }
                    None => {
                        drop(to_store);
                        let _ = child.wait();
                        return Err(Failure::Error(
                            "The installer stopped. Nothing was lost: start again to continue."
                                .into(),
                        ));
                    }
                }
            }
            PlanLine::Ready => {}
        }
    }
    drop(to_store);
    let status = child
        .wait()
        .map_err(|e| Failure::Error(format!("pkexec: {e}")))?;
    if sources.is_empty() && !status.success() {
        return Err(refusal_from(Some(status)));
    }
    if all_ok && status.success() {
        println!("\nDone.");
        Ok(())
    } else {
        Err(Failure::Error(
            "Some packs were not installed (see above).".into(),
        ))
    }
}

fn refusal_from(status: Option<std::process::ExitStatus>) -> Failure {
    match status.and_then(|s| s.code()) {
        Some(126) | Some(127) => {
            Failure::Refused("Not authorised: the administrator's password was not given.".into())
        }
        Some(2) => Failure::Refused("The installer refused the request (see above).".into()),
        _ => Failure::Error("The installer failed (see above).".into()),
    }
}

fn read_outcome(r: &mut impl BufRead, id: &str) -> Option<Outcome> {
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if let Some((got, o)) = Outcome::parse(&line)
            && got == id
        {
            return Some(o);
        }
    }
}

struct Progress {
    total: u64,
    done: u64,
    started: Instant,
    start_bytes: u64,
    last: Instant,
}

impl Progress {
    fn new(total: u64, done: u64) -> Progress {
        let now = Instant::now();
        Progress {
            total,
            done,
            started: now,
            start_bytes: done,
            last: now - Duration::from_secs(1),
        }
    }

    fn add(&mut self, n: u64) {
        self.done += n;
        if self.last.elapsed() >= Duration::from_millis(500) || self.done == self.total {
            self.last = Instant::now();
            let secs = self.started.elapsed().as_secs_f64().max(0.1);
            let rate = ((self.done - self.start_bytes) as f64 / secs) as u64;
            let pct = self.done * 100 / self.total.max(1);
            eprint!(
                "\r  {} of {}  ({pct}%)  {}/s      ",
                human_bytes(self.done),
                human_bytes(self.total),
                human_bytes(rate)
            );
            let _ = io::stderr().flush();
        }
    }
}

/// Copy `need` bytes from `r` to the store. Returns the bytes sent.
fn pump(
    r: &mut impl Read,
    w: &mut impl Write,
    need: u64,
    progress: &mut Progress,
) -> io::Result<u64> {
    let mut buf = vec![0u8; CHUNK];
    let mut sent = 0;
    while sent < need {
        let want = (need - sent).min(CHUNK as u64) as usize;
        let n = match r.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        packs::write_chunk(w, &buf[..n])?;
        sent += n as u64;
        progress.add(n as u64);
    }
    Ok(sent)
}

fn send_file(w: &mut impl Write, path: &Path, p: &Pack, offset: u64) -> Result<(), String> {
    let mut f = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut progress = Progress::new(p.size, offset);
    let need = p.size - offset;
    let sent = pump(&mut f, w, need, &mut progress).map_err(|e| e.to_string())?;
    if sent < need {
        return Err(format!("{} ended early.", path.display()));
    }
    Ok(())
}

fn curl(cfg: &PackConfig, url: &str, from: u64) -> io::Result<(Child, ChildStdout)> {
    let mut cmd = Command::new(&cfg.curl);
    // -q first: ignore ~/.curlrc. HTTPS only, also after redirects.
    cmd.args([
        "-q",
        "--fail",
        "--location",
        "--max-redirs",
        "5",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--silent",
        "--show-error",
        "--connect-timeout",
        "30",
        "--speed-limit",
        "1024",
        "--speed-time",
        "120",
    ]);
    if from > 0 {
        cmd.args(["--continue-at", &from.to_string()]);
    }
    cmd.args(["--output", "-", "--", url]);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no pipe"))?;
    Ok((child, out))
}

fn send_download(
    cfg: &PackConfig,
    w: &mut impl Write,
    p: &Pack,
    offset: u64,
) -> Result<(), String> {
    let mut have = offset;
    let mut progress = Progress::new(p.size, offset);
    let mut last_error = String::new();
    for attempt in 0..ATTEMPTS {
        let url = &p.urls[attempt % p.urls.len()];
        if attempt > 0 {
            eprintln!("\n  trying again in 5 seconds...");
            std::thread::sleep(Duration::from_secs(5));
        }
        let (mut child, mut out) = curl(cfg, url, have).map_err(|e| format!("curl: {e}"))?;
        let need = p.size - have;
        let sent = pump(&mut out, w, need, &mut progress);
        drop(out);
        if matches!(&sent, Ok(n) if *n == need) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        let status = child.wait();
        match sent {
            Ok(n) => have += n,
            // The store is gone: nothing more can be sent.
            Err(e) => return Err(format!("cannot pass the data on: {e}")),
        }
        last_error = match status {
            Ok(s) if s.success() => "the server sent less data than expected".into(),
            Ok(s) => format!("download failed ({s})"),
            Err(e) => format!("curl: {e}"),
        };
    }
    Err(format!(
        "Download stopped: {last_error}. Check the network, then start again: it continues \
         where it stopped."
    ))
}

// ----------------------------------------------------------------- check

/// SHA-256 of a file through `sha256sum`, with progress on stderr.
fn hash_file(cfg: &PackConfig, path: &Path, size: u64) -> Result<String, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut child = Command::new(&cfg.sha256sum)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("sha256sum: {e}"))?;
    let mut input = child.stdin.take().ok_or("no pipe")?;
    let mut progress = Progress::new(size, 0);
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        input
            .write_all(&buf[..n])
            .map_err(|e| format!("sha256sum: {e}"))?;
        progress.add(n as u64);
    }
    drop(input);
    let out = child
        .wait_with_output()
        .map_err(|e| format!("sha256sum: {e}"))?;
    let hex = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    eprint!("\r{:72}\r", "");
    if out.status.success() && packs::is_sha256(&hex) {
        Ok(hex)
    } else {
        Err("sha256sum failed".into())
    }
}

fn check(cfg: &PackConfig, path: &Path) -> Result<(), Failure> {
    let meta = fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .ok_or_else(|| Failure::Refused(format!("{} is not available.", path.display())))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let pack = cfg.packs.iter().find(|p| p.file == name);
    println!("Checking {}", path.display());
    if let Some(p) = pack {
        println!("against: {} ({})\n", p.title, human_bytes(p.size));
        if meta.len() != p.size {
            return Err(Failure::Error(format!(
                "DOES NOT MATCH: the file is {} but the catalog expects {}. It is \
                 incomplete or a different file.",
                human_bytes(meta.len()),
                human_bytes(p.size)
            )));
        }
    }
    let hex = hash_file(cfg, path, meta.len()).map_err(Failure::Error)?;
    match pack {
        Some(p) if hex == p.sha256 => {
            println!(
                "MATCHES: this is exactly the file of the catalog. It can be read or installed."
            );
            Ok(())
        }
        Some(_) => Err(Failure::Error(
            "DOES NOT MATCH: the file is damaged or was altered. Do not rely on it; copy it \
             again from another source."
                .into(),
        )),
        None => Err(Failure::Refused(format!(
            "This file is not in the catalog of this system, so it cannot be checked.\n\
             Its SHA-256 is {hex}.\nCompare it with the fingerprint published by its source."
        ))),
    }
}

// ------------------------------------------------------------------ open

/// Uid owning the IPv4 socket listening on 127.0.0.1:`port`, from
/// /proc/net/tcp. Another account could take a port we just released.
fn listener_uid(tcp: &str, port: u16) -> Option<u32> {
    let want = format!("0100007F:{port:04X}");
    tcp.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f.len() > 7 && f[1] == want && f[3] == "0A")
            .then(|| f[7].parse().ok())
            .flatten()
    })
}

fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Start the server for one file and wait until it accepts connections.
fn start_server(cfg: &PackConfig, file: &Path) -> Result<(Child, u16), String> {
    // SAFETY: getpid cannot fail.
    let pid = unsafe { libc::getpid() };
    for _ in 0..3 {
        let port = free_port().map_err(|e| format!("no free local port: {e}"))?;
        let mut child = Command::new(&cfg.kiwix_serve)
            .arg("--address=127.0.0.1")
            .arg(format!("--port={port}"))
            .arg("--blockexternal")
            .arg(format!("--attachToProcess={pid}"))
            // Absolute (checked in `open`), so never read as an option.
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start kiwix-serve: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            if TcpStream::connect_timeout(
                &([127, 0, 0, 1], port).into(),
                Duration::from_millis(200),
            )
            .is_ok()
            {
                // Ours only if the listener belongs to this account and the
                // server is still running.
                // SAFETY: getuid has no preconditions.
                let me = unsafe { libc::getuid() };
                let tcp = fs::read_to_string("/proc/net/tcp").unwrap_or_default();
                if listener_uid(&tcp, port) == Some(me) && matches!(child.try_wait(), Ok(None)) {
                    return Ok((child, port));
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    Err("the pack could not be opened (is the file damaged?)".into())
}

fn open(cfg: &PackConfig, args: &[String]) -> Result<(), Failure> {
    let (file, search) = match args {
        [f] => (PathBuf::from(f), None),
        [f, flag, q] if flag == "--search" => (PathBuf::from(f), Some(q.as_str())),
        _ => return Err(Failure::Usage),
    };
    if !file.is_absolute() {
        return Err(Failure::Refused(format!(
            "{} must be an absolute path.",
            file.display()
        )));
    }
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| packs::valid_file_name(n))
        .ok_or_else(|| Failure::Refused(format!("{} is not a pack file.", file.display())))?;
    if !fs::metadata(&file).is_ok_and(|m| m.is_file()) {
        return Err(Failure::Refused(format!(
            "{} is not available.",
            file.display()
        )));
    }
    let book = name.strip_suffix(".zim").unwrap_or(name);
    let pack = cfg.packs.iter().find(|p| p.file == name);
    if let Some(notice) = pack.and_then(|p| p.notice.as_deref())
        && io::stdin().is_terminal()
    {
        println!(
            "{}\n\n{}\n",
            pack.map(|p| p.title.as_str()).unwrap_or(book),
            notice.trim()
        );
        print!("Press Enter to open it...");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    let (mut server, port) = start_server(cfg, &file).map_err(Failure::Error)?;
    let mut argv = cfg.browser.clone();
    argv.push(packs::reader_url(port, book, search));
    let status = launch::run_foreground(&argv, &[], &[]);
    let _ = server.kill();
    let _ = server.wait();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(Failure::Error(format!("The text browser exited with {s}."))),
        Err(e) => Err(Failure::Error(format!(
            "Cannot start the text browser: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn change_arguments() {
        let c = parse_change(&s(&[
            "--from",
            "/run/media/u/S",
            "--get",
            "a",
            "--remove",
            "b.zim",
        ]))
        .ok()
        .expect("ok");
        assert_eq!(c.from, Some(PathBuf::from("/run/media/u/S")));
        assert_eq!((c.get, c.remove), (s(&["a"]), s(&["b.zim"])));
        assert!(parse_change(&s(&[])).is_err());
        assert!(parse_change(&s(&["--from", "x"])).is_err());
        assert!(parse_change(&s(&["--from", "x", "--from", "y", "--get", "a"])).is_err());
        assert!(parse_change(&s(&["--get"])).is_err());
    }

    #[test]
    fn finds_the_owner_of_a_loopback_listener() {
        let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   \
            0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 1 1\n   \
            1: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 2 1\n   \
            2: 0100007F:1F91 0100007F:9C40 01 00000000:00000000 00:00000000 00000000  1001        0 3 1\n";
        assert_eq!(listener_uid(tcp, 8080), Some(1000));
        assert_eq!(listener_uid(tcp, 80), None, "not on the loopback address");
        assert_eq!(listener_uid(tcp, 8081), None, "not listening");
        // SAFETY: getuid has no preconditions.
        let me = unsafe { libc::getuid() };
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().expect("addr").port();
        let real = fs::read_to_string("/proc/net/tcp").expect("proc");
        assert_eq!(listener_uid(&real, port), Some(me));
    }

    #[test]
    fn pump_stops_at_the_expected_size() {
        let data = vec![7u8; 1000];
        let mut out = Vec::new();
        let mut p = Progress::new(600, 0);
        let n = pump(&mut &data[..], &mut out, 600, &mut p).expect("pump");
        assert_eq!(n, 600);
        packs::write_end(&mut out).expect("end");
        let mut r = &out[..];
        let mut buf = Vec::new();
        let mut total = 0;
        while let Some(k) = packs::read_chunk(&mut r, &mut buf).expect("chunk") {
            total += k;
        }
        assert_eq!(total, 600);
    }
}
