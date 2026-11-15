//! OpenPGP (GnuPG) operations, for interoperability (ADR 0015).
//!
//! gpg does all the cryptography. Passphrases are asked by gpg-agent's
//! pinentry-curses on the terminal (`GPG_TTY`), never by this program. gpg
//! always runs with `--batch` (no question of its own, results through
//! `--status-fd`) and never fetches keys from the network.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use resilience_core::crypto::{self, Format, MAX_MESSAGE, free_path};
use resilience_core::openpgp::{self, PgpKey, Verdict, grouped_fingerprint};
use resilience_core::printable;

use super::{
    Ctx, Failure, Partial, Res, absolute, ask, ensure_private_dir, heading, read_small, say,
    show_text, shown, stamp, yes,
};

/// A gpg command: batch mode, no key fetching, the terminal for pinentry.
fn gpg(c: &Ctx) -> Command {
    let mut cmd = Command::new(&c.tools.gpg);
    cmd.args([
        "--batch",
        "--no-auto-key-retrieve",
        "--no-auto-key-locate",
        "--status-fd",
        "1",
    ]);
    if let Some(t) = tty_name() {
        cmd.env("GPG_TTY", t);
    }
    cmd.stdin(Stdio::null());
    cmd
}

fn tty_name() -> Option<String> {
    // SAFETY: ttyname returns a pointer to a static buffer or NULL; it is
    // copied at once, and this program has a single thread here.
    let p = unsafe { libc::ttyname(0) };
    if p.is_null() {
        return None;
    }
    // SAFETY: p is a valid NUL-terminated string (checked non-NULL above).
    let s = unsafe { std::ffi::CStr::from_ptr(p) };
    s.to_str().ok().map(str::to_string)
}

fn run(cmd: &mut Command) -> Result<(bool, String, String), Failure> {
    let out = cmd
        .output()
        .map_err(|e| Failure::Error(format!("Cannot start gpg: {e}")))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// Largest text decrypted or verified into memory.
const MAX_SHOWN: usize = 4 * MAX_MESSAGE;

/// Run gpg with its status lines on stderr and the data on stdout, keeping
/// at most [`MAX_SHOWN`] bytes of data in memory: beyond that gpg is
/// stopped (compressed packets can expand enormously). Returns (success,
/// data, status and messages, truncated).
fn capture(c: &Ctx, args: &[&OsStr]) -> Result<(bool, Vec<u8>, String, bool), Failure> {
    use std::io::Read;
    let mut child = Command::new(&c.tools.gpg)
        .args([
            "--batch",
            "--no-auto-key-retrieve",
            "--no-auto-key-locate",
            "--status-fd",
            "2",
        ])
        .args(args)
        .envs(tty_name().map(|t| ("GPG_TTY", t)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Error(format!("Cannot start gpg: {e}")))?;
    let mut err = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.by_ref().take(1024 * 1024).read_to_end(&mut buf);
            let _ = std::io::copy(e, &mut std::io::sink());
        }
        String::from_utf8_lossy(&buf).into_owned()
    });
    let mut data = Vec::new();
    let mut truncated = false;
    if let Some(mut out) = child.stdout.take() {
        let _ = out
            .by_ref()
            .take(MAX_SHOWN as u64 + 1)
            .read_to_end(&mut data);
        if data.len() > MAX_SHOWN {
            truncated = true;
            data.clear();
            let _ = child.kill();
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false) && !truncated;
    let status = reader.join().unwrap_or_default();
    Ok((ok, data, status, truncated))
}

/// gpg's human messages (not the status protocol), for error reports.
fn human_lines(status: &str) -> String {
    status
        .lines()
        .filter(|l| !l.starts_with("[GNUPG:]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The keyring's keys; own keys (with a secret part) are marked.
pub fn list(c: &Ctx) -> Vec<PgpKey> {
    let listing = |what: &str| {
        run(gpg(c).args(["--with-colons", "--fingerprint", "--fingerprint", what]))
            .map(|(_, out, _)| openpgp::parse_keys(&out))
            .unwrap_or_default()
    };
    let secret: Vec<String> = listing("--list-secret-keys")
        .into_iter()
        .map(|k| k.fingerprint)
        .collect();
    let mut keys = listing("--list-keys");
    for k in &mut keys {
        k.secret = secret.contains(&k.fingerprint);
    }
    keys
}

fn who(k: &PgpKey) -> String {
    format!(
        "{}{}",
        printable(&k.uid),
        if k.secret { " (your key)" } else { "" }
    )
}

// ----- encrypt / decrypt ---------------------------------------------------------

pub fn encrypt(c: &Ctx, file: &Path, fprs: &[String]) -> Res {
    let file = absolute(file)?;
    let (meta, head) = super::head_of(&file)?;
    if matches!(
        crypto::detect(&head),
        Format::OpenPgp { .. } | Format::Age { .. }
    ) {
        return Err(Failure::Refused(format!(
            "{} is already encrypted or signed.",
            shown(&file)
        )));
    }
    let keys = recipients(c, fprs)?;
    let dir = file.parent().unwrap_or(Path::new("/"));
    let mut name = file
        .file_name()
        .unwrap_or(OsStr::new("file"))
        .to_os_string();
    name.push(".gpg");
    let target = free_path(dir, &name)
        .ok_or_else(|| Failure::Refused("Too many files with this name.".into()))?;
    heading("ENCRYPT A FILE (OpenPGP)");
    say(&format!(
        "File:   {} ({})\nFor:    {}\nResult: {}",
        shown(&file),
        resilience_core::human_bytes(meta.len()),
        keys.iter().map(who).collect::<Vec<_>>().join(", "),
        shown(&target)
    ));
    let (partial, out) = Partial::create(&target, 0o644)?;
    let mut cmd = gpg(c);
    // The user chose these keys and checked their fingerprints when adding
    // them; gpg's web of trust is not used.
    cmd.args(["--trust-model", "always", "--yes", "--output"])
        .arg(&partial.path);
    for k in &keys {
        cmd.args(["--recipient", &k.fingerprint]);
    }
    cmd.arg("--encrypt").arg(&file);
    let (ok, status, err) = run(&mut cmd)?;
    if !ok || !status.contains("END_ENCRYPTION") {
        return Err(Failure::Error(format!(
            "Encryption failed.\n\n({})",
            printable(err.trim())
        )));
    }
    partial.finish(&out, &target)?;
    heading("DONE");
    say(&format!(
        "Encrypted copy: {}\n\nThe original file is unchanged: it is still readable by \
         anyone who can open it.",
        shown(&target)
    ));
    Ok(())
}

/// The owners of these keys, in words.
pub fn names(c: &Ctx, fprs: &[String]) -> Result<String, Failure> {
    Ok(recipients(c, fprs)?
        .iter()
        .map(who)
        .collect::<Vec<_>>()
        .join(", "))
}

/// Keys for these fingerprints, each able to encrypt.
fn recipients(c: &Ctx, fprs: &[String]) -> Result<Vec<PgpKey>, Failure> {
    let all = list(c);
    fprs.iter()
        .map(|f| {
            let k = all
                .iter()
                .find(|k| &k.fingerprint == f)
                .ok_or_else(|| Failure::Refused(format!("No OpenPGP key {f} in your keyring.")))?;
            if k.unusable || !k.can_encrypt {
                return Err(Failure::Refused(format!(
                    "The key of {} cannot be used to encrypt (expired, revoked or \
                     signing only).",
                    printable(&k.uid)
                )));
            }
            Ok(k.clone())
        })
        .collect()
}

fn decrypt_failure(d: &openpgp::Decryption, err: &str) -> Failure {
    let hint = if d.bad_passphrase {
        "The passphrase is not the right one."
    } else if d.no_secret_key {
        "This was not encrypted for your key (or you have no OpenPGP key)."
    } else if d.ok && !d.integrity {
        "The message is not protected against changes: it was refused."
    } else {
        "The file is damaged, was altered, or the operation was cancelled."
    };
    Failure::Error(format!(
        "Decryption failed. {hint}\n\n({})",
        printable(err.trim())
    ))
}

fn signature_note(v: &Option<Verdict>) -> String {
    match v {
        None => String::new(),
        Some(Verdict::Good {
            uid,
            fingerprint,
            warning,
        }) => format!(
            "\nIt was also SIGNED by {} (fingerprint {}){}.",
            printable(uid),
            grouped_fingerprint(fingerprint),
            warning.map(|w| format!(", but {w}")).unwrap_or_default()
        ),
        Some(Verdict::Bad { .. }) => "\nWARNING: it carries an INVALID signature.".into(),
        Some(Verdict::UnknownKey { key_id }) => {
            format!("\nIt is signed by a key that is not in your keyring ({key_id}).")
        }
        Some(Verdict::Error) => String::new(),
    }
}

pub fn decrypt(c: &Ctx, file: &Path, meta_len: u64) -> Res {
    let dir = c.home.join(crypto::DECRYPTED_DIR);
    ensure_private_dir(&dir)?;
    let name = file.file_name().unwrap_or(OsStr::new("file"));
    let target = free_path(&dir, &crypto::decrypted_name(name))
        .ok_or_else(|| Failure::Refused("Too many files with this name.".into()))?;
    heading("DECRYPT A FILE (OpenPGP)");
    say(&format!(
        "File:   {} ({})\nResult: {}\n\nGnuPG will ask for the passphrase (of your key, or \
         of the file).",
        shown(file),
        resilience_core::human_bytes(meta_len),
        shown(&target)
    ));
    let (partial, out) = Partial::create(&target, 0o600)?;
    let (_, status, err) = run(gpg(c)
        .args(["--yes", "--output"])
        .arg(&partial.path)
        .arg("--decrypt")
        .arg(file))?;
    let d = openpgp::parse_decrypt(&status);
    if !(d.ok && d.integrity) {
        return Err(decrypt_failure(&d, &err));
    }
    partial.finish(&out, &target)?;
    heading("DONE");
    say(&format!(
        "Decrypted into: {}\n\nGnuPG checked that the content was not changed since it \
         was encrypted (that alone does not prove who wrote it).{}\nThe decrypted copy \
         is readable by your account only.",
        shown(&target),
        signature_note(&d.signature)
    ));
    Ok(())
}

pub fn read_message(c: &Ctx, file: &Path) -> Res {
    heading("READ AN ENCRYPTED MESSAGE (OpenPGP)");
    say(&format!(
        "Message: {}\n\nGnuPG will ask for the passphrase.",
        shown(file)
    ));
    // Plaintext on stdout (memory only, bounded), status lines on stderr.
    let (ok, data, status, truncated) = capture(c, &[OsStr::new("--decrypt"), file.as_os_str()])?;
    if truncated {
        return Err(Failure::Refused(
            "This is too big for a message: use \"Decrypt a file\".".into(),
        ));
    }
    let d = openpgp::parse_decrypt(&status);
    if !ok || !(d.ok && d.integrity) {
        return Err(decrypt_failure(&d, &human_lines(&status)));
    }
    if data.contains(&0) {
        return Err(Failure::Refused(
            "This is a file, not a text message: use \"Decrypt a file\".".into(),
        ));
    }
    let out_text = String::from_utf8_lossy(&data);
    let page = format!(
        "DECRYPTED MESSAGE (shown only, not saved). GnuPG checked that it was not \
         changed.{}\nPress q to close.\n\n{}",
        signature_note(&d.signature),
        out_text
    );
    show_text(c, &page)
}

pub fn write_message(c: &Ctx, text: &str, fprs: &[String]) -> Res {
    let keys = recipients(c, fprs)?;
    let dir = c.home.join(crypto::MESSAGES_DIR);
    ensure_private_dir(&dir)?;
    let target = free_path(&dir, OsStr::new(&format!("message-{}.asc", stamp())))
        .ok_or_else(|| Failure::Refused("Too many messages with this name.".into()))?;
    let (partial, out) = Partial::create(&target, 0o600)?;
    let mut cmd = gpg(c);
    cmd.args(["--trust-model", "always", "--yes", "--armor", "--output"])
        .arg(&partial.path);
    for k in &keys {
        cmd.args(["--recipient", &k.fingerprint]);
    }
    let mut child = cmd
        .arg("--encrypt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Error(format!("Cannot start gpg: {e}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Failure::Error("no pipe".into()))?;
    let owned = text.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(owned.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|e| Failure::Error(e.to_string()))?;
    let _ = writer.join();
    let status = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !status.contains("END_ENCRYPTION") {
        return Err(Failure::Error(format!(
            "Encryption failed.\n\n({})",
            printable(String::from_utf8_lossy(&output.stderr).trim())
        )));
    }
    partial.finish(&out, &target)?;
    let armored = std::fs::read_to_string(&target).unwrap_or_default();
    heading("DONE");
    say(&format!(
        "Saved: {}\n\nOnly {} can read it:\n",
        shown(&target),
        keys.iter().map(who).collect::<Vec<_>>().join(", ")
    ));
    say(&armored);
    Ok(())
}

// ----- signatures ----------------------------------------------------------------

/// An OpenPGP signature for `file`: (signature file, signed data). The data
/// is `None` when the signature contains the text (clear-signed or inline).
pub fn signature_for(file: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let kind = |p: &Path| match super::head_of(p) {
        Ok((_, head)) => match crypto::detect(&head) {
            Format::OpenPgp { what, .. } => Some(what),
            _ => None,
        },
        Err(_) => None,
    };
    let name = file.file_name()?.as_encoded_bytes().to_vec();
    for ext in [&b".sig"[..], b".asc", b".gpg"] {
        if let Some(stem) = name.strip_suffix(ext) {
            let data = file.with_file_name(OsStr::new(std::str::from_utf8(stem).ok()?));
            match kind(file) {
                Some("signature") if data.is_file() => {
                    return Some((file.to_path_buf(), Some(data)));
                }
                Some(
                    "text with a signature"
                    | "signed file"
                    | "compressed data (often a signed file)",
                ) => {
                    return Some((file.to_path_buf(), None));
                }
                _ => {}
            }
        }
    }
    for ext in [".sig", ".asc", ".gpg"] {
        let mut s = file.as_os_str().to_os_string();
        s.push(ext);
        let sig = PathBuf::from(s);
        if sig.is_file() && kind(&sig) == Some("signature") {
            return Some((sig, Some(file.to_path_buf())));
        }
    }
    if matches!(kind(file), Some("text with a signature" | "signed file")) {
        return Some((file.to_path_buf(), None));
    }
    None
}

pub fn verify(c: &Ctx, sig: &Path, data: Option<&Path>) -> Res {
    heading("CHECK A SIGNATURE (OpenPGP)");
    say(&format!(
        "Signature: {}{}",
        shown(sig),
        data.map(|d| format!("\nFile:      {}", shown(d)))
            .unwrap_or_default()
    ));
    let Some(data) = data else {
        return verify_inline(c, sig);
    };
    let (_, status, err) = run(gpg(c).arg("--verify").arg(sig).arg(data))?;
    if status_says_not_detached(&err) {
        return Err(Failure::Refused(not_detached(sig)));
    }
    match openpgp::parse_verify(&status) {
        Verdict::Good {
            uid,
            fingerprint,
            warning,
        } => {
            heading("VALID SIGNATURE");
            say(&format!(
                "Signed by: {}\nFingerprint: {}{}\n\n\
                 The file was not changed since this key signed it. Whether the key really \
                 belongs to that person or project is for you to check: compare the \
                 fingerprint above with the one they publish (website, paper, in person). \
                 It does not prove that the file is safe.",
                printable(&uid),
                grouped_fingerprint(&fingerprint),
                warning
                    .map(|w| format!("\nWARNING: {w}."))
                    .unwrap_or_default()
            ));
            Ok(())
        }
        Verdict::Bad { uid } => Err(Failure::Refused(format!(
            "\nINVALID SIGNATURE\n=================\nThe file does NOT match the signature \
             of {}: it was changed after signing, or the signature is damaged. Do not \
             trust this file.",
            printable(&uid)
        ))),
        Verdict::UnknownKey { key_id } => Err(Failure::Refused(format!(
            "\nNOT CHECKED: this was signed with the OpenPGP key {key_id}, which is not in \
             your keyring.\n\nGet the signer's public key from a source you trust, add it in \
             Keys & contacts (a), then try again."
        ))),
        Verdict::Error => Err(Failure::Error(format!(
            "The signature could not be checked.\n\n({})",
            printable(err.trim())
        ))),
    }
}

fn status_says_not_detached(messages: &str) -> bool {
    messages.contains("not a detached signature")
}

fn not_detached(sig: &Path) -> String {
    format!(
        "\nNOT CHECKED: {} contains its own signed text. It does NOT cover any other \
         file, even one with a similar name. Choose this file itself to see what it \
         signs.",
        shown(sig)
    )
}

/// A signature that contains the signed text (clear-signed or inline): the
/// verdict, then exactly the text that was signed.
fn verify_inline(c: &Ctx, sig: &Path) -> Res {
    // A file next to it with the same name is NOT covered by this signature.
    let name = sig
        .file_name()
        .map(|n| n.as_encoded_bytes().to_vec())
        .unwrap_or_default();
    for ext in [&b".sig"[..], b".asc", b".gpg"] {
        if let Some(stem) = name.strip_suffix(ext) {
            if let Ok(stem) = std::str::from_utf8(stem) {
                if sig.with_file_name(stem).exists() {
                    return Err(Failure::Refused(not_detached(sig)));
                }
            }
        }
    }
    let (_, data, status, truncated) = capture(c, &[OsStr::new("--decrypt"), sig.as_os_str()])?;
    if truncated {
        return Err(Failure::Refused(
            "The signed text is too big to show here.".into(),
        ));
    }
    match openpgp::parse_verify(&status) {
        Verdict::Good {
            uid,
            fingerprint,
            warning,
        } => {
            let page = format!(
                "VALID SIGNATURE by {}\nFingerprint: {}{}\n\nWhat was signed is exactly the \
                 text below, and nothing else (check that the fingerprint is the one its \
                 author publishes). Press q to close.\n\n{}",
                printable(&uid),
                grouped_fingerprint(&fingerprint),
                warning
                    .map(|w| format!("\nWARNING: {w}."))
                    .unwrap_or_default(),
                String::from_utf8_lossy(&data)
            );
            show_text(c, &page)?;
            heading("VALID SIGNATURE");
            say(&format!(
                "Signed by: {}\nFingerprint: {}",
                printable(&uid),
                grouped_fingerprint(&fingerprint)
            ));
            Ok(())
        }
        Verdict::Bad { uid } => Err(Failure::Refused(format!(
            "\nINVALID SIGNATURE\n=================\nThe text does NOT match the signature \
             of {}: it was changed after signing. Do not trust it.",
            printable(&uid)
        ))),
        Verdict::UnknownKey { key_id } => Err(Failure::Refused(format!(
            "\nNOT CHECKED: this was signed with the OpenPGP key {}, which is not in your \
             keyring.",
            printable(&key_id)
        ))),
        Verdict::Error => Err(Failure::Error(format!(
            "The signature could not be checked.\n\n({})",
            printable(human_lines(&status).trim())
        ))),
    }
}

// ----- inspect -------------------------------------------------------------------

pub fn inspect(c: &Ctx, file: &Path) {
    // Only for encrypted messages; nothing is decrypted, and any content
    // goes nowhere (it must never mix with the status lines).
    let Ok((ok, status, _)) = run(gpg(c)
        .args(["--output", "/dev/null", "--list-only", "--decrypt"])
        .arg(file))
    else {
        return;
    };
    let (ids, _) = openpgp::parse_enc_to(&status);
    if !ok && ids.is_empty() {
        return;
    }
    if ids.is_empty() {
        return;
    }
    let keys = list(c);
    say("\nEncrypted for:");
    for id in ids {
        match keys.iter().find(|k| k.has_id(&id)) {
            Some(k) => say(&format!("  {} (key {})", who(k), printable(&id))),
            None => say(&format!("  key {}, not in your keyring", printable(&id))),
        }
    }
}

// ----- keys ----------------------------------------------------------------------

pub fn add_contact(c: &Ctx, file: &Path) -> Res {
    let file = &absolute(file)?;
    heading("ADD AN OPENPGP KEY");
    let text = read_small(file, 1024 * 1024)?;
    let (_, shown_keys, err) = run(gpg(c)
        .args(["--with-colons", "--fingerprint", "--show-keys"])
        .arg(file))?;
    let keys = openpgp::parse_keys(&shown_keys);
    if crypto::contains_secret_key(&text) || keys.iter().any(|k| k.secret) {
        return Err(Failure::Refused(
            "STOP: this file contains a PRIVATE key. Private keys must never be shared. \
             Nothing was added."
                .into(),
        ));
    }
    if keys.is_empty() {
        return Err(Failure::Refused(format!(
            "No OpenPGP public key found in {}.\n\n({})",
            shown(file),
            printable(err.trim())
        )));
    }
    say(&format!("Key file: {}\n", shown(file)));
    for k in &keys {
        say(&format!(
            "  {}\n  Fingerprint: {}\n  Created {}{}{}\n",
            printable(&k.uid),
            grouped_fingerprint(&k.fingerprint),
            date(k.created),
            k.expires
                .map(|e| format!(", expires {}", date(Some(e))))
                .unwrap_or_default(),
            if k.unusable {
                "\n  WARNING: this key is revoked or expired."
            } else {
                ""
            }
        ));
    }
    say(
        "Before trusting a key, compare its fingerprint with the one its owner gives you \
         in person, by phone, or publishes (for software: on its website or in its \
         documentation). A key received by message could have been replaced.\n",
    );
    if !yes("Add these keys to your keyring?") {
        return Err(Failure::Refused("Cancelled: nothing was added.".into()));
    }
    let (_, status, err) = run(gpg(c).arg("--import").arg(file))?;
    let imported = openpgp::parse_import(&status);
    if imported.is_empty() {
        return Err(Failure::Error(format!(
            "GnuPG did not import the key.\n\n({})",
            printable(err.trim())
        )));
    }
    heading("DONE");
    say(&format!(
        "{} key(s) added. You can now check their signatures and encrypt for them.",
        imported.len()
    ));
    Ok(())
}

fn date(t: Option<i64>) -> String {
    t.map(|t| {
        resilience_core::admin::rfc3339_utc(t)
            .chars()
            .take(10)
            .collect()
    })
    .unwrap_or_else(|| "?".into())
}

pub fn new_key(c: &Ctx) -> Res {
    heading("CREATE YOUR OPENPGP KEY (GnuPG)");
    if let Some(k) = list(c).into_iter().find(|k| k.secret && !k.unusable) {
        say(&format!(
            "You already have an OpenPGP key: {}\nFingerprint: {}",
            printable(&k.uid),
            grouped_fingerprint(&k.fingerprint)
        ));
        return Ok(());
    }
    say(
        "An OpenPGP key lets you exchange encrypted files and signatures with people who \
         use GnuPG or other OpenPGP programs (e-mail, software publishers). Your key \
         carries a name, and an e-mail address if you want, which others will see.\n\
         It will be protected by a passphrase: if you forget it, nothing encrypted for \
         this key can ever be opened again.\n",
    );
    let uid = loop {
        let Some(name) = ask("Your name (Enter alone cancels): ") else {
            return Err(Failure::Refused("Cancelled.".into()));
        };
        if name.trim().is_empty() {
            return Err(Failure::Refused("Cancelled.".into()));
        }
        let email = ask("Your e-mail address (optional, Enter to skip): ").unwrap_or_default();
        match openpgp::user_id(&name, &email) {
            Ok(u) => break u,
            Err(e) => say(e),
        }
    };
    say("\nGnuPG will now ask for the passphrase, twice.");
    let (ok, status, err) = run(gpg(c)
        .arg("--quick-generate-key")
        .arg("--")
        .arg(&uid)
        .args(["future-default", "default", "2y"]))?;
    let fpr = status
        .lines()
        .find_map(|l| l.strip_prefix("[GNUPG:] KEY_CREATED "))
        .and_then(|r| r.split(' ').nth(1))
        .filter(|f| openpgp::is_fingerprint(f));
    let Some(fpr) = fpr.filter(|_| ok) else {
        return Err(Failure::Error(format!(
            "The key was not created (cancelled, or the passphrases differ).\n\n({})",
            printable(err.trim())
        )));
    };
    heading("DONE");
    say(&format!(
        "Your OpenPGP key: {}\nFingerprint: {}\nIt expires in 2 years (it can be extended \
         with gpg).\n\nA revocation certificate was saved in \
         ~/.gnupg/openpgp-revocs.d/{fpr}.rev: if your key is ever stolen, it lets you \
         tell others to stop using it. Keep a copy on paper or on a stick.",
        printable(&uid),
        grouped_fingerprint(fpr),
    ));
    Ok(())
}

/// Save the user's own OpenPGP public keys into `dir`. `None` when they
/// have none.
pub fn export(c: &Ctx, dir: &Path, user: &str) -> Result<Option<PathBuf>, Failure> {
    let own: Vec<PgpKey> = list(c).into_iter().filter(|k| k.secret).collect();
    if own.is_empty() {
        return Ok(None);
    }
    let target = free_path(dir, OsStr::new(&format!("{user}-openpgp-public-key.asc")))
        .ok_or_else(|| Failure::Refused("Too many files with this name.".into()))?;
    let (partial, out) = Partial::create(&target, 0o644)?;
    let mut cmd = gpg(c);
    cmd.args(["--yes", "--armor", "--output"])
        .arg(&partial.path)
        .arg("--export");
    for k in &own {
        cmd.arg(&k.fingerprint);
    }
    let (ok, _, err) = run(&mut cmd)?;
    if !ok {
        return Err(Failure::Error(format!(
            "Exporting the OpenPGP key failed.\n\n({})",
            printable(err.trim())
        )));
    }
    partial.finish(&out, &target)?;
    for k in &own {
        say(&format!(
            "OpenPGP: {}\n  fingerprint {}",
            printable(&k.uid),
            grouped_fingerprint(&k.fingerprint)
        ));
    }
    Ok(Some(target))
}
