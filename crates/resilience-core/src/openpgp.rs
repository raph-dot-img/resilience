//! OpenPGP (GnuPG) interoperability (ADR 0015): parsers for GnuPG's
//! machine-readable outputs, `--status-fd` lines and `--with-colons` key
//! listings (both documented in GnuPG's doc/DETAILS). No cryptography here:
//! gpg does all of it.

/// Outcome of `gpg --status-fd 1 --verify` (or of a decryption that also
/// carried a signature).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Good signature. `fingerprint` is the primary key's (from VALIDSIG).
    Good {
        uid: String,
        fingerprint: String,
        /// The key or the signature expired, or the key was revoked.
        warning: Option<&'static str>,
    },
    /// The signature does not match the data.
    Bad { uid: String },
    /// Signed by a key that is not in the keyring.
    UnknownKey { key_id: String },
    /// No signature found, or an error gpg could not classify.
    Error,
}

/// Read the status lines of a verification. The first signature decides
/// (files with several signatures are rare; the others are ignored).
pub fn parse_verify(status: &str) -> Verdict {
    let mut uid = None;
    let mut fingerprint = None;
    let mut warning = None;
    let mut bad = None;
    let mut missing = None;
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("[GNUPG:] ") else {
            continue;
        };
        let mut f = rest.splitn(3, ' ');
        let (kw, a, b) = (
            f.next().unwrap_or(""),
            f.next().unwrap_or(""),
            f.next().unwrap_or(""),
        );
        match kw {
            "GOODSIG" => uid = Some(b.to_string()),
            "EXPSIG" | "EXPKEYSIG" => {
                uid = Some(b.to_string());
                warning = Some("the signature or the key has expired");
            }
            "REVKEYSIG" => {
                uid = Some(b.to_string());
                warning = Some("the key was REVOKED by its owner");
            }
            "VALIDSIG" => {
                // VALIDSIG <fpr> <date> ... <primary-fpr> (last field).
                let all: Vec<&str> = rest.split(' ').collect();
                let primary = all.last().filter(|p| is_fingerprint(p)).copied();
                fingerprint = primary.or(Some(a)).map(str::to_string);
            }
            "BADSIG" => bad = Some(b.to_string()),
            "NO_PUBKEY" => missing = Some(a.to_string()),
            _ => {}
        }
    }
    if let Some(uid) = bad {
        return Verdict::Bad { uid };
    }
    match (uid, fingerprint, missing) {
        (Some(uid), Some(fingerprint), _) => Verdict::Good {
            uid,
            fingerprint,
            warning,
        },
        (_, _, Some(key_id)) => Verdict::UnknownKey { key_id },
        _ => Verdict::Error,
    }
}

/// Status of a decryption: did gpg decrypt it, and was it protected
/// against changes (MDC/AEAD)?
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decryption {
    pub ok: bool,
    /// The message is protected against changes (no DECRYPTION_INFO with
    /// mdc 0, no "no integrity protection" failure).
    pub integrity: bool,
    pub signature: Option<Verdict>,
    pub no_secret_key: bool,
    pub bad_passphrase: bool,
}

pub fn parse_decrypt(status: &str) -> Decryption {
    let has = |kw: &str| {
        status.lines().any(|l| {
            l.strip_prefix("[GNUPG:] ")
                .is_some_and(|r| r.starts_with(kw))
        })
    };
    // DECRYPTION_INFO <mdc_method> <sym_algo> [<aead_algo>]: 0 0 = none.
    let protected = status.lines().any(|l| {
        l.strip_prefix("[GNUPG:] DECRYPTION_INFO ")
            .is_some_and(|r| {
                let f: Vec<&str> = r.split(' ').collect();
                f.first().is_some_and(|m| *m != "0") || f.get(2).is_some_and(|a| *a != "0")
            })
    });
    let signature = has("NEWSIG").then(|| parse_verify(status));
    Decryption {
        ok: has("DECRYPTION_OKAY") && !has("DECRYPTION_FAILED"),
        integrity: protected && !has("DECRYPTION_FAILED"),
        signature,
        no_secret_key: has("NO_SECKEY"),
        bad_passphrase: has("BAD_PASSPHRASE"),
    }
}

/// Keys imported by `gpg --status-fd 1 --import`: their fingerprints.
pub fn parse_import(status: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in status.lines() {
        if let Some(r) = l.strip_prefix("[GNUPG:] IMPORT_OK ")
            && let Some(fpr) = r.split(' ').nth(1)
            && is_fingerprint(fpr)
            && !out.iter().any(|f| f == fpr)
        {
            out.push(fpr.to_string());
        }
    }
    out
}

/// Recipients of an encrypted message, from `gpg --list-only --status-fd 1
/// --decrypt` (nothing is decrypted): the key ids it is encrypted to, and
/// whether a passphrase also opens it.
pub fn parse_enc_to(status: &str) -> (Vec<String>, bool) {
    let mut ids: Vec<String> = Vec::new();
    let mut passphrase = false;
    for l in status.lines() {
        let Some(r) = l.strip_prefix("[GNUPG:] ") else {
            continue;
        };
        if let Some(id) = r.strip_prefix("ENC_TO ").and_then(|x| x.split(' ').next()) {
            if !ids.iter().any(|i| i == id) {
                ids.push(id.to_string());
            }
        } else if r.starts_with("NEED_PASSPHRASE_SYM") || r.starts_with("SYMKEY_ENC") {
            passphrase = true;
        }
    }
    (ids, passphrase)
}

/// A key of the GnuPG keyring, from `--with-colons --fingerprint`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgpKey {
    pub fingerprint: String,
    /// First user id ("Name <e-mail>").
    pub uid: String,
    /// Creation and expiry, seconds since the epoch.
    pub created: Option<i64>,
    pub expires: Option<i64>,
    /// Revoked, expired or disabled: not usable to encrypt.
    pub unusable: bool,
    /// The secret key is in the keyring (one of the user's own keys).
    pub secret: bool,
    /// Can encrypt (has an encryption subkey or capability).
    pub can_encrypt: bool,
    /// Fingerprints of its subkeys (messages name the subkey they are
    /// encrypted to).
    pub subkeys: Vec<String>,
}

impl PgpKey {
    /// This key, or one of its subkeys, has the 16-digit key id `id`.
    pub fn has_id(&self, id: &str) -> bool {
        let id = id.to_ascii_uppercase();
        std::iter::once(&self.fingerprint)
            .chain(&self.subkeys)
            .any(|f| f.ends_with(&id))
    }
}

/// Parse a `gpg --with-colons --fingerprint --list-keys` (or
/// `--list-secret-keys`, `sec` records) listing.
pub fn parse_keys(colons: &str) -> Vec<PgpKey> {
    let mut keys: Vec<PgpKey> = Vec::new();
    let mut want_fpr = false;
    let mut want_sub = false;
    for line in colons.lines() {
        let f: Vec<&str> = line.split(':').collect();
        match f.first().copied() {
            Some(kind @ ("pub" | "sec")) => {
                let validity = f.get(1).copied().unwrap_or("");
                let caps = f.get(11).copied().unwrap_or("");
                keys.push(PgpKey {
                    fingerprint: String::new(),
                    uid: String::new(),
                    created: f.get(5).and_then(|s| s.parse().ok()),
                    expires: f.get(6).and_then(|s| s.parse().ok()),
                    unusable: matches!(validity, "r" | "e" | "d" | "i") || caps.contains('D'),
                    secret: kind == "sec",
                    can_encrypt: caps.contains('E'),
                    subkeys: Vec::new(),
                });
                want_fpr = true;
                want_sub = false;
            }
            Some("fpr") if want_fpr || want_sub => {
                if let (Some(k), Some(fpr)) = (keys.last_mut(), f.get(9)) {
                    if want_fpr {
                        k.fingerprint = fpr.to_string();
                    } else {
                        k.subkeys.push(fpr.to_string());
                    }
                }
                want_fpr = false;
                want_sub = false;
            }
            Some("uid") => {
                if let Some(k) = keys.last_mut()
                    && k.uid.is_empty()
                {
                    k.uid = unescape(f.get(9).copied().unwrap_or(""));
                }
            }
            Some("sub" | "ssb") => {
                want_fpr = false;
                want_sub = true;
            }
            _ => {}
        }
    }
    keys.retain(|k| is_fingerprint(&k.fingerprint));
    keys
}

/// `--with-colons` escapes `:` and other bytes as `\xHH`.
fn unescape(s: &str) -> String {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1] == b'x' {
            let hex = std::str::from_utf8(&b[i + 2..i + 4]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    crate::printable(&String::from_utf8_lossy(&out))
}

/// A v4 (40 hex) or v5/v6 (64 hex) fingerprint, upper case as gpg prints it.
pub fn is_fingerprint(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Fingerprint in groups of four for reading aloud.
pub fn grouped_fingerprint(fpr: &str) -> String {
    fpr.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A user id typed for a new key: "Name <e-mail>" built from two parts,
/// both checked (no `<>` in the name, a plausible e-mail or none).
pub fn user_id(name: &str, email: &str) -> Result<String, &'static str> {
    let name = name.trim();
    let email = email.trim();
    if name.chars().count() < 2 || name.chars().count() > 60 {
        return Err("The name must have 2 to 60 characters.");
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"'))
    {
        return Err("The name cannot contain < > : or quotes.");
    }
    // Never something gpg could read as an option.
    if !name.chars().next().is_some_and(char::is_alphanumeric) {
        return Err("The name must start with a letter or a digit.");
    }
    if email.is_empty() {
        return Ok(name.to_string());
    }
    let ok = email.len() <= 100
        && email.split_once('@').is_some_and(|(u, d)| {
            !u.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.')
        })
        && email
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@.-_+".contains(c));
    if !ok {
        return Err("This is not an e-mail address (leave it empty if you have none).");
    }
    Ok(format!("{name} <{email}>"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPR: &str = "BD9588ED90B75D0C9B97CDB5E9BD41B619251F40";

    // Real gpg 2.4.9 outputs (2026-11-15).
    const GOOD: &str = "[GNUPG:] NEWSIG
[GNUPG:] KEY_CONSIDERED BD9588ED90B75D0C9B97CDB5E9BD41B619251F40 0
[GNUPG:] SIG_ID nGEqcLenHwf9MgO27pYqci1vkjQ 2026-11-15 1794734011
[GNUPG:] GOODSIG E9BD41B619251F40 Test User <t@example.org>
[GNUPG:] VALIDSIG BD9588ED90B75D0C9B97CDB5E9BD41B619251F40 2026-11-15 1794734011 0 4 0 22 10 00 BD9588ED90B75D0C9B97CDB5E9BD41B619251F40
[GNUPG:] TRUST_ULTIMATE 0 pgp
";
    const BAD: &str = "[GNUPG:] NEWSIG
[GNUPG:] BADSIG E9BD41B619251F40 Test User <t@example.org>
[GNUPG:] FAILURE gpg-exit 33554433
";
    const UNKNOWN: &str = "[GNUPG:] NEWSIG
[GNUPG:] ERRSIG E9BD41B619251F40 22 10 00 1794734011 9 BD9588ED90B75D0C9B97CDB5E9BD41B619251F40
[GNUPG:] NO_PUBKEY E9BD41B619251F40
";
    const COLONS: &str = "tru::1:1794734011:1857806011:3:1:5
pub:u:255:22:E9BD41B619251F40:1794734011:1857806011::u:::scESC:::::ed25519:::0:
fpr:::::::::BD9588ED90B75D0C9B97CDB5E9BD41B619251F40:
uid:u::::1794734011::3EE03E13A486C0B6E628FDD3BA98727F0AAE75FC::Test User <t@example.org>::::::::::0:
sub:u:255:18:C3B7C7F0176BFEFA:1794734011::::::e:::::cv25519::
fpr:::::::::2BE0AD4644D8CA0B7638EA45C3B7C7F0176BFEFA:
pub:r:255:22:1111111111111111:1700000000:::-:::sc:::::ed25519:::0:
fpr:::::::::1111111111111111111111111111111111111111:
uid:r::::1700000000::AA::Old Key \\x3a revoked::::::::::0:
";

    #[test]
    fn verifications() {
        assert_eq!(
            parse_verify(GOOD),
            Verdict::Good {
                uid: "Test User <t@example.org>".into(),
                fingerprint: FPR.into(),
                warning: None
            }
        );
        assert_eq!(
            parse_verify(BAD),
            Verdict::Bad {
                uid: "Test User <t@example.org>".into()
            }
        );
        assert_eq!(
            parse_verify(UNKNOWN),
            Verdict::UnknownKey {
                key_id: "E9BD41B619251F40".into()
            }
        );
        assert_eq!(parse_verify("[GNUPG:] NODATA 1\n"), Verdict::Error);
        assert_eq!(parse_verify(""), Verdict::Error);
        let revoked = GOOD.replace("GOODSIG", "REVKEYSIG");
        assert!(
            matches!(parse_verify(&revoked), Verdict::Good { warning: Some(w), .. } if w.contains("REVOKED"))
        );
        // A GOODSIG without VALIDSIG is not trusted as good.
        assert_eq!(parse_verify("[GNUPG:] GOODSIG AB Name\n"), Verdict::Error);
        // Lines not from the status protocol are ignored.
        assert_eq!(
            parse_verify("gpg: [GNUPG:] GOODSIG fake\n[GNUPG:] BADSIG X Y\n"),
            Verdict::Bad { uid: "Y".into() }
        );
    }

    #[test]
    fn decryptions() {
        let ok = "[GNUPG:] DECRYPTION_INFO 2 9 0\n[GNUPG:] PLAINTEXT 62 0\n[GNUPG:] DECRYPTION_OKAY\n[GNUPG:] GOODMDC\n";
        let d = parse_decrypt(ok);
        assert!(d.ok && d.integrity && d.signature.is_none());
        let aead = "[GNUPG:] DECRYPTION_INFO 0 9 2\n[GNUPG:] DECRYPTION_OKAY\n";
        assert!(parse_decrypt(aead).integrity);
        let weak = "[GNUPG:] DECRYPTION_INFO 0 3\n[GNUPG:] DECRYPTION_OKAY\n";
        let d = parse_decrypt(weak);
        assert!(d.ok && !d.integrity);
        let nokey = "[GNUPG:] NO_SECKEY 1234\n[GNUPG:] DECRYPTION_FAILED\n";
        let d = parse_decrypt(nokey);
        assert!(!d.ok && d.no_secret_key);
        let pass = "[GNUPG:] BAD_PASSPHRASE 1234\n[GNUPG:] DECRYPTION_FAILED\n";
        assert!(parse_decrypt(pass).bad_passphrase);
        let signed = format!("[GNUPG:] DECRYPTION_INFO 2 9 0\n[GNUPG:] DECRYPTION_OKAY\n{GOOD}");
        assert!(matches!(
            parse_decrypt(&signed).signature,
            Some(Verdict::Good { .. })
        ));
    }

    #[test]
    fn imports_and_listings() {
        let st = "[GNUPG:] IMPORTED E9BD41B619251F40 Test User <t@example.org>
[GNUPG:] IMPORT_OK 1 BD9588ED90B75D0C9B97CDB5E9BD41B619251F40
[GNUPG:] IMPORT_OK 0 BD9588ED90B75D0C9B97CDB5E9BD41B619251F40
[GNUPG:] IMPORT_RES 1 0 1 0 0 0 0 0 0 0 0 0 0 0 0
";
        assert_eq!(parse_import(st), vec![FPR.to_string()]);
        let keys = parse_keys(COLONS);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].fingerprint, FPR);
        assert_eq!(keys[0].uid, "Test User <t@example.org>");
        assert_eq!(keys[0].created, Some(1794734011));
        assert_eq!(keys[0].expires, Some(1857806011));
        assert!(keys[0].can_encrypt && !keys[0].unusable && !keys[0].secret);
        assert!(keys[1].unusable && !keys[1].can_encrypt);
        assert_eq!(
            keys[0].subkeys,
            vec!["2BE0AD4644D8CA0B7638EA45C3B7C7F0176BFEFA".to_string()]
        );
        assert!(keys[0].has_id("C3B7C7F0176BFEFA") && keys[0].has_id("e9bd41b619251f40"));
        assert!(!keys[1].has_id("C3B7C7F0176BFEFA"));
        assert_eq!(keys[1].uid, "Old Key : revoked");
        // A bad escape followed by a multi-byte character does not panic.
        assert_eq!(unescape("a\\xé"), "a\\xé");
        let sec = COLONS.replace("pub:u", "sec:u");
        assert!(parse_keys(&sec)[0].secret);
        assert_eq!(
            grouped_fingerprint(FPR),
            "BD95 88ED 90B7 5D0C 9B97 CDB5 E9BD 41B6 1925 1F40"
        );
    }

    #[test]
    fn recipients_of_a_message() {
        let st = "[GNUPG:] ENC_TO C3B7C7F0176BFEFA 18 0\n[GNUPG:] ENC_TO C3B7C7F0176BFEFA 18 0\n[GNUPG:] ENC_TO 1122334455667788 1 0\n";
        assert_eq!(
            parse_enc_to(st),
            (
                vec![
                    "C3B7C7F0176BFEFA".to_string(),
                    "1122334455667788".to_string()
                ],
                false
            )
        );
        assert_eq!(
            parse_enc_to("[GNUPG:] NEED_PASSPHRASE_SYM 9 3 2\n"),
            (vec![], true)
        );
    }

    #[test]
    fn user_ids() {
        assert_eq!(
            user_id("Alice Martin", "alice@example.org"),
            Ok("Alice Martin <alice@example.org>".into())
        );
        assert_eq!(user_id("  Bob ", ""), Ok("Bob".into()));
        assert!(user_id("A", "").is_err());
        assert!(user_id("Eve <x>", "").is_err());
        assert!(user_id("--status-file=/tmp/x", "").is_err());
        assert!(user_id("-x", "").is_err());
        assert!(user_id("Eve", "not-an-address").is_err());
        assert!(user_id("Eve", "a@b").is_err());
        assert!(user_id("Eve", "a b@c.d").is_err());
    }
}
