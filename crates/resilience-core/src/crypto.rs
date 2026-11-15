//! Cryptography workflows (ADR 0015): where keys and contacts are kept,
//! recognising public keys and file formats, naming output files, and
//! comparing checksums.
//!
//! **There is no cryptography in this module.** Encryption, decryption,
//! signatures and hashes are done by age, minisign, GnuPG and coreutils; this
//! module only reads their text formats (the same way a file manager
//! recognises a file type), so that the interface can guide the user.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Keys and contacts, relative to the home folder. Private (0700).
pub const KEYS_DIR: &str = ".local/share/resilience/keys";
/// Decrypted files go here, never next to the encrypted file (which is often
/// on a stick), and never over an existing file.
pub const DECRYPTED_DIR: &str = "Decrypted";
/// Encrypted messages written with the interface.
pub const MESSAGES_DIR: &str = "Messages";
/// Largest message typed or read in the interface.
pub const MAX_MESSAGE: usize = 256 * 1024;
/// Largest key file read when adding a contact.
pub const MAX_KEY_FILE: u64 = 64 * 1024;

// ----- key store -----------------------------------------------------------

/// The user's own keys and their contacts' public keys.
///
/// ```text
/// ~/.local/share/resilience/keys/        0700
///   own/encryption.age    age identity, encrypted with a passphrase (age -p)
///   own/encryption.pub    its public key (age1...)
///   own/signing.key       minisign secret key, encrypted with a password
///   own/signing.pub       its public key (minisign format)
///   contacts/NAME.toml    age = "age1...", minisign = "RW...", added = "..."
/// ```
#[derive(Debug, Clone)]
pub struct KeyStore {
    pub dir: PathBuf,
}

/// The public halves of the user's own keys, when they exist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnKeys {
    pub encryption: Option<String>,
    pub signing: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    #[serde(skip)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minisign: Option<String>,
    /// When the contact was added (RFC 3339), for the user's information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
}

impl KeyStore {
    pub fn for_home(home: &Path) -> KeyStore {
        KeyStore {
            dir: home.join(KEYS_DIR),
        }
    }

    pub fn own_dir(&self) -> PathBuf {
        self.dir.join("own")
    }
    pub fn encryption_key(&self) -> PathBuf {
        self.own_dir().join("encryption.age")
    }
    pub fn encryption_pub(&self) -> PathBuf {
        self.own_dir().join("encryption.pub")
    }
    pub fn signing_key(&self) -> PathBuf {
        self.own_dir().join("signing.key")
    }
    pub fn signing_pub(&self) -> PathBuf {
        self.own_dir().join("signing.pub")
    }
    pub fn contacts_dir(&self) -> PathBuf {
        self.dir.join("contacts")
    }
    pub fn contact_path(&self, name: &str) -> PathBuf {
        self.contacts_dir().join(format!("{name}.toml"))
    }

    /// Create the folders (0700 each), and make existing ones private again.
    pub fn ensure_dirs(&self) -> io::Result<()> {
        for d in [self.dir.clone(), self.own_dir(), self.contacts_dir()] {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&d)?;
            fs::set_permissions(&d, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// The user's own public keys. A secret key whose public file is missing
    /// or damaged counts as absent for encrypting/verifying, but its file is
    /// still there (see [`KeyStore::has_encryption_key`]).
    pub fn own(&self) -> OwnKeys {
        let read = |p: PathBuf| fs::read_to_string(p).ok();
        OwnKeys {
            encryption: read(self.encryption_pub())
                .and_then(|t| find_public_keys(&t).age.into_iter().next()),
            signing: read(self.signing_pub())
                .and_then(|t| find_public_keys(&t).minisign.into_iter().next()),
        }
    }

    pub fn has_encryption_key(&self) -> bool {
        self.encryption_key().exists()
    }
    pub fn has_signing_key(&self) -> bool {
        self.signing_key().exists()
    }

    /// Every readable contact, sorted by name. Damaged files are skipped.
    pub fn contacts(&self) -> Vec<Contact> {
        let Ok(rd) = fs::read_dir(self.contacts_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<Contact> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.strip_suffix(".toml")?.to_string();
                if !valid_contact_name(&name) {
                    return None;
                }
                let text = fs::read_to_string(e.path()).ok()?;
                let mut c: Contact = toml::from_str(&text).ok()?;
                c.name = name;
                // Keep only keys that still look valid.
                c.age = c.age.filter(|k| is_age_recipient(k));
                c.minisign = c.minisign.filter(|k| minisign_key_id(k).is_some());
                Some(c)
            })
            .collect();
        out.sort_by_key(|c| c.name.to_lowercase());
        out
    }

    pub fn contact(&self, name: &str) -> Option<Contact> {
        self.contacts().into_iter().find(|c| c.name == name)
    }

    /// Write a contact (replacing its file atomically).
    pub fn save_contact(&self, c: &Contact) -> io::Result<()> {
        if !valid_contact_name(&c.name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid contact name",
            ));
        }
        self.ensure_dirs()?;
        let text = toml::to_string(c).map_err(io::Error::other)?;
        let path = self.contact_path(&c.name);
        let tmp = self
            .contacts_dir()
            .join(format!(".{}.toml.partial", c.name));
        let _ = fs::remove_file(&tmp);
        let mut f = create_new(&tmp, 0o600)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, &path)
    }

    pub fn remove_contact(&self, name: &str) -> io::Result<()> {
        if !valid_contact_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid contact name",
            ));
        }
        fs::remove_file(self.contact_path(name))
    }

    /// Who signed with `key_id`: "you", a contact's name, or nobody known.
    pub fn signer(&self, key_id: &str) -> Option<Signer> {
        if self
            .own()
            .signing
            .as_deref()
            .and_then(minisign_key_id)
            .is_some_and(|id| id == key_id)
        {
            return self.own().signing.map(|k| Signer {
                name: None,
                public_key: k,
            });
        }
        self.contacts().into_iter().find_map(|c| {
            let k = c.minisign?;
            (minisign_key_id(&k).as_deref() == Some(key_id)).then_some(Signer {
                name: Some(c.name),
                public_key: k,
            })
        })
    }
}

/// The owner of a signing key: `name` is `None` for the user's own key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub name: Option<String>,
    pub public_key: String,
}

/// Contact names are also file names: 1 to 40 letters, digits, spaces and
/// `-_.'`, starting with a letter or digit, not ending with a space.
pub fn valid_contact_name(s: &str) -> bool {
    let n = s.chars().count();
    (1..=40).contains(&n)
        && s.chars().next().is_some_and(char::is_alphanumeric)
        && !s.ends_with(' ')
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '\''))
}

// ----- public keys ---------------------------------------------------------

const BECH32: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

/// Looks like an age public key (`age1` + Bech32). The checksum is checked
/// by age itself when the key is first used (`age -r KEY` on empty input).
pub fn is_age_recipient(s: &str) -> bool {
    s.len() >= 59
        && s.len() <= 4096
        && s.strip_prefix("age1")
            .is_some_and(|rest| rest.chars().all(|c| BECH32.contains(c)))
}

/// Public keys found in a text (an exported key file, a minisign `.pub`
/// file, age-keygen output, or a key typed by hand). `secret` is set if the
/// text holds a private key: such a file must never be shared or imported.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FoundKeys {
    pub age: Vec<String>,
    pub minisign: Vec<String>,
    pub secret: bool,
}

pub fn find_public_keys(text: &str) -> FoundKeys {
    let mut found = FoundKeys {
        secret: contains_secret_key(text),
        ..FoundKeys::default()
    };
    let tokens = text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';'));
    for t in tokens {
        // "age:age1..." or "minisign=RW..." forms.
        let t = t.rsplit([':', '=']).next().unwrap_or(t);
        if is_age_recipient(t) && !found.age.iter().any(|k| k == t) {
            found.age.push(t.to_string());
        } else if minisign_key_id(t).is_some() && !found.minisign.iter().any(|k| k == t) {
            found.minisign.push(t.to_string());
        }
    }
    found
}

/// The text contains a private key of a kind we know.
pub fn contains_secret_key(text: &str) -> bool {
    text.contains("AGE-SECRET-KEY-")
        || text.contains("minisign encrypted secret key")
        || text.contains("PRIVATE KEY-----")
        || text.contains("BEGIN PGP PRIVATE KEY BLOCK")
}

/// Standard base64 (RFC 4648), padding optional. Only used to read the key
/// numbers in minisign keys and signatures, and the header of an armored
/// age file.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // Leftover bits must be zero padding (a 1-character tail is invalid).
    (bits < 6 && acc == 0).then_some(out)
}

/// The key number of a minisign public key (as minisign prints it, e.g.
/// `19E7B977E1687E00`), or `None` if `b64` is not a minisign public key.
pub fn minisign_key_id(b64: &str) -> Option<String> {
    if b64.len() != 56 {
        return None;
    }
    let raw = base64_decode(b64)?;
    (raw.len() == 42 && &raw[..2] == b"Ed").then(|| key_id_hex(&raw[2..10]))
}

fn key_id_hex(bytes: &[u8]) -> String {
    bytes.iter().rev().map(|b| format!("{b:02X}")).collect()
}

/// The key number in a minisign signature file.
pub fn minisign_signature_key_id(sig: &str) -> Option<String> {
    let line = sig
        .lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with("untrusted comment:"))?;
    let raw = base64_decode(line.trim())?;
    (raw.len() == 74 && (&raw[..2] == b"Ed" || &raw[..2] == b"ED")).then(|| key_id_hex(&raw[2..10]))
}

/// The trusted comment of a minisign signature (signed with the file).
pub fn minisign_trusted_comment(sig: &str) -> Option<String> {
    sig.lines()
        .find_map(|l| l.strip_prefix("trusted comment: "))
        .map(str::to_string)
}

/// minisign's default trusted comment (`timestamp:N<TAB>file:NAME<TAB>hashed`)
/// in words; any other comment is returned as it is, tabs as spaces.
pub fn describe_trusted_comment(comment: &str) -> String {
    let mut when = None;
    let mut file = None;
    for part in comment.split('\t') {
        if let Some(t) = part.strip_prefix("timestamp:") {
            when = t.parse::<i64>().ok().map(crate::admin::rfc3339_utc);
        } else if let Some(f) = part.strip_prefix("file:") {
            file = Some(f.to_string());
        }
    }
    match (when, file) {
        (Some(w), Some(f)) => format!(
            "signed on {} UTC, file name \"{f}\"",
            w.replace('T', " ").trim_end_matches('Z')
        ),
        _ => comment.replace('\t', "  "),
    }
}

/// A short form of an age public key for reading aloud: the last 8
/// characters, in two groups.
pub fn age_short(recipient: &str) -> String {
    let tail: String = recipient
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(
        "...{} {}",
        &tail[..4.min(tail.len())],
        &tail[4.min(tail.len())..]
    )
}

/// The text of an exported public key file.
pub fn export_text(owner: &str, own: &OwnKeys, date: &str) -> String {
    let mut s = format!(
        "# Public keys of {owner}, exported from Resilience on {date}.\n\
         # Give this file to people who will send you encrypted files or check\n\
         # your signatures. It contains NO secret. Before trusting it, they\n\
         # should check the keys with you in person or by phone.\n"
    );
    if let Some(k) = &own.encryption {
        s.push_str(&format!(
            "#\n# Encryption key (age), ends with {}\nage: {k}\n",
            age_short(k)
        ));
    }
    if let Some(k) = &own.signing {
        let id = minisign_key_id(k).unwrap_or_default();
        s.push_str(&format!(
            "#\n# Signing key (minisign), key number {id}\nminisign: {k}\n"
        ));
    }
    s
}

// ----- file formats --------------------------------------------------------

/// What an encrypted, signed or key file is, recognised from its first bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Format {
    /// An age encrypted file; `stanzas` are its recipient types
    /// (`X25519`, `scrypt` for a passphrase, ...), when readable.
    Age {
        armored: bool,
        stanzas: Vec<String>,
    },
    AgeSecretKey,
    MinisignSignature,
    MinisignPublicKey,
    MinisignSecretKey,
    /// OpenPGP (GnuPG). `what`: message, signature, public key, ...
    OpenPgp {
        armored: bool,
        what: &'static str,
    },
    SshPrivateKey,
    SshPublicKey,
    /// `openssl enc` with a password ("Salted__").
    OpenSslEnc,
    Luks,
    Zip {
        encrypted: bool,
    },
    SevenZip,
    /// Another PEM block (certificate, key, ...): its label.
    Pem(String),
    ChecksumList,
    Text,
    Unknown,
}

impl Format {
    pub fn is_age(&self) -> bool {
        matches!(self, Format::Age { .. })
    }

    /// True when decrypting needs the passphrase rather than a key.
    pub fn age_passphrase(&self) -> bool {
        matches!(self, Format::Age { stanzas, .. } if stanzas.iter().any(|s| s == "scrypt"))
    }
}

const AGE_MAGIC: &[u8] = b"age-encryption.org/v1\n";
const AGE_ARMOR: &str = "-----BEGIN AGE ENCRYPTED FILE-----";

/// Recognise the format from the start of a file (a few KiB are enough).
pub fn detect(head: &[u8]) -> Format {
    if head.starts_with(AGE_MAGIC) {
        return Format::Age {
            armored: false,
            stanzas: age_stanzas(head),
        };
    }
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start();
    if trimmed.starts_with(AGE_ARMOR) {
        let body: String = trimmed
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("-----END"))
            .map(str::trim)
            .collect();
        // Decode whole 4-character groups only: the head may be cut.
        let cut = body.len() - body.len() % 4;
        let stanzas = base64_decode(&body[..cut])
            .map(|raw| age_stanzas(&raw))
            .unwrap_or_default();
        return Format::Age {
            armored: true,
            stanzas,
        };
    }
    if text.contains("AGE-SECRET-KEY-1") {
        return Format::AgeSecretKey;
    }
    if trimmed.starts_with("untrusted comment:") {
        let first = trimmed.lines().next().unwrap_or("");
        return if first.contains("signature") {
            Format::MinisignSignature
        } else if first.contains("secret key") {
            Format::MinisignSecretKey
        } else if first.contains("public key") {
            Format::MinisignPublicKey
        } else {
            Format::MinisignSignature
        };
    }
    if let Some(label) = trimmed
        .strip_prefix("-----BEGIN ")
        .and_then(|r| r.split_once("-----"))
        .map(|(l, _)| l.to_string())
    {
        return match label.as_str() {
            "PGP MESSAGE" => Format::OpenPgp {
                armored: true,
                what: "encrypted or signed message",
            },
            "PGP SIGNED MESSAGE" => Format::OpenPgp {
                armored: true,
                what: "text with a signature",
            },
            "PGP SIGNATURE" => Format::OpenPgp {
                armored: true,
                what: "signature",
            },
            "PGP PUBLIC KEY BLOCK" => Format::OpenPgp {
                armored: true,
                what: "public key",
            },
            "PGP PRIVATE KEY BLOCK" => Format::OpenPgp {
                armored: true,
                what: "PRIVATE key",
            },
            "OPENSSH PRIVATE KEY" => Format::SshPrivateKey,
            _ => Format::Pem(label),
        };
    }
    if trimmed.starts_with("ssh-ed25519 ")
        || trimmed.starts_with("ssh-rsa ")
        || trimmed.starts_with("ecdsa-sha2-")
    {
        return Format::SshPublicKey;
    }
    if head.starts_with(b"Salted__") {
        return Format::OpenSslEnc;
    }
    if head.starts_with(b"LUKS\xba\xbe") {
        return Format::Luks;
    }
    if head.starts_with(b"PK\x03\x04") && head.len() >= 8 {
        return Format::Zip {
            encrypted: head[6] & 1 == 1,
        };
    }
    if head.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Format::SevenZip;
    }
    if let Some(what) = openpgp_packet(head) {
        return Format::OpenPgp {
            armored: false,
            what,
        };
    }
    if std::str::from_utf8(head).is_ok() || text.chars().all(|c| c != '\u{fffd}') {
        if checksum_list_algorithm(&text).is_some() {
            return Format::ChecksumList;
        }
        if !head.contains(&0) {
            return Format::Text;
        }
    }
    Format::Unknown
}

/// Recipient types in an age header (`-> X25519 ...` lines).
fn age_stanzas(raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw);
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().skip(1) {
        if line.starts_with("---") {
            break;
        }
        if let Some(kind) = line.strip_prefix("-> ").and_then(|r| r.split(' ').next()) {
            if !kind.is_empty() && !out.iter().any(|k| k == kind) {
                out.push(kind.to_string());
            }
        }
    }
    out
}

/// A binary OpenPGP file, from the tag of its first packet. Only the first
/// byte is looked at, so this is a strong hint, not a proof.
fn openpgp_packet(head: &[u8]) -> Option<&'static str> {
    let b = *head.first()?;
    if b & 0x80 == 0 || head.len() < 3 {
        return None;
    }
    let tag = if b & 0x40 != 0 {
        b & 0x3f
    } else {
        (b >> 2) & 0x0f
    };
    match tag {
        1 => Some("message encrypted to a key"),
        3 => Some("message encrypted with a passphrase"),
        2 => Some("signature"),
        4 => Some("signed file"),
        5 => Some("PRIVATE key"),
        6 => Some("public key"),
        8 => Some("compressed data (often a signed file)"),
        _ => None,
    }
}

/// A beginner's explanation of a format.
pub fn describe(f: &Format) -> (String, String) {
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    match f {
        Format::Age { armored, stanzas } => {
            let how = if stanzas.iter().any(|s| s == "scrypt") {
                "It opens with a passphrase.".to_string()
            } else if stanzas.is_empty() {
                "Its recipients could not be read.".to_string()
            } else {
                let pq = stanzas
                    .iter()
                    .any(|s| s.contains("pq") || s.contains("mlkem"));
                format!(
                    "It opens with the private key of one of its recipients ({} key{}).",
                    stanzas.join(", "),
                    if pq { ", post-quantum" } else { "" }
                )
            };
            (
                format!(
                    "An age encrypted {}",
                    if *armored {
                        "message (text form)"
                    } else {
                        "file"
                    }
                ),
                format!(
                    "age is a modern, simple encryption format (age-encryption.org). It \
                     protects the content and detects any change to it: a damaged or altered \
                     file is refused, not opened. {how} Use \"Decrypt a file\" or \"Read an \
                     encrypted message\"."
                ),
            )
        }
        Format::AgeSecretKey => s(
            "An age PRIVATE key, NOT encrypted",
            "Anyone with this file can read what is encrypted for this key. Never share it. \
             Keys made by Resilience are always stored encrypted with a passphrase.",
        ),
        Format::MinisignSignature => s(
            "A minisign signature",
            "Proves that a file was signed by a key and not changed since. Use \"Check a \
             signature\" on the signed file.",
        ),
        Format::MinisignPublicKey => s(
            "A minisign public key",
            "Lets you check signatures made by its owner. Add it to your contacts after \
             checking with the owner that it is theirs.",
        ),
        Format::MinisignSecretKey => s(
            "A minisign PRIVATE (signing) key",
            "Anyone with this file and its password can sign in its owner's name. Never share it.",
        ),
        Format::OpenPgp { armored, what } => (
            format!(
                "OpenPGP ({}): {what}",
                if *armored { "text form" } else { "binary" }
            ),
            "OpenPGP is the format of GnuPG (gpg), widely used for e-mail and for signing \
             software. Its keys are managed in GnuPG, not in Resilience's contacts."
                .to_string(),
        ),
        Format::SshPrivateKey => s(
            "An SSH PRIVATE key",
            "Used to log in to other computers. Never share it.",
        ),
        Format::SshPublicKey => s(
            "An SSH public key",
            "Used to allow a login on another computer. It contains no secret.",
        ),
        Format::OpenSslEnc => s(
            "Encrypted with \"openssl enc\" and a password",
            "This older format does not detect changes to the encrypted data, and the \
             settings used (cipher, key derivation) are not stored in the file: they must \
             be known to decrypt it. Prefer age for new files.",
        ),
        Format::Luks => s(
            "An encrypted disk (LUKS)",
            "A disk or partition image encrypted with LUKS (cryptsetup), the standard on \
             Linux. Opening it needs its passphrase and an administrator.",
        ),
        Format::Zip { encrypted } => {
            if *encrypted {
                s(
                    "A password-protected ZIP archive",
                    "ZIP encryption is often the old \"ZipCrypto\" method, which is weak. Do \
                     not rely on it for secrets.",
                )
            } else {
                s(
                    "A ZIP archive (not encrypted)",
                    "Anyone can read its content.",
                )
            }
        }
        Format::SevenZip => s(
            "A 7-Zip archive",
            "It may or may not be encrypted (7-Zip uses AES-256 when a password is set).",
        ),
        Format::Pem(label) => (
            format!("A PEM block: {label}"),
            if label.contains("PRIVATE") {
                "This is a PRIVATE key: never share it.".to_string()
            } else {
                "A certificate, key or similar object in text form.".to_string()
            },
        ),
        Format::ChecksumList => s(
            "A list of checksums",
            "Each line gives the checksum of a file. Use \"Check a list of checksums\" to \
             check the files it names.",
        ),
        Format::Text => s(
            "Plain text, not encrypted",
            "Anyone who can read this file can read its content.",
        ),
        Format::Unknown => s(
            "Not recognised",
            "This is not a format Resilience recognises. It may be ordinary data (a picture, \
             a program...) or something encrypted with a tool not listed here. Random-looking \
             data alone does not prove that a file is encrypted securely.",
        ),
    }
}

// ----- output files --------------------------------------------------------

/// `report.pdf` → `report.pdf.age`.
pub fn encrypted_name(name: &OsStr) -> OsString {
    let mut n = name.to_os_string();
    n.push(".age");
    n
}

/// `report.pdf.age` → `report.pdf`; other names get `.decrypted`.
pub fn decrypted_name(name: &OsStr) -> OsString {
    let b = name.as_bytes();
    for ext in [&b".age"[..], b".asc", b".gpg", b".pgp"] {
        if b.len() > ext.len() && b.ends_with(ext) {
            return OsString::from_vec(b[..b.len() - ext.len()].to_vec());
        }
    }
    let mut n = name.to_os_string();
    n.push(".decrypted");
    n
}

/// `name` in `dir`, or `name (2)`, `name (3)`... before the last extension,
/// the first that does not exist. `None` after 999 tries.
pub fn free_path(dir: &Path, name: &OsStr) -> Option<PathBuf> {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return Some(first);
    }
    let b = name.as_bytes();
    let (stem, ext) = match b.iter().rposition(|&c| c == b'.') {
        Some(i) if i > 0 => (&b[..i], &b[i..]),
        _ => (b, &b""[..]),
    };
    (2..1000).find_map(|n| {
        let mut v = stem.to_vec();
        v.extend_from_slice(format!(" ({n})").as_bytes());
        v.extend_from_slice(ext);
        let p = dir.join(OsStr::from_bytes(&v));
        fs::symlink_metadata(&p).is_err().then_some(p)
    })
}

/// Create a new file: fails if anything exists at `path`, never follows a
/// symbolic link.
pub fn create_new(path: &Path, mode: u32) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

/// Rename without replacing an existing file. File systems that do not
/// support `RENAME_NOREPLACE` fall back to a check followed by a rename.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    let c = |p: &Path| {
        CString::new(p.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))
    };
    let (f, t) = (c(from)?, c(to)?);
    // SAFETY: both are valid NUL-terminated paths; AT_FDCWD resolves them
    // relative to the current directory like rename(2).
    let r = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            f.as_ptr(),
            libc::AT_FDCWD,
            t.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EINVAL) || e.raw_os_error() == Some(libc::ENOSYS) {
        if fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        return fs::rename(from, to);
    }
    Err(e)
}

// ----- checksums -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algo {
    Md5,
    Sha1,
    Sha256,
    Sha512,
    Blake2b,
}

impl Algo {
    pub fn name(self) -> &'static str {
        match self {
            Algo::Md5 => "MD5",
            Algo::Sha1 => "SHA-1",
            Algo::Sha256 => "SHA-256",
            Algo::Sha512 => "SHA-512",
            Algo::Blake2b => "BLAKE2b-512",
        }
    }

    /// Broken algorithms: a match does not prove the file was not altered on
    /// purpose, only that it was not damaged by accident.
    pub fn weak(self) -> bool {
        matches!(self, Algo::Md5 | Algo::Sha1)
    }

    /// The coreutils program computing it.
    pub fn program(self) -> &'static str {
        match self {
            Algo::Md5 => "md5sum",
            Algo::Sha1 => "sha1sum",
            Algo::Sha256 => "sha256sum",
            Algo::Sha512 => "sha512sum",
            Algo::Blake2b => "b2sum",
        }
    }

    /// Algorithms whose hex form has `len` digits (SHA-512 and BLAKE2b share 128).
    pub fn for_hex_len(len: usize) -> &'static [Algo] {
        match len {
            32 => &[Algo::Md5],
            40 => &[Algo::Sha1],
            64 => &[Algo::Sha256],
            128 => &[Algo::Sha512, Algo::Blake2b],
            _ => &[],
        }
    }

    fn from_tag(tag: &str) -> Option<Algo> {
        match tag.to_ascii_uppercase().as_str() {
            "MD5" => Some(Algo::Md5),
            "SHA1" => Some(Algo::Sha1),
            "SHA256" => Some(Algo::Sha256),
            "SHA512" => Some(Algo::Sha512),
            "BLAKE2B" => Some(Algo::Blake2b),
            _ => None,
        }
    }
}

/// An expected checksum typed or pasted by the user: the hexadecimal part,
/// in lower case, and the algorithms it may be. Accepts `HEX`, `HEX  file`,
/// `sha256:HEX`, `SHA256 (file) = HEX`, with spaces inside the hex.
pub fn parse_expected(input: &str) -> Result<(String, &'static [Algo]), String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Type the checksum you were given.".into());
    }
    // Hex tokens of a known length, longest first; spaces between groups
    // ("ab12 cd34 ...") are joined.
    let joined: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let candidates = [
        input
            .rsplit(['=', ':'])
            .next()
            .unwrap_or(input)
            .trim()
            .to_string(),
        input.split_whitespace().next().unwrap_or("").to_string(),
        joined.rsplit(['=', ':']).next().unwrap_or("").to_string(),
    ];
    for c in candidates {
        let c: String = c.chars().filter(|c| !c.is_whitespace()).collect();
        if c.chars().all(|ch| ch.is_ascii_hexdigit()) {
            let algos = Algo::for_hex_len(c.len());
            if !algos.is_empty() {
                return Ok((c.to_ascii_lowercase(), algos));
            }
        }
    }
    Err(
        "This is not a checksum: expected 32, 40, 64 or 128 hexadecimal characters \
         (0-9, a-f)."
            .into(),
    )
}

/// The algorithm of a checksum list (`sha256sum` output, `SHA256SUMS`, or
/// BSD style `SHA256 (file) = hex`), from its first entry.
pub fn checksum_list_algorithm(text: &str) -> Option<Algo> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))?;
    if let Some((tag, rest)) = line.split_once(" (") {
        let algo = Algo::from_tag(tag)?;
        let hex = rest.rsplit_once(") = ")?.1;
        return (hex.chars().all(|c| c.is_ascii_hexdigit())
            && Algo::for_hex_len(hex.len()).contains(&algo))
        .then_some(algo);
    }
    let (hex, name) = line.split_once(char::is_whitespace)?;
    let name = name.trim_start().trim_start_matches('*');
    if name.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Algo::for_hex_len(hex.len()).first().copied()
}

/// The hex digest at the start of a `sha256sum`-style output line.
pub fn digest_of_output(out: &str) -> Option<String> {
    let first = out.split_whitespace().next()?;
    // `\` prefix when the file name has special characters.
    let hex = first.trim_start_matches('\\');
    (hex.len() >= 32 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

/// Group a hex digest in blocks of 8 for reading and comparing by eye.
pub fn grouped(hex: &str) -> String {
    hex.as_bytes()
        .chunks(8)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

// ----- random --------------------------------------------------------------

/// From `xkcdpass -V` output: the passphrase (last line) and the entropy it
/// reports ("roughly N bits").
pub fn parse_xkcdpass(out: &str) -> Option<(String, Option<u32>)> {
    let phrase = out.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let bits = out
        .split("roughly ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok());
    (!phrase.starts_with("With the current options") && !phrase.contains("bits of entropy"))
        .then(|| (phrase.to_string(), bits))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGE_PUB: &str = "age1fnc4atdyhfzcxgdtr7zmcgx6mgwzd4x4gehhge7myxhn4kjdy9ps7jmt65";
    const MS_PUB: &str = "RWQAfmjhd7nnGUBTzemb8G8wF9ElertEKU4C1kTZoS2w7SMHLeK0wIfG";
    const MS_SIG: &str = "untrusted comment: signature from minisign secret key
RUQAfmjhd7nnGYhDxiGHeCFq/5lcn5Z1iwCdYzYxvyxVtGmnx7sYFTJT6KosmiDGdFv50crMIdJym31dWq89bHHCsSquA3hz0wU=
trusted comment: signed by alice
dfqiX2FQVLiTMfb7hTcmQ9B9KiBhyTyLsZVhh5CFMuCWDcmMyollHa70tF+BK45z8CHz+6UegRXiR4e7xr7wCw==
";

    fn tmp(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("resilience-crypto-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64_decode("").as_deref(), Some(&b""[..]));
        assert_eq!(base64_decode("Zg==").as_deref(), Some(&b"f"[..]));
        assert_eq!(base64_decode("Zm8").as_deref(), Some(&b"fo"[..]));
        assert_eq!(base64_decode("Zm9vYmFy").as_deref(), Some(&b"foobar"[..]));
        assert_eq!(base64_decode("Zm9v!"), None);
        assert_eq!(base64_decode("Z"), None);
    }

    #[test]
    fn minisign_key_numbers() {
        // As printed by minisign 0.12 for this key.
        assert_eq!(minisign_key_id(MS_PUB).as_deref(), Some("19E7B977E1687E00"));
        assert_eq!(
            minisign_signature_key_id(MS_SIG).as_deref(),
            Some("19E7B977E1687E00")
        );
        assert_eq!(
            minisign_trusted_comment(MS_SIG).as_deref(),
            Some("signed by alice")
        );
        assert_eq!(minisign_key_id(&MS_PUB[1..]), None);
        assert_eq!(minisign_key_id(AGE_PUB), None);
        assert_eq!(
            minisign_signature_key_id("untrusted comment: x\nnope\n"),
            None
        );
    }

    #[test]
    fn finds_public_keys_in_many_forms() {
        let exported = export_text(
            "alice",
            &OwnKeys {
                encryption: Some(AGE_PUB.into()),
                signing: Some(MS_PUB.into()),
            },
            "2026-11-15",
        );
        let f = find_public_keys(&exported);
        assert_eq!(f.age, vec![AGE_PUB.to_string()]);
        assert_eq!(f.minisign, vec![MS_PUB.to_string()]);
        assert!(!f.secret);
        assert!(exported.contains("19E7B977E1687E00"));
        // A signature's comment mentions the secret key: not a secret.
        assert!(!find_public_keys(MS_SIG).secret);
        assert!(find_public_keys("untrusted comment: minisign encrypted secret key\nRWS").secret);
        let pubfile =
            format!("untrusted comment: minisign public key 19E7B977E1687E00\n{MS_PUB}\n");
        assert_eq!(find_public_keys(&pubfile).minisign.len(), 1);
        let typed = format!("  {AGE_PUB}  ");
        assert_eq!(find_public_keys(&typed).age.len(), 1);
        let keygen = format!("# public key: {AGE_PUB}\nAGE-SECRET-KEY-1ABC\n");
        let f = find_public_keys(&keygen);
        assert!(f.secret);
        assert_eq!(f.age.len(), 1);
        assert!(!is_age_recipient("age1short"));
        assert!(!is_age_recipient(&AGE_PUB.to_uppercase()));
        assert!(!is_age_recipient(
            &format!("{AGE_PUB}b")[..].replace('q', "B")
        ));
    }

    #[test]
    fn contact_names() {
        for ok in ["Alice", "Hélène", "Bob Smith", "o'neil", "n-2_x.y"] {
            assert!(valid_contact_name(ok), "{ok}");
        }
        for bad in [
            "",
            " a",
            "a ",
            ".hidden",
            "a/b",
            "../x",
            "a\nb",
            &"x".repeat(41),
        ] {
            assert!(!valid_contact_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn key_store_round_trip() {
        let home = tmp("store");
        let ks = KeyStore::for_home(&home);
        assert_eq!(ks.own(), OwnKeys::default());
        assert!(ks.contacts().is_empty());
        let c = Contact {
            name: "Alice".into(),
            age: Some(AGE_PUB.into()),
            minisign: Some(MS_PUB.into()),
            added: Some("2026-11-15T10:00:00Z".into()),
        };
        ks.save_contact(&c).expect("save");
        let mode = fs::metadata(&ks.dir).expect("dir").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(ks.contacts(), vec![c.clone()]);
        let signer = ks.signer("19E7B977E1687E00").expect("known");
        assert_eq!(signer.name.as_deref(), Some("Alice"));
        assert_eq!(ks.signer("0000000000000000"), None);
        // A damaged file and a bad name are skipped.
        fs::write(ks.contacts_dir().join("Bad.toml"), "age = 3").expect("write");
        fs::write(ks.contacts_dir().join(".x.toml"), "").expect("write");
        assert_eq!(ks.contacts().len(), 1);
        assert!(
            ks.save_contact(&Contact {
                name: "../x".into(),
                ..c.clone()
            })
            .is_err()
        );
        // Own keys.
        fs::write(ks.encryption_pub(), format!("{AGE_PUB}\n")).expect("write");
        fs::write(
            ks.signing_pub(),
            format!("untrusted comment: minisign public key\n{MS_PUB}\n"),
        )
        .expect("write");
        let own = ks.own();
        assert_eq!(own.encryption.as_deref(), Some(AGE_PUB));
        assert_eq!(own.signing.as_deref(), Some(MS_PUB));
        assert_eq!(ks.signer("19E7B977E1687E00").expect("own").name, None);
        ks.remove_contact("Alice").expect("remove");
        assert!(ks.contact("Alice").is_none());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn detects_formats() {
        let age =
            b"age-encryption.org/v1\n-> X25519 abc\nxyz\n-> scrypt salt 18\nq\n--- mac\n\x00\x01";
        assert_eq!(
            detect(age),
            Format::Age {
                armored: false,
                stanzas: vec!["X25519".into(), "scrypt".into()]
            }
        );
        assert!(detect(age).age_passphrase());
        // Armored: header base64 of "age-encryption.org/v1\n-> X25519 abc\n---".
        let raw = b"age-encryption.org/v1\n-> X25519 abc\nxyz\n--- mac\n";
        let b64 = b64_encode(raw);
        let armored = format!(
            "-----BEGIN AGE ENCRYPTED FILE-----\n{b64}\n-----END AGE ENCRYPTED FILE-----\n"
        );
        assert_eq!(
            detect(armored.as_bytes()),
            Format::Age {
                armored: true,
                stanzas: vec!["X25519".into()]
            }
        );
        assert!(!detect(armored.as_bytes()).age_passphrase());
        assert_eq!(detect(MS_SIG.as_bytes()), Format::MinisignSignature);
        assert_eq!(
            detect(b"untrusted comment: minisign public key 1\nRW..."),
            Format::MinisignPublicKey
        );
        assert_eq!(
            detect(b"untrusted comment: minisign encrypted secret key\nRW..."),
            Format::MinisignSecretKey
        );
        assert_eq!(
            detect(b"# created\nAGE-SECRET-KEY-1XYZ\n"),
            Format::AgeSecretKey
        );
        assert_eq!(
            detect(b"-----BEGIN PGP SIGNATURE-----\n\nabc"),
            Format::OpenPgp {
                armored: true,
                what: "signature"
            }
        );
        assert_eq!(
            detect(b"-----BEGIN CERTIFICATE-----\nMII"),
            Format::Pem("CERTIFICATE".into())
        );
        assert_eq!(
            detect(b"-----BEGIN OPENSSH PRIVATE KEY-----\n"),
            Format::SshPrivateKey
        );
        assert_eq!(detect(b"Salted__12345678"), Format::OpenSslEnc);
        assert_eq!(detect(b"LUKS\xba\xbe\x00\x02"), Format::Luks);
        assert_eq!(
            detect(b"PK\x03\x04\x14\x00\x01\x00"),
            Format::Zip { encrypted: true }
        );
        assert_eq!(
            detect(b"PK\x03\x04\x14\x00\x00\x08"),
            Format::Zip { encrypted: false }
        );
        // gpg -c output starts with an old-format SKESK packet.
        assert_eq!(
            detect(b"\x8c\x0d\x04\x09\x03\x02"),
            Format::OpenPgp {
                armored: false,
                what: "message encrypted with a passphrase"
            }
        );
        assert_eq!(
            detect(b"\xc1\x0c\x03\x00"),
            Format::OpenPgp {
                armored: false,
                what: "message encrypted to a key"
            }
        );
        let sums = format!("{}  a.iso\n", "ab".repeat(32));
        assert_eq!(detect(sums.as_bytes()), Format::ChecksumList);
        assert_eq!(detect(b"hello world\n"), Format::Text);
        assert_eq!(detect(b"\x7fELF\x02\x01\x00\x00"), Format::Unknown);
        for f in [
            Format::Unknown,
            Format::Luks,
            Format::Zip { encrypted: true },
        ] {
            assert!(!describe(&f).1.is_empty());
        }
    }

    fn b64_encode(raw: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for ch in raw.chunks(3) {
            let n = (u32::from(ch[0]) << 16)
                | (u32::from(*ch.get(1).unwrap_or(&0)) << 8)
                | u32::from(*ch.get(2).unwrap_or(&0));
            for i in 0..=ch.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        while s.len() % 4 != 0 {
            s.push('=');
        }
        s
    }

    #[test]
    fn output_names() {
        assert_eq!(encrypted_name(OsStr::new("a.pdf")), "a.pdf.age");
        assert_eq!(decrypted_name(OsStr::new("a.pdf.age")), "a.pdf");
        assert_eq!(decrypted_name(OsStr::new("m.asc")), "m");
        assert_eq!(decrypted_name(OsStr::new(".age")), ".age.decrypted");
        assert_eq!(decrypted_name(OsStr::new("blob")), "blob.decrypted");
        let d = tmp("names");
        assert_eq!(free_path(&d, OsStr::new("a.pdf")), Some(d.join("a.pdf")));
        fs::write(d.join("a.pdf"), "x").expect("write");
        assert_eq!(
            free_path(&d, OsStr::new("a.pdf")),
            Some(d.join("a (2).pdf"))
        );
        fs::write(d.join("a (2).pdf"), "x").expect("write");
        assert_eq!(
            free_path(&d, OsStr::new("a.pdf")),
            Some(d.join("a (3).pdf"))
        );
        assert_eq!(free_path(&d, OsStr::new(".x")), Some(d.join(".x")));
        // A dangling symbolic link counts as taken.
        std::os::unix::fs::symlink("/nonexistent", d.join("l")).expect("symlink");
        assert_eq!(free_path(&d, OsStr::new("l")), Some(d.join("l (2)")));
        assert!(create_new(&d.join("l"), 0o600).is_err());
        let mut f = create_new(&d.join("new"), 0o600).expect("create");
        f.write_all(b"1").expect("write");
        assert!(create_new(&d.join("new"), 0o600).is_err());
        assert_eq!(
            fs::metadata(d.join("new")).expect("m").permissions().mode() & 0o777,
            0o600
        );
        assert!(rename_noreplace(&d.join("new"), &d.join("a.pdf")).is_err());
        rename_noreplace(&d.join("new"), &d.join("moved")).expect("rename");
        assert!(d.join("moved").exists() && !d.join("new").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn expected_checksums() {
        let h = "ab".repeat(32);
        assert_eq!(
            parse_expected(&h).expect("ok"),
            (h.clone(), &[Algo::Sha256][..])
        );
        assert_eq!(parse_expected(&h.to_uppercase()).expect("ok").0, h);
        assert_eq!(parse_expected(&format!("{h}  file.iso")).expect("ok").0, h);
        assert_eq!(parse_expected(&format!("sha256:{h}")).expect("ok").0, h);
        assert_eq!(
            parse_expected(&format!("SHA256 (file.iso) = {h}"))
                .expect("ok")
                .0,
            h
        );
        let spaced = grouped(&h);
        assert_eq!(parse_expected(&spaced).expect("ok").0, h);
        assert_eq!(parse_expected(&"0".repeat(128)).expect("ok").1.len(), 2);
        assert!(parse_expected(&"0".repeat(40)).expect("ok").1[0].weak());
        assert!(parse_expected("").is_err());
        assert!(parse_expected("hello").is_err());
        assert!(parse_expected(&"g".repeat(64)).is_err());
    }

    #[test]
    fn checksum_lists() {
        let h = "0".repeat(64);
        assert_eq!(
            checksum_list_algorithm(&format!("{h}  a.iso\n")),
            Some(Algo::Sha256)
        );
        assert_eq!(
            checksum_list_algorithm(&format!("# c\n{h} *a.iso\n")),
            Some(Algo::Sha256)
        );
        assert_eq!(
            checksum_list_algorithm(&format!("SHA512 (a.iso) = {}\n", "1".repeat(128))),
            Some(Algo::Sha512)
        );
        assert_eq!(
            checksum_list_algorithm(&format!("SHA512 (a.iso) = {h}\n")),
            None
        );
        assert_eq!(checksum_list_algorithm("hello world\n"), None);
        assert_eq!(checksum_list_algorithm(&h), None);
        assert_eq!(digest_of_output(&format!("{h}  a\n")), Some(h.clone()));
        assert_eq!(digest_of_output(&format!("\\{h}  a\\nb\n")), Some(h));
        assert_eq!(digest_of_output("sha256sum: x: No such file"), None);
    }

    #[test]
    fn trusted_comments() {
        assert_eq!(
            describe_trusted_comment("timestamp:1794732981\tfile:release.txt\thashed"),
            "signed on 2026-11-15 08:56:21 UTC, file name \"release.txt\""
        );
        assert_eq!(
            describe_trusted_comment("signed by alice"),
            "signed by alice"
        );
        assert_eq!(describe_trusted_comment("a\tb"), "a  b");
    }

    #[test]
    fn xkcdpass_output() {
        let out = "With the current options, your word list contains 7227 words.\n\
                   A 6 word password from this list will have roughly 76 (12.82 * 6) bits of entropy,\n\
                   assuming truly random word selection.\n\ngraceful encore snooze reoccur factor storeroom\n";
        assert_eq!(
            parse_xkcdpass(out),
            Some((
                "graceful encore snooze reoccur factor storeroom".into(),
                Some(76)
            ))
        );
        assert_eq!(parse_xkcdpass(""), None);
        assert_eq!(
            parse_xkcdpass("With the current options, your word list contains 0 words."),
            None
        );
    }

    #[test]
    fn short_forms() {
        assert_eq!(age_short(AGE_PUB), "...ps7j mt65");
        assert_eq!(grouped("0123456789abcdef01"), "01234567 89abcdef 01");
    }
}
