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
//! resilience-library change --from-peer ADDRESS --get ID...
//!                                               install packs from a nearby
//!                                               machine that shares (ADR 0025)
//! resilience-library peer-list ADDRESS          catalog packs a nearby machine offers
//! resilience-library peer-files ADDRESS         files in its shared folder
//! resilience-library peer-get ADDRESS NAME      copy one of them into ~/Received
//! resilience-library open-remote ADDRESS index|library|pages
//!                                               read a nearby machine's pages
//! ```
//!
//! Nearby machines (ADR 0025): ADDRESS must be a local address on one of
//! this machine's networks (see `peer::resolve`). Packs from them come over
//! plain HTTP and go through the same store, which checks the catalog
//! hash, so a machine that sends other bytes wastes the download, nothing
//! more. Their pages are read through a loopback relay (`peer::Relay`).
//!
//! `change` downloads with `curl` (HTTPS only), or reads the files from a
//! mounted stick (`--from`), as the user, and streams the bytes to
//! `resilience-pack-store`, which runs as the `resilience-packs` account and
//! installs each pack only if its SHA-256 matches the catalog. An interrupted
//! download continues where it stopped next time.
//!
//! `open` starts `kiwix-serve` on 127.0.0.1 for a ZIM file, runs the text
//! browser on it, and stops the server when the browser exits. A map pack
//! (`.mbtiles`, ADR 0016) is never given to kiwix-serve: see [`open_map`].

use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use resilience_core::human_bytes;
use resilience_core::launch::{self, PolkitAgent};
use resilience_core::packs::{self, Outcome, Pack, PackConfig, PackKind, PackState, PlanLine};
use resilience_core::share::Peer;

mod peer;

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
  resilience-library check FILE
  resilience-library change --from-peer ADDRESS --get ID...
  resilience-library peer-list ADDRESS
  resilience-library peer-files ADDRESS
  resilience-library peer-get ADDRESS NAME
  resilience-library open-remote ADDRESS index|library|pages";

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
        Some("peer-list") if args.len() == 2 => config().and_then(|c| peer_list(&c, &args[1])),
        Some("peer-files") if args.len() == 2 => {
            config().and_then(|c| peer_files(&c, &args[1]).map(|_| ()))
        }
        Some("peer-get") if args.len() == 3 => {
            config().and_then(|c| peer_get(&c, &args[1], &args[2]))
        }
        Some("open-remote") if args.len() == 3 => {
            config().and_then(|c| open_remote(&c, &args[1], &args[2]))
        }
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
    /// A nearby machine that shares (ADR 0025), as given.
    peer: Option<String>,
    remove: Vec<String>,
    get: Vec<String>,
}

fn parse_change(args: &[String]) -> Result<ChangeArgs, Failure> {
    if !args.len().is_multiple_of(2) {
        return Err(Failure::Usage);
    }
    let mut c = ChangeArgs {
        from: None,
        peer: None,
        remove: Vec::new(),
        get: Vec::new(),
    };
    for pair in args.chunks(2) {
        match pair[0].as_str() {
            "--from" if c.from.is_none() && c.peer.is_none() => {
                c.from = Some(PathBuf::from(&pair[1]))
            }
            "--from-peer" if c.from.is_none() && c.peer.is_none() => c.peer = Some(pair[1].clone()),
            "--remove" => c.remove.push(pair[1].clone()),
            "--get" => c.get.push(pair[1].clone()),
            _ => return Err(Failure::Usage),
        }
    }
    if c.remove.is_empty() && c.get.is_empty() {
        return Err(Failure::Usage);
    }
    if c.peer.is_some() && (c.get.is_empty() || !c.remove.is_empty()) {
        return Err(Failure::Usage);
    }
    Ok(c)
}

/// Where the bytes of one pack come from.
enum Source {
    File(PathBuf),
    Download,
    Peer(Peer),
}

fn change(cfg: &PackConfig, args: &[String]) -> Result<(), Failure> {
    let req = parse_change(args)?;
    let from_peer = match &req.peer {
        Some(a) => Some(peer::resolve(a).map_err(Failure::Refused)?),
        None => None,
    };
    let mut sources = Vec::new();
    for id in &req.get {
        let p = cfg
            .pack(id)
            .ok_or_else(|| Failure::Refused(format!("{id:?} is not in the catalog.")))?;
        let source = match &req.from {
            _ if from_peer.is_some() => Source::Peer(from_peer.clone().ok_or(Failure::Usage)?),
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
            Source::Peer(_) => "get     ",
        };
        println!("  {how} {}  ({})", p.title, human_bytes(p.size));
    }
    if let Some(p) = &from_peer {
        println!(
            "\nFrom the computer at {}. Each pack is checked against this system's \
             catalog before it is installed.",
            p.display()
        );
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
                    Source::Download => send_download(cfg, &mut to_store, p, *offset, None),
                    Source::Peer(peer) => send_download(cfg, &mut to_store, p, *offset, Some(peer)),
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

fn curl(
    cfg: &PackConfig,
    url: &str,
    from: u64,
    peer: Option<&Peer>,
) -> io::Result<(Child, ChildStdout)> {
    let mut cmd = Command::new(&cfg.curl);
    if let Some(p) = peer {
        // `url` is the pack's path on the peer.
        cmd.args(peer::curl_args(p, url, from));
        return spawn_curl(cmd);
    }
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
    spawn_curl(cmd)
}

fn spawn_curl(mut cmd: Command) -> io::Result<(Child, ChildStdout)> {
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
    peer: Option<&Peer>,
) -> Result<(), String> {
    let urls = match peer {
        Some(_) => vec![format!("/packs/{}", p.file)],
        None => p.urls.clone(),
    };
    let mut have = offset;
    let mut progress = Progress::new(p.size, offset);
    let mut last_error = String::new();
    for attempt in 0..ATTEMPTS {
        let url = &urls[attempt % urls.len()];
        if attempt > 0 {
            eprintln!("\n  trying again in 5 seconds...");
            std::thread::sleep(Duration::from_secs(5));
        }
        let (mut child, mut out) = curl(cfg, url, have, peer).map_err(|e| format!("curl: {e}"))?;
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
        .ok_or_else(|| {
            Failure::Refused(format!(
                "{} is not available.",
                resilience_core::printable(&path.to_string_lossy())
            ))
        })?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let pack = cfg.packs.iter().find(|p| p.file == name);
    println!(
        "Checking {}",
        resilience_core::printable(&path.to_string_lossy())
    );
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
            resilience_core::printable(&file.to_string_lossy())
        )));
    }
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| packs::valid_file_name(n))
        .ok_or_else(|| {
            Failure::Refused(format!(
                "{} is not a pack file.",
                resilience_core::printable(&file.to_string_lossy())
            ))
        })?;
    if !fs::metadata(&file).is_ok_and(|m| m.is_file()) {
        return Err(Failure::Refused(format!(
            "{} is not available.",
            file.display()
        )));
    }
    if PackKind::of(name) == Some(PackKind::Map) {
        return open_map(&file, search);
    }
    let book = name.strip_suffix(".zim").unwrap_or(name);
    let size = fs::metadata(&file).map_or(0, |m| m.len());
    let pack = cfg.packs.iter().find(|p| p.file == name && p.size == size);
    // Installed files (and older versions) were checked when installed.
    let builtin = cfg.builtin.iter().any(|b| b.path == file) || file.starts_with(&cfg.dir);
    if pack.is_none() && !builtin && io::stdin().is_terminal() {
        println!(
            "{}\n\nThis file is not in the catalog of this system, so it was not checked. \
             Open it only if you trust where it comes from.\n",
            resilience_core::printable(&file.to_string_lossy())
        );
        print!("Press Enter to open it, or Ctrl-C to cancel...");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
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
    // `localhost` may be [::1]: hold that port too, so that no other
    // account can answer there instead of our server.
    let _guard = match peer::Relay::guard_v6(port) {
        Ok(g) => g,
        Err(e) => {
            let _ = server.kill();
            let _ = server.wait();
            return Err(Failure::Error(format!(
                "Another program holds the local port {port} ({e}); try again."
            )));
        }
    };
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

// ------------------------------------------------------ nearby machines

/// The catalog packs a nearby machine offers (ADR 0025): one line per
/// pack, `ID<TAB>FILE<TAB>SIZE<TAB>TITLE`, from its index page.
fn peer_list(cfg: &PackConfig, address: &str) -> Result<(), Failure> {
    let p = peer::resolve(address).map_err(Failure::Refused)?;
    let mut argv = vec![cfg.curl.clone()];
    argv.extend(peer::curl_args(&p, "/", 0));
    // Its index page is small; anything longer is cut.
    argv.splice(1..1, ["--max-filesize".to_string(), "262144".to_string()]);
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let out = resilience_core::proc::run_full(&args, Duration::from_secs(20))
        .ok_or_else(|| Failure::Error("The computer did not answer in time.".into()))?;
    if !out.success() {
        return Err(Failure::Error(format!(
            "The computer at {} did not answer: {}",
            p.display(),
            resilience_core::printable(out.stderr.trim())
        )));
    }
    let files: Vec<&str> = cfg.packs.iter().map(|q| q.file.as_str()).collect();
    for f in resilience_core::share::packs_in_index(&out.stdout, &files) {
        if let Some(q) = cfg.packs.iter().find(|q| q.file == f) {
            println!("{}\t{}\t{}\t{}", q.id, q.file, q.size, q.title);
        }
    }
    Ok(())
}

/// Fetch a small page from a peer into memory (at most `max` bytes).
fn peer_page(cfg: &PackConfig, p: &Peer, path: &str, max: u64) -> Result<String, Failure> {
    let mut argv = vec![cfg.curl.clone()];
    argv.extend(peer::curl_args(p, path, 0));
    argv.splice(1..1, ["--max-filesize".to_string(), max.to_string()]);
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let out = resilience_core::proc::run_full(&args, Duration::from_secs(20))
        .ok_or_else(|| Failure::Error("The computer did not answer in time.".into()))?;
    if !out.success() {
        return Err(Failure::Error(format!(
            "The computer at {} did not answer: {}",
            p.display(),
            resilience_core::printable(out.stderr.trim())
        )));
    }
    Ok(out.stdout)
}

/// The files of a nearby machine's shared folder: `NAME<TAB>SIZE` lines.
fn peer_files(
    cfg: &PackConfig,
    address: &str,
) -> Result<Vec<resilience_core::share::SharedFile>, Failure> {
    let p = peer::resolve(address).map_err(Failure::Refused)?;
    let page = peer_page(cfg, &p, "/files/", 1 << 20)?;
    let files = resilience_core::share::parse_listing(&page);
    for f in &files {
        println!("{}\t{}", f.name, f.size);
    }
    Ok(files)
}

/// Copy one file of a nearby machine's shared folder into ~/Received
/// (ADR 0025 §5, with the rules of ADR 0020): the size its listing gives
/// and not one byte more, a hidden `.partial` file (0600) until complete,
/// then moved in under a safe name that never replaces a file.
fn peer_get(cfg: &PackConfig, address: &str, name: &str) -> Result<(), Failure> {
    use resilience_core::share::{self, MAX_FOLDER_FILE};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if !share::valid_shared_name(name) {
        return Err(Failure::Refused("Not a file name.".into()));
    }
    let p = peer::resolve(address).map_err(Failure::Refused)?;
    let listing = share::parse_listing(&peer_page(cfg, &p, "/files/", 1 << 20)?);
    let f = listing.iter().find(|f| f.name == name).ok_or_else(|| {
        Failure::Refused(format!(
            "{} is not in that computer's shared folder.",
            resilience_core::printable(name)
        ))
    })?;
    if f.size > MAX_FOLDER_FILE {
        return Err(Failure::Refused(format!(
            "{} is too large ({}; at most {}).",
            resilience_core::printable(name),
            human_bytes(f.size),
            human_bytes(MAX_FOLDER_FILE)
        )));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Failure::Error("No home folder.".into()))?;
    let dest = home.join(resilience_core::messages::RECEIVED_DIR);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dest)
        .map_err(|e| Failure::Error(format!("{}: {e}", dest.display())))?;
    if free_bytes(&dest).is_some_and(|free| free < f.size + 64 * 1024 * 1024) {
        return Err(Failure::Refused(format!(
            "Not enough free space for {} ({}).",
            resilience_core::printable(name),
            human_bytes(f.size)
        )));
    }
    // SAFETY: getpid cannot fail.
    let pid = unsafe { libc::getpid() };
    let partial = dest.join(format!(".download-{pid}.partial"));
    let _ = fs::remove_file(&partial);
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&partial)
        .map_err(|e| Failure::Error(format!("{}: {e}", partial.display())))?;
    println!(
        "{} ({}) from the computer at {}",
        resilience_core::printable(name),
        human_bytes(f.size),
        p.display()
    );
    let path = format!("/files/{}", share::percent_encode(name));
    let mut cmd = Command::new(&cfg.curl);
    cmd.args(peer::curl_args(&p, &path, 0));
    let result = (|| -> Result<(), String> {
        let (mut child, mut body) = spawn_curl(cmd).map_err(|e| format!("curl: {e}"))?;
        let mut buf = vec![0u8; CHUNK];
        let mut got: u64 = 0;
        let mut progress = Progress::new(f.size, 0);
        loop {
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.to_string());
                }
            };
            got += n as u64;
            if got > f.size {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the computer sent more than the file's size".into());
            }
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            progress.add(n as u64);
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("download failed ({status})"));
        }
        if got != f.size {
            return Err("the download stopped before the end".into());
        }
        out.sync_all().map_err(|e| e.to_string())
    })();
    eprintln!();
    if let Err(e) = result {
        let _ = fs::remove_file(&partial);
        return Err(Failure::Error(format!(
            "{}: {e}. Nothing was kept.",
            resilience_core::printable(name)
        )));
    }
    match resilience_core::messages::move_into(&partial, &dest, name) {
        Ok(to) => {
            println!(
                "Saved as {}.\nIt was not checked: open it only with care.",
                to.display()
            );
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&partial);
            Err(Failure::Error(format!("Cannot save it: {e}")))
        }
    }
}

/// Free bytes for an unprivileged user on the file system of `p`.
fn free_bytes(p: &Path) -> Option<u64> {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: zeroed statvfs is a valid out-parameter; `c` is NUL-terminated.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointers for the duration of the call.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Read a nearby machine's index, library or pages in the text browser,
/// through a relay on 127.0.0.1 (lynx keeps `-localhost`).
fn open_remote(cfg: &PackConfig, address: &str, what: &str) -> Result<(), Failure> {
    let path = match what {
        "index" => "/",
        // kiwix-serve's list of books without JavaScript.
        "library" => "/library/nojs",
        "pages" => "/pages/",
        _ => return Err(Failure::Usage),
    };
    let p = peer::resolve(address).map_err(Failure::Refused)?;
    let target = peer::socket_addr(&p).map_err(Failure::Refused)?;
    let relay = peer::Relay::start(target)
        .map_err(|e| Failure::Error(format!("Cannot open a local port: {e}")))?;
    if io::stdin().is_terminal() {
        println!(
            "Pages from the computer at {}.\n\nThey were not checked: they are as trustworthy as \
             whoever runs that computer. Anyone on this network can read them too.\n",
            p.display()
        );
        print!("Press Enter to open them...");
        let _ = io::stdout().flush();
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    let mut argv = cfg.browser.clone();
    argv.push(format!("http://localhost:{}{path}", relay.port));
    let status = launch::run_foreground(&argv, &[], &[]);
    drop(relay);
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(Failure::Error(format!("The text browser exited with {s}."))),
        Err(e) => Err(Failure::Error(format!(
            "Cannot start the text browser: {e}"
        ))),
    }
}

/// The message for a map pack, which the text browser cannot show.
const MAP_REFUSAL: &str = "This file is a map. Maps are not read in the text browser: open it \
     from \"Maps & navigation\" in the main menu.";

/// A map pack (`.mbtiles`). The map viewer comes with a later part of ADR
/// 0016; until then this refuses. This is the one place that will start it.
/// `file` is absolute, has a valid pack name and is a regular file.
fn open_map(file: &Path, search: Option<&str>) -> Result<(), Failure> {
    let _ = (file, search);
    Err(Failure::Refused(MAP_REFUSAL.into()))
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
        let c = parse_change(&s(&["--from-peer", "192.168.1.5", "--get", "a"]))
            .ok()
            .expect("peer");
        assert_eq!(c.peer.as_deref(), Some("192.168.1.5"));
        // From a peer: only getting, and not with a stick at the same time.
        assert!(parse_change(&s(&["--from-peer", "x", "--remove", "a.zim"])).is_err());
        assert!(
            parse_change(&s(&["--from-peer", "x", "--get", "a", "--remove", "b.zim"])).is_err()
        );
        assert!(parse_change(&s(&["--from", "/m", "--from-peer", "x", "--get", "a"])).is_err());
        assert!(parse_change(&s(&["--from-peer", "x", "--from-peer", "y", "--get", "a"])).is_err());
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
    fn map_packs_are_never_given_to_the_kiwix_server() {
        let dir = std::env::temp_dir().join(format!("res-lib-map-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        let map = dir.join("map-osm-luxembourg_2026-01.mbtiles");
        fs::write(&map, b"SQLite format 3\0").expect("w");
        // kiwix-serve and the browser do not exist: reaching them would fail
        // with an error, not this refusal.
        let cfg = PackConfig {
            dir: dir.join("packs"),
            store_user: "resilience-packs".into(),
            store: "/nonexistent/store".into(),
            pkexec: "/nonexistent/pkexec".into(),
            pkttyagent: "/nonexistent/pkttyagent".into(),
            curl: "/nonexistent/curl".into(),
            sha256sum: "/nonexistent/sha256sum".into(),
            kiwix_serve: "/nonexistent/kiwix-serve".into(),
            browser: vec!["/nonexistent/lynx".into()],
            session_shell: "/bin/sh".into(),
            packs: Vec::new(),
            sets: Vec::new(),
            builtin: Vec::new(),
        };
        let path = map.to_string_lossy().into_owned();
        for args in [
            vec![path.clone()],
            vec![path, "--search".into(), "x".into()],
        ] {
            match open(&cfg, &args) {
                Err(Failure::Refused(m)) => {
                    assert_eq!(m, MAP_REFUSAL);
                    assert!(m.contains("Maps & navigation"), "{m}");
                }
                _ => panic!("a map must be refused"),
            }
        }
        // Not a pack at all: still refused by name, before anything runs.
        let other = dir.join("x.mbtiles-wal");
        fs::write(&other, b"x").expect("w");
        assert!(matches!(
            open(&cfg, &[other.to_string_lossy().into_owned()]),
            Err(Failure::Refused(m)) if m.contains("not a pack file")
        ));
        let _ = fs::remove_dir_all(&dir);
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
