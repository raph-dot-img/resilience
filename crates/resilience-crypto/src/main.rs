//! `resilience-crypto` — guided encryption, signatures, checksums and keys
//! (ADR 0015).
//!
//! Runs as the user, never with privileges. The interface starts it in the
//! foreground for one operation; it can also be used from a shell.
//!
//! No cryptography is implemented here: age encrypts and decrypts, minisign
//! signs and verifies, coreutils compute checksums. Passphrases and
//! passwords are typed into those programs directly (they read the
//! terminal), so they never pass through this program, its arguments, a
//! file or a log. Outputs never replace an existing file.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use resilience_core::backup::{self, Stage, run_pipeline};
use resilience_core::config::{SystemConfig, ToolPaths};
use resilience_core::crypto::{
    self, Algo, Contact, Format, KeyStore, MAX_KEY_FILE, MAX_MESSAGE, create_new, detect,
    find_public_keys, free_path, minisign_key_id, rename_noreplace,
};
use resilience_core::{human_bytes, printable};

mod pgp;

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;

const USAGE: &str = "\
Usage (TO is --passphrase, or one or more of --me and --to CONTACT, or
one or more --to-pgp FINGERPRINT for OpenPGP keys):
  resilience-crypto encrypt TO FILE       Encrypt FILE into FILE.age next to it
  resilience-crypto decrypt FILE          Decrypt into ~/Decrypted (never overwrites)
  resilience-crypto write-message TO      Type a message; save it encrypted in ~/Messages
  resilience-crypto read-message FILE     Show an encrypted message (not saved)
  resilience-crypto sign FILE             Sign FILE with your key: FILE.minisig
  resilience-crypto verify FILE           Check FILE's signature (FILE.minisig)
  resilience-crypto checksum FILE         Show FILE's SHA-256 and SHA-512
  resilience-crypto compare SUM FILE      Compare FILE with an expected checksum
  resilience-crypto check-list FILE       Check the files listed in a checksum list
  resilience-crypto inspect FILE          Tell what kind of file this is
  resilience-crypto new-keys              Create your encryption and signing keys
  resilience-crypto add-contact [FILE]    Add someone's public keys (typed or a file)
  resilience-crypto export-keys DIR       Save your public keys into DIR
  resilience-crypto new-openpgp-key       Create your OpenPGP (GnuPG) key

OpenPGP files (.gpg, .asc, .sig) are handled with GnuPG automatically.";

enum Failure {
    Usage,
    Refused(String),
    Error(String),
}

type Res = Result<(), Failure>;

struct Ctx {
    tools: ToolPaths,
    home: PathBuf,
    keys: KeyStore,
}

/// Who a file or message is encrypted for.
enum Recipients {
    Passphrase,
    Keys {
        me: bool,
        to: Vec<String>,
    },
    /// OpenPGP keys (fingerprints) in the GnuPG keyring.
    OpenPgp(Vec<String>),
}

fn main() -> ExitCode {
    install_signal_handlers();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match argv.as_slice() {
        ["--version"] => {
            println!("resilience-crypto {}", resilience_core::VERSION);
            Ok(())
        }
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        [] => Err(Failure::Usage),
        [op, rest @ ..] => ctx().and_then(|c| dispatch(&c, op, rest)),
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

fn dispatch(c: &Ctx, op: &str, rest: &[&str]) -> Res {
    let file = |rest: &[&str]| match rest {
        [f] => Ok(PathBuf::from(f)),
        _ => Err(Failure::Usage),
    };
    match op {
        "encrypt" => match rest.split_last() {
            Some((f, to)) => encrypt(c, Path::new(f), &recipients(to)?),
            None => Err(Failure::Usage),
        },
        "decrypt" => decrypt(c, &file(rest)?),
        "write-message" => write_message(c, &recipients(rest)?),
        "read-message" => read_message(c, &file(rest)?),
        "sign" => sign(c, &file(rest)?),
        "verify" => verify(c, &file(rest)?),
        "checksum" => checksum(c, &file(rest)?),
        "compare" => match rest {
            [sum, f] => compare(c, Path::new(f), sum),
            _ => Err(Failure::Usage),
        },
        "check-list" => check_list(c, &file(rest)?),
        "inspect" => inspect(c, &file(rest)?),
        "new-keys" if rest.is_empty() => new_keys(c),
        "add-contact" => match rest {
            [] => add_contact(c, None),
            [f] => add_contact(c, Some(Path::new(f))),
            _ => Err(Failure::Usage),
        },
        "export-keys" => export_keys(c, &file(rest)?),
        "new-openpgp-key" if rest.is_empty() => pgp::new_key(c),
        _ => Err(Failure::Usage),
    }
}

fn ctx() -> Result<Ctx, Failure> {
    let tools = SystemConfig::load()
        .map(|(c, _, _)| c.tools)
        .map_err(|e| Failure::Error(format!("Cannot read the system configuration: {e}")))?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or_else(|| Failure::Error("HOME is not set.".into()))?;
    let keys = KeyStore::for_home(&home);
    for d in [crypto::DECRYPTED_DIR, crypto::MESSAGES_DIR] {
        clean_stale_partials(&home.join(d));
    }
    clean_stale_partials(&keys.own_dir());
    Ok(Ctx { tools, home, keys })
}

fn recipients(args: &[&str]) -> Result<Recipients, Failure> {
    match args {
        ["--passphrase"] => Ok(Recipients::Passphrase),
        _ => {
            let (mut me, mut to, mut pgp) = (false, Vec::new(), Vec::new());
            let mut it = args.iter();
            while let Some(a) = it.next() {
                match *a {
                    "--me" => me = true,
                    "--to" => to.push(it.next().ok_or(Failure::Usage)?.to_string()),
                    "--to-pgp" => {
                        let f = it.next().ok_or(Failure::Usage)?.to_ascii_uppercase();
                        if !resilience_core::openpgp::is_fingerprint(&f) {
                            return Err(Failure::Usage);
                        }
                        pgp.push(f);
                    }
                    _ => return Err(Failure::Usage),
                }
            }
            match (me || !to.is_empty(), !pgp.is_empty()) {
                (true, false) => Ok(Recipients::Keys { me, to }),
                (false, true) => Ok(Recipients::OpenPgp(pgp)),
                // age and OpenPGP keys cannot be mixed in one file.
                _ => Err(Failure::Usage),
            }
        }
    }
}

/// age arguments for `r`, and who it is for (in words).
fn recipient_args(c: &Ctx, r: &Recipients) -> Result<(Vec<String>, String), Failure> {
    match r {
        Recipients::Passphrase => Ok((vec!["--passphrase".into()], "a passphrase".into())),
        Recipients::OpenPgp(f) => Ok((Vec::new(), pgp::names(c, f)?)),
        Recipients::Keys { me, to } => {
            let mut args = Vec::new();
            let mut who = Vec::new();
            if *me {
                let k = c.keys.own().encryption.ok_or_else(|| {
                    Failure::Refused(
                        "You have no encryption key yet. Create one in Keys & contacts.".into(),
                    )
                })?;
                args.extend(["--recipient".to_string(), k]);
                who.push("you".to_string());
            }
            for name in to {
                let contact = c.keys.contact(name).ok_or_else(|| {
                    Failure::Refused(format!("{} is not one of your contacts.", printable(name)))
                })?;
                let k = contact.age.ok_or_else(|| {
                    Failure::Refused(format!(
                        "{} has no encryption key in your contacts.",
                        printable(name)
                    ))
                })?;
                args.extend(["--recipient".to_string(), k]);
                who.push(name.clone());
            }
            Ok((args, who.join(", ")))
        }
    }
}

// ----- small helpers ------------------------------------------------------------

fn say(s: &str) {
    println!("{s}");
}

fn heading(s: &str) {
    println!("\n{s}\n{}", "=".repeat(s.chars().count().min(76)));
}

fn shown(p: &Path) -> String {
    printable(&p.to_string_lossy())
}

/// Ask a question on the terminal; `None` at end of input.
fn ask(prompt: &str) -> Option<String> {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
    }
}

fn yes(prompt: &str) -> bool {
    ask(&format!("{prompt} [y/N] "))
        .map(|a| matches!(a.trim().to_lowercase().as_str(), "y" | "yes" | "o" | "oui"))
        .unwrap_or(false)
}

/// The first bytes of a regular file (refuses folders, devices, pipes).
fn head_of(path: &Path) -> Result<(fs::Metadata, Vec<u8>), Failure> {
    let meta = fs::metadata(path)
        .map_err(|e| Failure::Refused(format!("Cannot open {}: {e}", shown(path))))?;
    if !meta.is_file() {
        return Err(Failure::Refused(format!("{} is not a file.", shown(path))));
    }
    let mut f = fs::File::open(path)
        .map_err(|e| Failure::Refused(format!("Cannot read {}: {e}", shown(path))))?;
    let mut buf = vec![0u8; 16 * 1024];
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) => return Err(Failure::Error(format!("Cannot read {}: {e}", shown(path)))),
        }
    }
    buf.truncate(n);
    Ok((meta, buf))
}

/// Absolute path, so that a name starting with `-` is never read as an option.
fn absolute(p: &Path) -> Result<PathBuf, Failure> {
    std::path::absolute(p).map_err(|e| Failure::Error(format!("{}: {e}", shown(p))))
}

/// Read a small file whole (keys, signatures, lists).
fn read_small(p: &Path, max: u64) -> Result<String, Failure> {
    let (meta, _) = head_of(p)?;
    if meta.len() > max {
        return Err(Failure::Refused(format!(
            "{} is too big for this ({}).",
            shown(p),
            human_bytes(meta.len())
        )));
    }
    let bytes = fs::read(p).map_err(|e| Failure::Error(format!("{}: {e}", shown(p))))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Output written to a hidden partial file next to its final place, renamed
/// only when complete, removed on failure.
struct Partial {
    path: PathBuf,
    keep: bool,
}

/// The partial file being written, for the signal handler: Ctrl-C (which
/// the banner offers), a hang-up or a kill must not leave partial
/// plaintext behind. One partial file exists at a time.
static PARTIAL: std::sync::atomic::AtomicPtr<libc::c_char> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

extern "C" fn on_signal(_: libc::c_int) {
    let p = PARTIAL.swap(std::ptr::null_mut(), std::sync::atomic::Ordering::SeqCst);
    // SAFETY: unlink and _exit are async-signal-safe; p is a leaked,
    // NUL-terminated CString or NULL.
    unsafe {
        if !p.is_null() {
            libc::unlink(p);
        }
        libc::_exit(130);
    }
}

/// Catch SIGINT/SIGTERM/SIGHUP. A caught signal is reset to the default in
/// programs we start (exec), so Ctrl-C still stops age, gpg or minisign.
fn install_signal_handlers() {
    for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: installing a handler that only calls async-signal-safe
        // functions.
        unsafe {
            libc::signal(sig, on_signal as *const () as libc::sighandler_t);
        }
    }
}

fn set_partial(path: Option<&Path>) {
    let new = path
        .and_then(|p| std::ffi::CString::new(p.as_os_str().as_bytes()).ok())
        .map(|c| c.into_raw())
        .unwrap_or(std::ptr::null_mut());
    let old = PARTIAL.swap(new, std::sync::atomic::Ordering::SeqCst);
    if !old.is_null() {
        // SAFETY: old came from CString::into_raw above and is no longer
        // reachable by the handler (swapped out).
        drop(unsafe { std::ffi::CString::from_raw(old) });
    }
}

/// Remove partial files left by an earlier run that was killed without its
/// handler (SIGKILL, power loss): `.NAME.partial-PID` whose PID is gone.
fn clean_stale_partials(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(n) = name.to_str() else { continue };
        let Some(pid) = n
            .rsplit_once(".partial-")
            .and_then(|(_, p)| p.parse::<i32>().ok())
        else {
            continue;
        };
        // SAFETY: kill with signal 0 only checks whether the process exists.
        let gone = unsafe { libc::kill(pid, 0) } != 0
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if n.starts_with('.') && gone {
            let _ = fs::remove_file(e.path());
        }
    }
}

impl Partial {
    fn create(target: &Path, mode: u32) -> Result<(Partial, fs::File), Failure> {
        let dir = target.parent().unwrap_or(Path::new("."));
        let name = target.file_name().unwrap_or(OsStr::new("output"));
        let mut tmp_name = b".".to_vec();
        tmp_name.extend_from_slice(name.as_bytes());
        tmp_name.extend_from_slice(format!(".partial-{}", std::process::id()).as_bytes());
        let path = dir.join(OsStr::from_bytes(&tmp_name));
        let f = create_new(&path, mode)
            .map_err(|e| Failure::Refused(format!("Cannot write in {}: {e}", shown(dir))))?;
        set_partial(Some(&path));
        Ok((Partial { path, keep: false }, f))
    }

    fn finish(mut self, file: &fs::File, target: &Path) -> Res {
        file.sync_all()
            .map_err(|e| Failure::Error(format!("Cannot save {}: {e}", shown(target))))?;
        rename_noreplace(&self.path, target)
            .map_err(|e| Failure::Error(format!("Cannot save {}: {e}", shown(target))))?;
        set_partial(None);
        self.keep = true;
        Ok(())
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.keep {
            set_partial(None);
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// A beginner's version of an age error.
fn age_hint(err: &str) -> &'static str {
    let e = err.to_lowercase();
    if e.contains("incorrect passphrase") {
        "The passphrase is not the right one."
    } else if e.contains("no identity matched") {
        "This was not encrypted for your key."
    } else if e.contains("could not read passphrase") {
        "The passphrase could not be asked: there is no terminal."
    } else if e.contains("interrupt") {
        "Cancelled."
    } else if e.contains("malformed")
        || e.contains("header")
        || e.contains("unexpected eof")
        || e.contains("chunk")
    {
        "The file is damaged or was altered, or is not complete."
    } else {
        ""
    }
}

/// age's error lines, without its "report unexpected errors" footer.
fn age_message(err: &str) -> String {
    err.lines()
        .filter(|l| !l.contains("report unexpected or unhelpful errors"))
        .map(printable)
        .collect::<Vec<_>>()
        .join("\n")
}

fn age_failure(what: &str, e: &backup::PipelineError) -> Failure {
    Failure::Error(format!(
        "{what} failed. {}\n\n({})",
        age_hint(&e.stderr),
        age_message(e.stderr.trim())
    ))
}

fn ensure_private_dir(p: &Path) -> Result<(), Failure> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(p)
        .map_err(|e| Failure::Error(format!("Cannot create {}: {e}", shown(p))))
}

fn stamp() -> String {
    backup::compact_stamp(&resilience_core::admin::now_rfc3339())
}

// ----- encrypt / decrypt -------------------------------------------------------

fn encrypt(c: &Ctx, file: &Path, r: &Recipients) -> Res {
    if let Recipients::OpenPgp(f) = r {
        return pgp::encrypt(c, file, f);
    }
    let file = absolute(file)?;
    let (meta, head) = head_of(&file)?;
    if detect(&head).is_age() {
        return Err(Failure::Refused(format!(
            "{} is already encrypted with age.",
            shown(&file)
        )));
    }
    let (args, who) = recipient_args(c, r)?;
    let dir = file.parent().unwrap_or(Path::new("/"));
    let name = file.file_name().unwrap_or(OsStr::new("file"));
    let target = free_path(dir, &crypto::encrypted_name(name))
        .ok_or_else(|| Failure::Refused("Too many files with this name.".into()))?;
    heading("ENCRYPT A FILE");
    say(&format!(
        "File:   {} ({})\nFor:    {who}\nResult: {}",
        shown(&file),
        human_bytes(meta.len()),
        shown(&target)
    ));
    if matches!(r, Recipients::Passphrase) {
        say(
            "\nChoose a long passphrase (several words). If it is forgotten, the file \
             can never be opened again.\nLeave it empty to let age make up a strong one \
             (write it down!).\n",
        );
    }
    let (partial, out) = Partial::create(&target, 0o644)?;
    let mut argv = vec![c.tools.age.clone(), "--encrypt".into()];
    argv.extend(args);
    argv.push(file.to_string_lossy().into_owned());
    // With a passphrase, age's messages (including a made-up passphrase)
    // must reach the terminal.
    let capture = !matches!(r, Recipients::Passphrase);
    let out_for_age = out.try_clone().map_err(|e| Failure::Error(e.to_string()))?;
    run_pipeline(
        &[Stage::new(argv, capture)],
        Stdio::null(),
        Stdio::from(out_for_age),
    )
    .map_err(|e| age_failure("Encryption", &e))?;
    partial.finish(&out, &target)?;
    heading("DONE");
    say(&format!(
        "Encrypted copy: {}\n\nThe original file is unchanged: it is still readable by \
         anyone who can open it. Deleting it may not erase it completely from USB sticks \
         and SSDs.",
        shown(&target)
    ));
    Ok(())
}

fn decrypt(c: &Ctx, file: &Path) -> Res {
    let file = absolute(file)?;
    let (meta, head) = head_of(&file)?;
    let fmt = detect(&head);
    if is_pgp_message(&fmt) {
        return pgp::decrypt(c, &file, meta.len());
    }
    if !fmt.is_age() {
        return Err(not_age(&fmt, &file));
    }
    let id_args = identity_args(c, &fmt)?;
    let dir = c.home.join(crypto::DECRYPTED_DIR);
    ensure_private_dir(&dir)?;
    let name = file.file_name().unwrap_or(OsStr::new("file"));
    let target = free_path(&dir, &crypto::decrypted_name(name))
        .ok_or_else(|| Failure::Refused("Too many files with this name.".into()))?;
    heading("DECRYPT A FILE");
    say(&format!(
        "File:   {} ({})\nResult: {}\n",
        shown(&file),
        human_bytes(meta.len()),
        shown(&target)
    ));
    say(if fmt.age_passphrase() {
        "Type the passphrase of this file."
    } else {
        "Type the passphrase of your encryption key."
    });
    let (partial, out) = Partial::create(&target, 0o600)?;
    let mut argv = vec![c.tools.age.clone(), "--decrypt".into()];
    argv.extend(id_args);
    argv.push(file.to_string_lossy().into_owned());
    let out_for_age = out.try_clone().map_err(|e| Failure::Error(e.to_string()))?;
    run_pipeline(
        &[Stage::new(argv, true)],
        Stdio::null(),
        Stdio::from(out_for_age),
    )
    .map_err(|e| age_failure("Decryption", &e))?;
    partial.finish(&out, &target)?;
    heading("DONE");
    say(&format!(
        "Decrypted into: {}\n\nage checked that the content was not changed since it was \
         encrypted. That does not prove who wrote it: anyone with your public key can \
         encrypt for you (a signature proves the author). The decrypted copy is readable \
         by your account only; delete it when you no longer need it.",
        shown(&target)
    ));
    Ok(())
}

/// An OpenPGP encrypted message (not a key or a detached signature).
fn is_pgp_message(fmt: &Format) -> bool {
    matches!(fmt, Format::OpenPgp { what, .. } if what.contains("message"))
}

fn not_age(fmt: &Format, file: &Path) -> Failure {
    let (title, text) = crypto::describe(fmt);
    Failure::Refused(format!(
        "{} is not an age encrypted file.\n\nIt looks like: {title}.\n{text}",
        shown(file)
    ))
}

/// `-i` with the user's key, unless the file opens with a passphrase.
fn identity_args(c: &Ctx, fmt: &Format) -> Result<Vec<String>, Failure> {
    if fmt.age_passphrase() {
        return Ok(Vec::new());
    }
    if !c.keys.has_encryption_key() {
        return Err(Failure::Refused(
            "This was encrypted for someone's key, and you have no encryption key. Only \
             the people it was encrypted for can open it."
                .into(),
        ));
    }
    Ok(vec![
        "--identity".into(),
        c.keys.encryption_key().to_string_lossy().into_owned(),
    ])
}

// ----- messages ------------------------------------------------------------------

fn write_message(c: &Ctx, r: &Recipients) -> Res {
    let (args, who) = recipient_args(c, r)?;
    heading("WRITE AN ENCRYPTED MESSAGE");
    say(&format!(
        "For: {who}\n\nType your message. To finish, type a line with only a dot (.) or \
         press Ctrl-D at the start of a line. Ctrl-C cancels.\n"
    ));
    let mut text = String::new();
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| Failure::Error(e.to_string()))?;
        if line == "." {
            break;
        }
        if text.len() + line.len() + 1 > MAX_MESSAGE {
            return Err(Failure::Refused("The message is too long.".into()));
        }
        text.push_str(&line);
        text.push('\n');
    }
    if text.trim().is_empty() {
        return Err(Failure::Refused(
            "The message is empty: nothing was saved.".into(),
        ));
    }
    if let Recipients::OpenPgp(f) = r {
        return pgp::write_message(c, &text, f);
    }
    let dir = c.home.join(crypto::MESSAGES_DIR);
    ensure_private_dir(&dir)?;
    let target = free_path(&dir, OsStr::new(&format!("message-{}.age.txt", stamp())))
        .ok_or_else(|| Failure::Refused("Too many messages with this name.".into()))?;
    if matches!(r, Recipients::Passphrase) {
        say("\nChoose a passphrase. The person reading the message needs it.\n");
    }
    let (partial, out) = Partial::create(&target, 0o600)?;
    let mut argv = vec![c.tools.age.clone(), "--encrypt".into(), "--armor".into()];
    argv.extend(args);
    let out_for_age = out.try_clone().map_err(|e| Failure::Error(e.to_string()))?;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(out_for_age))
        .stderr(if matches!(r, Recipients::Passphrase) {
            Stdio::inherit()
        } else {
            Stdio::piped()
        })
        .spawn()
        .map_err(|e| Failure::Error(format!("Cannot start age: {e}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Failure::Error("no pipe".into()))?;
    let writer = std::thread::spawn(move || stdin.write_all(text.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|e| Failure::Error(e.to_string()))?;
    let _ = writer.join();
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(Failure::Error(format!(
            "Encryption failed. {}\n\n({})",
            age_hint(&err),
            age_message(err.trim())
        )));
    }
    partial.finish(&out, &target)?;
    let armored = fs::read_to_string(&target).unwrap_or_default();
    heading("DONE");
    say(&format!(
        "Saved: {}\n\nThis text can be copied, sent or printed; only {who} can read it:\n",
        shown(&target)
    ));
    say(&armored);
    Ok(())
}

fn read_message(c: &Ctx, file: &Path) -> Res {
    let file = absolute(file)?;
    let (meta, head) = head_of(&file)?;
    let fmt = detect(&head);
    if meta.len() > 4 * MAX_MESSAGE as u64 {
        return Err(Failure::Refused(
            "This is too big for a message: use \"Decrypt a file\".".into(),
        ));
    }
    if is_pgp_message(&fmt) {
        return pgp::read_message(c, &file);
    }
    if !fmt.is_age() {
        return Err(not_age(&fmt, &file));
    }
    if meta.len() > 4 * MAX_MESSAGE as u64 {
        return Err(Failure::Refused(
            "This is too big for a message: use \"Decrypt a file\".".into(),
        ));
    }
    let id_args = identity_args(c, &fmt)?;
    heading("READ AN ENCRYPTED MESSAGE");
    say(&format!("Message: {}\n", shown(&file)));
    say(if fmt.age_passphrase() {
        "Type the passphrase of this message."
    } else {
        "Type the passphrase of your encryption key."
    });
    let mut argv = vec![c.tools.age.clone(), "--decrypt".to_string()];
    argv.extend(id_args);
    argv.push(file.to_string_lossy().into_owned());
    // The plaintext stays in memory: it is never written to a file.
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Failure::Error(format!("Cannot start age: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Failure::Error(format!(
            "Decryption failed. {}\n\n({})",
            age_hint(&err),
            age_message(err.trim())
        )));
    }
    if out.stdout.contains(&0) {
        return Err(Failure::Refused(
            "This is a file, not a text message: use \"Decrypt a file\".".into(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let page = format!(
        "DECRYPTED MESSAGE (shown only, not saved). age checked that it was not changed \
         since it was encrypted; that does not prove who wrote it.\n\
         Press q to close.\n\n{text}"
    );
    show_text(c, &page)
}

/// Show text in `less` (secure mode: no shell, no files, no editor) when on
/// a terminal, else print it.
fn show_text(c: &Ctx, text: &str) -> Res {
    // SAFETY: isatty only inspects a descriptor.
    if unsafe { libc::isatty(1) } != 1 {
        say(text);
        return Ok(());
    }
    let mut cmd = Command::new(&c.tools.less);
    cmd.env_clear()
        .env("LESSSECURE", "1")
        .env("LESSHISTFILE", "-")
        .stdin(Stdio::piped());
    for k in ["TERM", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Failure::Error(format!("Cannot start less: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
    Ok(())
}

// ----- signatures ----------------------------------------------------------------

fn sign(c: &Ctx, file: &Path) -> Res {
    let file = absolute(file)?;
    let (meta, _) = head_of(&file)?;
    if !c.keys.has_signing_key() {
        return Err(Failure::Refused(
            "You have no signing key yet. Create one in Keys & contacts.".into(),
        ));
    }
    let mut sig_name = file
        .file_name()
        .unwrap_or(OsStr::new("file"))
        .to_os_string();
    sig_name.push(".minisig");
    let target = file.with_file_name(&sig_name);
    if fs::symlink_metadata(&target).is_ok() {
        return Err(Failure::Refused(format!(
            "A signature already exists: {}. It was not replaced.",
            shown(&target)
        )));
    }
    heading("SIGN A FILE");
    say(&format!(
        "File:      {} ({})\nSignature: {}\n\nType the password of your signing key.",
        shown(&file),
        human_bytes(meta.len()),
        shown(&target)
    ));
    let (partial, out) = Partial::create(&target, 0o644)?;
    let argv = [
        c.tools.minisign.clone(),
        "-S".into(),
        "-s".into(),
        c.keys.signing_key().to_string_lossy().into_owned(),
        "-x".into(),
        partial.path.to_string_lossy().into_owned(),
        "-m".into(),
        file.to_string_lossy().into_owned(),
    ];
    let st = Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|e| Failure::Error(format!("Cannot start minisign: {e}")))?;
    if !st.success() {
        return Err(Failure::Error(
            "Signing failed (wrong password, or cancelled).".into(),
        ));
    }
    partial.finish(&out, &target)?;
    let id = c.keys.own().signing.as_deref().and_then(minisign_key_id);
    heading("DONE");
    say(&format!(
        "Signature saved: {}\n\nGive it together with the file. People who have your \
         public signing key (number {}) can check that the file comes from you and was not \
         changed.",
        shown(&target),
        id.unwrap_or_else(|| "?".into())
    ));
    Ok(())
}

fn verify(c: &Ctx, file: &Path) -> Res {
    let file = absolute(file)?;
    let name = file
        .file_name()
        .unwrap_or(OsStr::new(""))
        .as_bytes()
        .to_vec();
    let (data, sig) = if let Some(stem) = name.strip_suffix(b".minisig") {
        (file.with_file_name(OsStr::from_bytes(stem)), file.clone())
    } else {
        let mut s = name.clone();
        s.extend_from_slice(b".minisig");
        (file.clone(), file.with_file_name(OsStr::from_bytes(&s)))
    };
    // OpenPGP (GnuPG) signatures: NAME.sig, NAME.asc, or a signed text.
    if !sig.is_file() {
        if let Some((pgp_sig, pgp_data)) = pgp::signature_for(&file) {
            return pgp::verify(c, &pgp_sig, pgp_data.as_deref());
        }
    }
    head_of(&data)?;
    heading("CHECK A SIGNATURE");
    say(&format!("File:      {}", shown(&data)));
    if !sig.is_file() {
        return Err(Failure::Refused(format!(
            "No signature found. A signature is a file next to the signed file, named \
             {} (minisign) or ending in .sig or .asc (OpenPGP).",
            shown(&sig)
        )));
    }
    say(&format!("Signature: {}", shown(&sig)));
    let sig_text = read_small(&sig, MAX_KEY_FILE)?;
    let key_id = crypto::minisign_signature_key_id(&sig_text).ok_or_else(|| {
        Failure::Refused("This is not a minisign signature, or it is damaged.".into())
    })?;
    let Some(signer) = c.keys.signer(&key_id) else {
        return Err(Failure::Refused(format!(
            "\nNOT CHECKED: this file was signed with key number {key_id}, which is not \
             one of your contacts' keys.\n\nIf you know who signed it, add their public \
             key to your contacts (after checking it with them), then try again."
        )));
    };
    let who = signer
        .name
        .clone()
        .map(|n| format!("your contact {}", printable(&n)))
        .unwrap_or_else(|| "your own key".into());
    let out = Command::new(&c.tools.minisign)
        .args(["-V", "-P", &signer.public_key, "-x"])
        .arg(&sig)
        .arg("-m")
        .arg(&data)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Failure::Error(format!("Cannot start minisign: {e}")))?;
    if out.status.success() {
        let comment = crypto::minisign_trusted_comment(&sig_text)
            .map(|t| crypto::describe_trusted_comment(&t))
            .unwrap_or_default();
        heading("VALID SIGNATURE");
        say(&format!(
            "Signed by: {who} (key {key_id})\nSigned note: {}\n\n\
             This proves that the file was not changed since that key signed it. It does \
             not prove that the file is safe, nor that the key really belongs to that \
             person: that depends on how you checked their key.",
            printable(&comment)
        ));
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(Failure::Refused(format!(
            "\nINVALID SIGNATURE\n=================\nThe file does NOT match the signature \
             of {who}: it was changed after signing, or the signature is damaged. Do not \
             trust this file.\n\n({})",
            printable(err.trim())
        )))
    }
}

// ----- checksums -----------------------------------------------------------------

fn tool_for(c: &Ctx, a: Algo) -> &str {
    match a {
        Algo::Md5 => &c.tools.md5sum,
        Algo::Sha1 => &c.tools.sha1sum,
        Algo::Sha256 => &c.tools.sha256sum,
        Algo::Sha512 => &c.tools.sha512sum,
        Algo::Blake2b => &c.tools.b2sum,
    }
}

fn digest(c: &Ctx, a: Algo, file: &Path) -> Result<String, Failure> {
    // LC_ALL=C: the output is parsed, it must not be translated.
    let out = Command::new(tool_for(c, a))
        .env("LC_ALL", "C")
        .arg("--")
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Failure::Error(format!("Cannot start {}: {e}", a.program())))?;
    let text = String::from_utf8_lossy(&out.stdout);
    match crypto::digest_of_output(&text) {
        Some(h) if out.status.success() => Ok(h),
        _ => Err(Failure::Error(format!(
            "{} failed: {}",
            a.program(),
            printable(String::from_utf8_lossy(&out.stderr).trim())
        ))),
    }
}

fn checksum(c: &Ctx, file: &Path) -> Res {
    let file = absolute(file)?;
    let (meta, _) = head_of(&file)?;
    heading("CHECKSUMS");
    say(&format!(
        "File: {} ({})\nComputing... (large files take a while)\n",
        shown(&file),
        human_bytes(meta.len())
    ));
    for a in [Algo::Sha256, Algo::Sha512] {
        let h = digest(c, a, &file)?;
        say(&format!("{}:\n  {}\n", a.name(), crypto::grouped(&h)));
    }
    say(
        "A checksum identifies the exact content: any change gives a completely \
         different one. Compare it with one from a source you trust.",
    );
    Ok(())
}

fn compare(c: &Ctx, file: &Path, expected: &str) -> Res {
    let file = absolute(file)?;
    let (meta, _) = head_of(&file)?;
    let (want, algos) = crypto::parse_expected(expected).map_err(Failure::Refused)?;
    heading("COMPARE WITH AN EXPECTED CHECKSUM");
    say(&format!(
        "File:     {} ({})\nExpected: {}\nComputing...\n",
        shown(&file),
        human_bytes(meta.len()),
        crypto::grouped(&want)
    ));
    let mut got = Vec::new();
    for &a in algos {
        let h = digest(c, a, &file)?;
        if h == want {
            heading("MATCH");
            say(&format!(
                "The file's {} is exactly the expected one.",
                a.name()
            ));
            if a.weak() {
                say(&format!(
                    "\nWarning: {} is an old, broken algorithm. The match shows the file \
                     was not damaged by accident, but someone could have made a different \
                     file with the same checksum. Prefer SHA-256 or a signature.",
                    a.name()
                ));
            }
            say("\nThis is only as trustworthy as the place the expected checksum came from.");
            return Ok(());
        }
        got.push((a, h));
    }
    let lines: Vec<String> = got
        .iter()
        .map(|(a, h)| format!("{}: {}", a.name(), crypto::grouped(h)))
        .collect();
    Err(Failure::Refused(format!(
        "\nDOES NOT MATCH\n==============\nThe file is not the expected one: it is \
         damaged, incomplete, a different version, or was altered.\n\nThis file: {}",
        lines.join("\n           ")
    )))
}

fn check_list(c: &Ctx, list: &Path) -> Res {
    let list = absolute(list)?;
    let text = read_small(&list, 4 * 1024 * 1024)?;
    let algo = crypto::checksum_list_algorithm(&text).ok_or_else(|| {
        Failure::Refused(
            "This is not a checksum list (lines like \"<checksum>  <file name>\").".into(),
        )
    })?;
    let dir = list.parent().unwrap_or(Path::new("/"));
    heading("CHECK A LIST OF CHECKSUMS");
    say(&format!(
        "List: {} ({})\nFiles are looked for in {}. Checking...\n",
        shown(&list),
        algo.name(),
        shown(dir)
    ));
    let out = Command::new(tool_for(c, algo))
        .env("LC_ALL", "C")
        .args(["--check", "--strict", "--"])
        .arg(&list)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Failure::Error(format!("Cannot start {}: {e}", algo.program())))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (mut ok, mut bad) = (0, 0);
    for line in stdout.lines() {
        if line.ends_with(": OK") {
            ok += 1;
        } else {
            bad += 1;
        }
        say(&format!("  {}", printable(line)));
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        say(&format!("\n{}", printable(err.trim())));
    }
    if out.status.success() && bad == 0 && ok > 0 {
        heading("ALL FILES MATCH");
        say(&format!("{ok} file(s) are exactly as listed."));
        if algo.weak() {
            say(&format!(
                "Warning: {} is an old, broken algorithm (accidental damage only).",
                algo.name()
            ));
        }
        Ok(())
    } else {
        Err(Failure::Refused(format!(
            "\nPROBLEMS FOUND\n==============\n{ok} file(s) match; the others are missing, \
             damaged or different (see above)."
        )))
    }
}

// ----- inspect -------------------------------------------------------------------

fn inspect(c: &Ctx, file: &Path) -> Res {
    let file = absolute(file)?;
    let (meta, head) = head_of(&file)?;
    let fmt = detect(&head);
    let (title, text) = crypto::describe(&fmt);
    heading("WHAT IS THIS FILE?");
    say(&format!(
        "File: {} ({})\n\n{title}\n\n{text}",
        shown(&file),
        human_bytes(meta.len())
    ));
    if fmt.is_age() {
        if let Ok(o) = Command::new(&c.tools.age_inspect)
            .arg(&file)
            .stdin(Stdio::null())
            .output()
        {
            let t = String::from_utf8_lossy(&o.stdout);
            let t = t.split("Tip:").next().unwrap_or("");
            if o.status.success() && !t.trim().is_empty() {
                say(&format!(
                    "\nDetails (age-inspect):\n{}",
                    printable(t.trim_end())
                ));
            }
        }
    }
    if is_pgp_message(&fmt) {
        pgp::inspect(c, &file);
    }
    if let Format::MinisignSignature = fmt {
        if let Some(id) = crypto::minisign_signature_key_id(&String::from_utf8_lossy(&head)) {
            let who = match c.keys.signer(&id) {
                Some(s) => s
                    .name
                    .map(|n| format!("your contact {}", printable(&n)))
                    .unwrap_or_else(|| "your own key".into()),
                None => "a key not in your contacts".into(),
            };
            say(&format!("\nMade with key number {id}: {who}."));
        }
    }
    Ok(())
}

// ----- keys ----------------------------------------------------------------------

fn new_keys(c: &Ctx) -> Res {
    heading("CREATE YOUR KEYS");
    c.keys
        .ensure_dirs()
        .map_err(|e| Failure::Error(format!("Cannot create the key folder: {e}")))?;
    let (has_enc, has_sig) = (c.keys.has_encryption_key(), c.keys.has_signing_key());
    if has_enc && has_sig {
        say("You already have both keys. They are never replaced from here.");
        return Ok(());
    }
    say(
        "Your keys are protected by passphrases that you choose now. Without them, the \
         keys are useless: nothing encrypted for you can ever be opened again, and no \
         one can recover it. Write them down and keep them safe.\n\n\
         Keys are saved in your home folder (~/.local/share/resilience/keys); \
         \"Backups & USB sticks\" includes them in your encrypted backups.",
    );
    if !has_enc {
        say("\n1. ENCRYPTION KEY (age): choose its passphrase.");
        say("   (Leave it empty to let age make up a strong one: write it down!)\n");
        let target = c.keys.encryption_key();
        let (partial, out) = Partial::create(&target, 0o600)?;
        let out_for_age = out.try_clone().map_err(|e| Failure::Error(e.to_string()))?;
        // The secret key only exists in the pipe between the two programs:
        // it reaches the disk already encrypted with the passphrase.
        let warnings = run_pipeline(
            &[
                Stage::new(vec![c.tools.age_keygen.clone()], true),
                Stage::new(
                    vec![
                        c.tools.age.clone(),
                        "--encrypt".into(),
                        "--passphrase".into(),
                        "--armor".into(),
                    ],
                    false,
                ),
            ],
            Stdio::null(),
            Stdio::from(out_for_age),
        )
        .map_err(|e| age_failure("Creating the encryption key", &e))?;
        let public = find_public_keys(&warnings)
            .age
            .into_iter()
            .next()
            .ok_or_else(|| Failure::Error("age-keygen did not print the public key.".into()))?;
        write_public(&c.keys.encryption_pub(), &format!("{public}\n"))?;
        partial.finish(&out, &target)?;
        say(&format!(
            "\n   Created. Public key: {public}\n   (ends with {})",
            crypto::age_short(&public)
        ));
    }
    if !has_sig {
        say(
            "\n2. SIGNING KEY (minisign): choose its password (asked twice). Do not \
             leave it empty: an empty password leaves the key unprotected.\n",
        );
        let tmp = c
            .keys
            .own_dir()
            .join(format!(".new-{}", std::process::id()));
        // Left by an interrupted run (they hold no usable secret: minisign
        // encrypts a secret key before writing it).
        if let Ok(rd) = fs::read_dir(c.keys.own_dir()) {
            for e in rd.flatten() {
                if e.file_name().as_bytes().starts_with(b".new-") {
                    let _ = fs::remove_dir_all(e.path());
                }
            }
        }
        ensure_private_dir(&tmp)?;
        let st = Command::new(&c.tools.minisign)
            .arg("-G")
            .arg("-p")
            .arg(tmp.join("signing.pub"))
            .arg("-s")
            .arg(tmp.join("signing.key"))
            .status();
        let moved = match st {
            Ok(s) if s.success() => {
                rename_noreplace(&tmp.join("signing.key"), &c.keys.signing_key())
                    .and_then(|_| fs::rename(tmp.join("signing.pub"), c.keys.signing_pub()))
            }
            Ok(_) => Err(io::Error::other(
                "minisign failed (passwords differ, or cancelled)",
            )),
            Err(e) => Err(e),
        };
        let _ = fs::remove_dir_all(&tmp);
        moved.map_err(|e| Failure::Error(format!("Creating the signing key failed: {e}")))?;
        let id = c.keys.own().signing.as_deref().and_then(minisign_key_id);
        say(&format!(
            "\n   Created. Key number: {}",
            id.unwrap_or_else(|| "?".into())
        ));
    }
    heading("DONE");
    say(
        "Give your PUBLIC keys to the people you exchange with (Keys & contacts → export). \
         Never give the passphrases or the key files.",
    );
    Ok(())
}

fn write_public(path: &Path, text: &str) -> Res {
    let _ = fs::remove_file(path);
    let mut f = create_new(path, 0o644)
        .map_err(|e| Failure::Error(format!("Cannot write {}: {e}", shown(path))))?;
    f.write_all(text.as_bytes())
        .map_err(|e| Failure::Error(format!("Cannot write {}: {e}", shown(path))))
}

/// age itself checks the key (Bech32 checksum, key type) by encrypting
/// nothing for it.
fn age_accepts(c: &Ctx, key: &str) -> bool {
    Command::new(&c.tools.age)
        .args(["--encrypt", "--recipient", key, "--output", "/dev/null"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn add_contact(c: &Ctx, file: Option<&Path>) -> Res {
    if let Some(f) = file {
        let (_, head) = head_of(f)?;
        if let Format::OpenPgp { .. } = detect(&head) {
            return pgp::add_contact(c, f);
        }
    }
    heading("ADD A CONTACT");
    let text = match file {
        Some(f) => {
            say(&format!("Key file: {}", shown(f)));
            read_small(f, MAX_KEY_FILE)?
        }
        None => {
            say(
                "Type or paste the public key(s): an encryption key (age1...) and/or a \
                 signing key (RW...). Press Enter on an empty line when done.\n",
            );
            let mut t = String::new();
            while let Some(l) = ask("> ") {
                if l.trim().is_empty() {
                    break;
                }
                t.push_str(&l);
                t.push('\n');
            }
            t
        }
    };
    let found = find_public_keys(&text);
    if found.secret {
        return Err(Failure::Refused(
            "STOP: this contains a PRIVATE key. Private keys must never be shared. Nothing \
             was added. If it is someone else's private key, tell them: they should make a \
             new key."
                .into(),
        ));
    }
    if found.age.len() > 1 || found.minisign.len() > 1 {
        return Err(Failure::Refused(
            "This contains several keys of the same kind: add one person at a time.".into(),
        ));
    }
    let age = found.age.into_iter().next();
    let minisign = found.minisign.into_iter().next();
    if age.is_none() && minisign.is_none() {
        return Err(Failure::Refused("No public key found.".into()));
    }
    if let Some(k) = &age {
        if !age_accepts(c, k) {
            return Err(Failure::Refused(
                "The encryption key is not valid (a character is wrong or missing).".into(),
            ));
        }
    }
    say("\nKeys found:");
    if let Some(k) = &age {
        say(&format!(
            "  Encryption (age): {k}\n                    ends with {}",
            crypto::age_short(k)
        ));
    }
    if let Some(k) = &minisign {
        say(&format!(
            "  Signing (minisign): key number {}",
            minisign_key_id(k).unwrap_or_default()
        ));
    }
    say(
        "\nBefore trusting these keys, check them with the person directly (in person or \
         by phone): read the end of the encryption key and the signing key number aloud. \
         A key received by message could have been replaced on the way.\n",
    );
    let name = loop {
        let Some(n) = ask("Name of this contact (Enter alone cancels): ") else {
            return Err(Failure::Refused("Cancelled: nothing was added.".into()));
        };
        let n = n.trim().to_string();
        if n.is_empty() {
            return Err(Failure::Refused("Cancelled: nothing was added.".into()));
        }
        if crypto::valid_contact_name(&n) {
            break n;
        }
        say("Use 1 to 40 letters, digits, spaces or - _ . ' (starting with a letter or digit).");
    };
    let mut contact = c.keys.contact(&name).unwrap_or(Contact {
        name: name.clone(),
        ..Contact::default()
    });
    for (have, new, kind) in [
        (&contact.age, &age, "encryption"),
        (&contact.minisign, &minisign, "signing"),
    ] {
        if let (Some(h), Some(n)) = (have, new) {
            if h != n {
                return Err(Failure::Refused(format!(
                    "{} already has a DIFFERENT {kind} key. A key that changes is a warning \
                     sign: check with them. To replace it, remove the contact first.",
                    printable(&name)
                )));
            }
        }
    }
    contact.age = contact.age.or(age);
    contact.minisign = contact.minisign.or(minisign);
    contact.added = contact
        .added
        .or_else(|| Some(resilience_core::admin::now_rfc3339()));
    if !yes(&format!("Save {} as a contact?", printable(&name))) {
        return Err(Failure::Refused("Cancelled: nothing was added.".into()));
    }
    c.keys
        .save_contact(&contact)
        .map_err(|e| Failure::Error(format!("Cannot save the contact: {e}")))?;
    heading("DONE");
    say(&format!("{} was added to your contacts.", printable(&name)));
    Ok(())
}

fn export_keys(c: &Ctx, dir: &Path) -> Res {
    let own = c.keys.own();
    let dir = absolute(dir)?;
    if !dir.is_dir() {
        return Err(Failure::Refused(format!(
            "{} is not available. Is the stick still plugged in?",
            shown(&dir)
        )));
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "me".into());
    let user: String = user
        .chars()
        .filter(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .collect();
    heading("PUBLIC KEYS SAVED");
    let mut saved = false;
    if own.encryption.is_some() || own.signing.is_some() {
        let date = resilience_core::admin::now_rfc3339();
        let text = crypto::export_text(&user, &own, date.get(..10).unwrap_or(&date));
        let target = free_path(&dir, OsStr::new(&format!("{user}-public-keys.txt")))
            .ok_or_else(|| Failure::Refused("Too many files with this name.".into()))?;
        let (partial, mut out) = Partial::create(&target, 0o644)?;
        out.write_all(text.as_bytes())
            .map_err(|e| Failure::Error(format!("Cannot write: {e}")))?;
        partial.finish(&out, &target)?;
        say(&format!("Saved: {}\n\n{text}", shown(&target)));
        saved = true;
    }
    if let Some(p) = pgp::export(c, &dir, &user)? {
        say(&format!("Saved: {}\n", shown(&p)));
        saved = true;
    }
    if !saved {
        return Err(Failure::Refused(
            "You have no keys yet. Create them in Keys & contacts.".into(),
        ));
    }
    say("These files contain no secret.");
    Ok(())
}
