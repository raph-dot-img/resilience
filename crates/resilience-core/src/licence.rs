//! The licence notice (ADR 0024): Resilience's own code is under the
//! Hippocratic License 2.1. Each account is shown a summary and the
//! original text before its first use of the interface, certifies it has
//! read it, and `resilience-admin record-licence` appends a timestamped
//! line to a root-owned record. Pure rules only.

use serde::Deserialize;
use std::path::PathBuf;

/// Root-owned record of who certified reading which licence text, when.
pub const RECORD_FILE: &str = "/var/lib/resilience/licence-acceptances.log";

/// The `[licence]` section of system.toml and admin.toml.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LicenceSettings {
    /// "Hippocratic License".
    pub name: String,
    /// "2.1".
    pub version: String,
    /// The original text, as shown.
    pub file: PathBuf,
    /// SHA-256 of `file` (hex), computed when the system was built.
    pub sha256: String,
    /// The record (normally [`RECORD_FILE`]).
    pub record: PathBuf,
    /// Show the notice (false only in automated tests).
    #[serde(default = "yes")]
    pub prompt: bool,
}

fn yes() -> bool {
    true
}

impl LicenceSettings {
    pub fn title(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

/// 64 lower-case hexadecimal digits.
pub fn valid_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Fields of one record line, after the time (see `admin::audit_line`).
pub fn record_fields<'a>(
    user: &'a str,
    uid: &'a str,
    s: &'a LicenceSettings,
) -> [(&'static str, &'a str); 5] {
    [
        ("user", user),
        ("uid", uid),
        ("sha256", s.sha256.as_str()),
        ("licence", s.name.as_str()),
        ("version", s.version.as_str()),
    ]
}

/// Whether `user` has a record for the text with this hash. The account
/// name and hash are the second and fourth fields, written by the helper
/// before any quoted value, so a quoted value cannot imitate them.
pub fn has_read(record: &str, user: &str, sha256: &str) -> bool {
    let want_user = format!("user={user}");
    let want_hash = format!("sha256={sha256}");
    record.lines().any(|l| {
        let mut f = l.split(' ');
        let (time, u, _uid, h) = (f.next(), f.next(), f.next(), f.next());
        time.is_some_and(|t| t.starts_with("time="))
            && u == Some(want_user.as_str())
            && h == Some(want_hash.as_str())
    })
}

/// The plain-language summary shown before the original text. It must
/// never replace or contradict the text: it says so.
pub const SUMMARY: &str = "\
Resilience's own programs are free to use, copy, change and share under
the Hippocratic License 2.1, an \"Ethical Source\" licence. This page is a
short summary in plain words. It is not legal advice and it does not
replace the licence: the original text, on the next page, is what counts.

WHAT YOU MAY DO
  - Use Resilience's programs for any purpose that respects human rights.
  - Copy them, change them and give them to others, free of charge.

WHAT YOU MUST DO
  - Respect human rights: use the software in a way consistent with the
    Universal Declaration of Human Rights and the UN Global Compact, and
    never for anything that violates laws protecting human, civil,
    labour, privacy, political, environmental, security, economic or
    due-process rights.
  - Keep the licence and the copyright notice with every copy you give,
    and say clearly when you changed something.
  - Disputes about human rights under the licence go to public
    arbitration (Hague Rules on Business and Human Rights Arbitration).

WHAT HAPPENS OTHERWISE
  - Breaking these conditions is a breach of the licence and can end
    your permission. A missing notice can be fixed within 30 days of
    being told.
  - You are responsible for harm caused by a use that violates human
    rights, and must cover the authors' costs for it.

NO WARRANTY
  - The software comes \"as is\". Nobody promises it works or is fit for
    a purpose, and the authors are not liable for damages.

WHAT IT COVERS
  - The licence covers the programs made by the Resilience project. This
    computer also runs other free software (Linux, NixOS, GNU tools and
    many more), each under its own licence; nothing here changes those
    licences or your rights under them.
  - The licence itself says it is accepted by using the software.
    Confirming here records that this text was shown to you and that you
    say you read it, with the date and time of this computer's clock.

Press Tab to read the original text.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::audit_line;

    fn settings() -> LicenceSettings {
        LicenceSettings {
            name: "Hippocratic License".into(),
            version: "2.1".into(),
            file: "/nix/store/x-LICENSE".into(),
            sha256: "a".repeat(64),
            record: RECORD_FILE.into(),
            prompt: true,
        }
    }

    #[test]
    fn hashes_are_64_lower_hex_digits() {
        assert!(valid_sha256(&"0123456789abcdef".repeat(4)));
        for bad in [
            "",
            "abc",
            &"A".repeat(64),
            &"g".repeat(64),
            &"a".repeat(65),
            &format!("{} ", "a".repeat(63)),
        ] {
            assert!(!valid_sha256(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_record_line_is_found_for_its_user_and_text_only() {
        let s = settings();
        let line = audit_line("2026-11-18T20:00:00Z", &record_fields("alice", "1000", &s));
        assert_eq!(
            line,
            format!(
                "time=2026-11-18T20:00:00Z user=alice uid=1000 sha256={} \
                 licence=\"Hippocratic License\" version=2.1\n",
                s.sha256
            )
        );
        assert!(has_read(&line, "alice", &s.sha256));
        assert!(!has_read(&line, "bob", &s.sha256));
        assert!(!has_read(&line, "alic", &s.sha256));
        assert!(!has_read(&line, "alice", &"b".repeat(64)));
        assert!(!has_read("", "alice", &s.sha256));
        // A forged-looking value later in a line does not count.
        let odd = format!(
            "time=x user=bob uid=1 sha256={} licence=\"user=alice sha256={}\"\n",
            "c".repeat(64),
            s.sha256
        );
        assert!(!has_read(&odd, "alice", &s.sha256));
        assert!(has_read(&format!("{odd}{line}"), "alice", &s.sha256));
    }

    #[test]
    fn the_summary_defers_to_the_original_text() {
        assert!(SUMMARY.contains("original text"));
        assert!(SUMMARY.contains("not legal advice"));
        assert!(SUMMARY.contains("own licence"));
        assert!(SUMMARY.lines().all(|l| l.chars().count() <= 76));
    }
}
