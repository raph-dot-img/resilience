//! Local messages (ADR 0018): the files shared with the per-user program
//! `resilience-messages`, which does all the networking.
//!
//! ```text
//! ~/.local/share/resilience/messages/      (0700)
//!   status.json      written by the program every second while it runs
//!   address          this account's address (32 hex digits)
//!   messages/ID.json one record per message, written by the program only
//!   peers.json       addresses heard on the network, with claimed names
//!   outbox/ID.json   written HERE: a message to send
//!   cancel/ID        written HERE: give up on a waiting message
//!   contacts.json    written HERE: names the user gave to addresses
//! ```
//!
//! Everything read comes from the network in the end (texts, names):
//! sizes and counts are bounded, what does not parse is skipped, and
//! [`clean_line`] / [`clean_text`] make text safe to draw. Nothing here
//! touches the network or starts a program.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

/// Longest message text, in bytes (the program refuses more).
pub const MAX_TEXT_BYTES: usize = 8 * 1024;
/// Largest record or list file read.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Most records read (the program keeps at most 5,000 finished ones).
pub const MAX_RECORDS: usize = 6000;
/// Most nearby addresses and contacts listed.
pub const MAX_PEERS: usize = 500;
/// Longest contact name, in characters.
pub const MAX_NAME_CHARS: usize = 40;
/// The program writes its status every second; older than this, it is
/// not running (or stuck).
pub const STATUS_FRESH_SECS: u64 = 20;

/// The message store of the account whose home is `home`.
pub fn store_dir(home: &Path) -> PathBuf {
    home.join(".local/share/resilience/messages")
}

/// A valid address: 32 lower-case hexadecimal digits.
pub fn valid_address(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// An address as typed or pasted: spaces, dashes and colons dropped,
/// upper case accepted. `None` if it is not an address.
pub fn parse_address(input: &str) -> Option<String> {
    let s: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != ':')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    valid_address(&s).then_some(s)
}

/// An address in groups of four, to read aloud and compare.
pub fn format_address(addr: &str) -> String {
    addr.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The short form shown in lists: the first two groups.
pub fn short_address(addr: &str) -> String {
    format!("{}..", addr.chars().take(8).collect::<String>())
}

fn drawable(c: char) -> bool {
    // No control characters, and none of the invisible formatting
    // characters that reorder or hide text (bidi controls, zero width).
    !c.is_control()
        && !matches!(c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// One line that is safe to draw: no control or invisible formatting
/// characters, white space runs as one space, at most `max` characters.
pub fn clean_line(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() || c.is_control() {
            space = !out.is_empty();
        } else if drawable(c) {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
        if out.chars().count() >= max {
            break;
        }
    }
    out.chars().take(max).collect()
}

/// A message text that is safe to draw: its lines, each cleaned (tabs
/// become spaces), empty lines kept, at most `max_lines`.
pub fn clean_text(s: &str, max_lines: usize) -> Vec<String> {
    s.lines()
        .take(max_lines)
        .map(|l| {
            l.chars()
                .map(|c| if c == '\t' { ' ' } else { c })
                .filter(|c| drawable(*c))
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Out,
    In,
}

/// What happened to a message (ADR 0018 §5). Never more than what is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Kept on this computer; it will be tried again.
    Queued,
    /// Handed to the network; no proof of delivery yet.
    Sent,
    /// The recipient's computer confirmed it has the message.
    Delivered,
    /// Given up; `detail` says why.
    Failed,
    /// An incoming message.
    Received,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Queued => "waiting",
            State::Sent => "sent",
            State::Delivered => "delivered",
            State::Failed => "FAILED",
            State::Received => "received",
        }
    }

    /// Whether the user can still cancel it.
    pub fn pending(self) -> bool {
        matches!(self, State::Queued | State::Sent)
    }
}

/// One message, as the program recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Record {
    pub id: String,
    pub direction: Direction,
    /// The other person's address; empty for a refused request.
    #[serde(default)]
    pub peer: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub created: u64,
    pub state: State,
    /// Incoming only: the signature was checked against the sender's key.
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub detail: String,
}

impl Record {
    /// The marker shown beside the message.
    pub fn marker(&self) -> String {
        match (self.direction, self.state) {
            (Direction::In, _) if self.verified => "from them".into(),
            (Direction::In, _) => "from them, NOT VERIFIED".into(),
            (_, s) => s.label().into(),
        }
    }
}

/// Someone heard on the network. The name is what that address claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub address: String,
    pub claimed_name: String,
    pub heard: u64,
}

/// Whether the account's messaging program is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Running {
        address: String,
    },
    /// Not running, or not answering; the address is known if it ever ran.
    Stopped {
        address: Option<String>,
    },
}

impl Status {
    pub fn address(&self) -> Option<&str> {
        match self {
            Status::Running { address } => Some(address),
            Status::Stopped { address } => address.as_deref(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SendError {
    #[error("this is not an address: an address has 32 characters, digits and the letters a to f")]
    BadAddress,
    #[error("the message is empty")]
    Empty,
    #[error("the message is too long ({0} bytes; at most {MAX_TEXT_BYTES})")]
    TooLong(usize),
    #[error("this is your own address")]
    Own,
    #[error("could not save the message: {0}")]
    Io(String),
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut buf = Vec::new();
    f.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= MAX_FILE_BYTES).then_some(buf)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Write `data` at `path` through a temporary name (the program ignores
/// `*.tmp`), mode 0600, never following a link.
fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// One account's message store.
#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    fn dir(&self, name: &str) -> io::Result<PathBuf> {
        let d = self.root.join(name);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&d)?;
        Ok(d)
    }

    /// Whether the program is running, from its status file and `now`
    /// (seconds since 1970).
    pub fn status(&self, now: u64) -> Status {
        #[derive(Deserialize)]
        struct Raw {
            address: String,
            time: u64,
        }
        let known = fs::read_to_string(self.root.join("address"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| valid_address(s));
        let raw = read_bounded(&self.root.join("status.json"))
            .and_then(|b| serde_json::from_slice::<Raw>(&b).ok())
            .filter(|r| valid_address(&r.address));
        match raw {
            Some(r) if now.saturating_sub(r.time) <= STATUS_FRESH_SECS => {
                Status::Running { address: r.address }
            }
            Some(r) => Status::Stopped {
                address: Some(r.address),
            },
            None => Status::Stopped { address: known },
        }
    }

    /// Every readable record, oldest first (by time, then id).
    pub fn records(&self) -> Vec<Record> {
        let Ok(dir) = fs::read_dir(self.root.join("messages")) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in dir.flatten().take(MAX_RECORDS * 2) {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            let Some(rec) =
                read_bounded(&entry.path()).and_then(|b| serde_json::from_slice::<Record>(&b).ok())
            else {
                continue;
            };
            let peer_ok = rec.peer.is_empty() || valid_address(&rec.peer);
            if rec.id == id && peer_ok && rec.text.len() <= 4 * MAX_TEXT_BYTES {
                out.push(rec);
            }
            if out.len() >= MAX_RECORDS {
                break;
            }
        }
        out.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
        out
    }

    /// People heard on the network, most recently heard first.
    pub fn peers(&self) -> Vec<Peer> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            name: String,
            #[serde(default)]
            heard: u64,
        }
        let map: BTreeMap<String, Raw> = read_bounded(&self.root.join("peers.json"))
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut out: Vec<Peer> = map
            .into_iter()
            .filter(|(a, _)| valid_address(a))
            .map(|(address, r)| Peer {
                address,
                claimed_name: clean_line(&r.name, MAX_NAME_CHARS),
                heard: r.heard,
            })
            .collect();
        out.sort_by(|a, b| b.heard.cmp(&a.heard).then(a.address.cmp(&b.address)));
        out.truncate(MAX_PEERS);
        out
    }

    /// Names the user gave to addresses.
    pub fn contacts(&self) -> BTreeMap<String, String> {
        let map: BTreeMap<String, String> = read_bounded(&self.root.join("contacts.json"))
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        map.into_iter()
            .filter(|(a, _)| valid_address(a))
            .map(|(a, n)| (a, clean_line(&n, MAX_NAME_CHARS)))
            .filter(|(_, n)| !n.is_empty())
            .take(MAX_PEERS)
            .collect()
    }

    /// Give `address` a name (an empty name removes the contact).
    pub fn set_contact(&self, address: &str, name: &str) -> Result<(), SendError> {
        if !valid_address(address) {
            return Err(SendError::BadAddress);
        }
        let mut all = self.contacts();
        let name = clean_line(name, MAX_NAME_CHARS);
        if name.is_empty() {
            all.remove(address);
        } else {
            all.insert(address.to_string(), name);
        }
        let io = |e: io::Error| SendError::Io(e.to_string());
        self.dir("").map_err(io)?;
        let data = serde_json::to_vec(&all).map_err(|e| SendError::Io(e.to_string()))?;
        write_new(&self.root.join("contacts.json"), &data).map_err(io)
    }

    /// Ask the program to send `text` to `to`. Returns the message's id;
    /// its record appears once the program has taken the request.
    pub fn queue(&self, to: &str, text: &str, own: Option<&str>) -> Result<String, SendError> {
        if !valid_address(to) {
            return Err(SendError::BadAddress);
        }
        if own == Some(to) {
            return Err(SendError::Own);
        }
        let text = text.trim_end();
        if text.trim().is_empty() {
            return Err(SendError::Empty);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(SendError::TooLong(text.len()));
        }
        let io = |e: io::Error| SendError::Io(e.to_string());
        let dir = self.dir("outbox").map_err(io)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let id = format!("m{}-{:09}-{}", now_secs(), nanos, std::process::id());
        let body = serde_json::json!({ "to": to, "text": text });
        let data = serde_json::to_vec(&body).map_err(|e| SendError::Io(e.to_string()))?;
        write_new(&dir.join(format!("{id}.json")), &data).map_err(io)?;
        Ok(id)
    }

    /// Ask the program to give up on a waiting message.
    pub fn cancel(&self, id: &str) -> io::Result<()> {
        let ok = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            && !id.starts_with('-');
        if !ok {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "bad message id",
            ));
        }
        let dir = self.dir("cancel")?;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join(id))
            .map(|_| ())
    }
}

/// What to call an address, and how sure that is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Name {
    /// A name the user chose for this address.
    Contact(String),
    /// A name the address announces: a claim, not checked by anyone.
    Claimed(String),
    Unknown,
}

impl Name {
    pub fn of(address: &str, contacts: &BTreeMap<String, String>, peers: &[Peer]) -> Name {
        if let Some(n) = contacts.get(address) {
            return Name::Contact(n.clone());
        }
        match peers.iter().find(|p| p.address == address) {
            Some(p) if !p.claimed_name.is_empty() => Name::Claimed(p.claimed_name.clone()),
            _ => Name::Unknown,
        }
    }

    /// For lists: `Marie`, `"marie"?` (claimed) or the short address.
    pub fn display(&self, address: &str) -> String {
        match self {
            Name::Contact(n) => n.clone(),
            Name::Claimed(n) => format!("\"{n}\"?"),
            Name::Unknown => short_address(address),
        }
    }
}

/// The messages exchanged with one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub peer: String,
    pub count: usize,
    /// Messages still waiting or sent without proof.
    pub pending: usize,
    pub failed: usize,
    pub last_time: u64,
    pub last_text: String,
    pub last_direction: Direction,
}

/// Conversations, the most recent first. Refused requests without an
/// address are left out (they are shown as errors when they happen).
pub fn conversations(records: &[Record]) -> Vec<Conversation> {
    let mut by_peer: BTreeMap<&str, Conversation> = BTreeMap::new();
    for r in records.iter().filter(|r| !r.peer.is_empty()) {
        let c = by_peer.entry(&r.peer).or_insert_with(|| Conversation {
            peer: r.peer.clone(),
            count: 0,
            pending: 0,
            failed: 0,
            last_time: 0,
            last_text: String::new(),
            last_direction: r.direction,
        });
        c.count += 1;
        c.pending += usize::from(r.state.pending());
        c.failed += usize::from(r.state == State::Failed);
        if r.created >= c.last_time {
            c.last_time = r.created;
            c.last_text = clean_line(&r.text, 60);
            c.last_direction = r.direction;
        }
    }
    let mut out: Vec<Conversation> = by_peer.into_values().collect();
    out.sort_by(|a, b| b.last_time.cmp(&a.last_time).then(a.peer.cmp(&b.peer)));
    out
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(then: u64, now: u64) -> String {
    let d = now.saturating_sub(then);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "ffffffffffffffffffffffffffffffff";

    fn temp(tag: &str) -> Store {
        let d = std::env::temp_dir().join(format!("res-msg-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("messages")).expect("mkdir");
        Store::new(d)
    }

    fn record(s: &Store, id: &str, json: &str) {
        fs::write(s.root.join("messages").join(format!("{id}.json")), json).expect("w");
    }

    #[test]
    fn addresses() {
        assert!(valid_address(A));
        for bad in [
            "",
            "0123",
            &A[1..],
            &format!("{A}0"),
            &A.to_uppercase(),
            &A.replace('0', "g"),
        ] {
            assert!(!valid_address(bad), "{bad}");
        }
        assert_eq!(parse_address(&format_address(A)).as_deref(), Some(A));
        assert_eq!(parse_address(&A.to_uppercase()).as_deref(), Some(A));
        assert_eq!(
            parse_address(" 0123-4567:89ab cdef 0123 4567 89ab cdef\n").as_deref(),
            Some(A)
        );
        assert_eq!(parse_address("hello"), None);
        assert_eq!(format_address(A), "0123 4567 89ab cdef 0123 4567 89ab cdef");
        assert_eq!(short_address(A), "01234567..");
    }

    #[test]
    fn hostile_text_is_made_safe_to_draw() {
        assert_eq!(
            clean_line("  a\tb\n\x1b[2Jc\u{202E}d\u{200B}  ", 40),
            "a b [2Jcd"
        );
        assert_eq!(clean_line("abcdef", 3), "abc");
        assert_eq!(clean_line("\x07\x00", 9), "");
        assert_eq!(
            clean_text("one\ttwo\x1b[31m\n\n  three \u{202E}x\r\nfour", 3),
            ["one two[31m", "", "  three x"]
        );
        assert!(clean_text("", 5).is_empty());
    }

    #[test]
    fn records_are_read_bounded_and_sorted() {
        let s = temp("records");
        record(
            &s,
            "m2",
            &format!(
                r#"{{"id":"m2","direction":"out","peer":"{B}","text":"second","created":20,"state":"delivered","verified":false,"detail":"","attempts":1,"next_try":0,"updated":21}}"#
            ),
        );
        record(
            &s,
            "m1",
            &format!(
                r#"{{"id":"m1","direction":"out","peer":"{B}","text":"first","created":10,"state":"queued","detail":"waiting"}}"#
            ),
        );
        record(
            &s,
            "in-1",
            &format!(
                r#"{{"id":"in-1","direction":"in","peer":"{B}","text":"reply","created":15,"state":"received","verified":true}}"#
            ),
        );
        // Skipped: wrong id, bad state, bad peer, not JSON, a link, too large.
        record(
            &s,
            "x",
            &format!(r#"{{"id":"y","direction":"out","peer":"{B}","state":"sent"}}"#),
        );
        record(
            &s,
            "bad-state",
            r#"{"id":"bad-state","direction":"out","state":"teleported"}"#,
        );
        record(
            &s,
            "bad-peer",
            r#"{"id":"bad-peer","direction":"in","peer":"../x","state":"received"}"#,
        );
        record(&s, "junk", "not json");
        std::os::unix::fs::symlink("/etc/passwd", s.root.join("messages/link.json")).expect("ln");
        record(&s, "huge", &"x".repeat(2 * 1024 * 1024));
        fs::write(s.root.join("messages/notes.txt"), "x").expect("w");

        let r = s.records();
        assert_eq!(
            r.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["m1", "in-1", "m2"]
        );
        assert_eq!(r[0].state, State::Queued);
        assert_eq!(r[0].marker(), "waiting");
        assert_eq!(r[1].marker(), "from them");
        assert_eq!(r[2].marker(), "delivered");
        let c = conversations(&r);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].count, c[0].pending, c[0].failed), (3, 1, 0));
        assert_eq!(
            (c[0].last_text.as_str(), c[0].last_direction),
            ("second", Direction::Out)
        );
        assert!(Store::new(s.root.join("missing")).records().is_empty());
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn an_unverified_message_says_so() {
        let r: Record = serde_json::from_str(&format!(
            r#"{{"id":"in-2","direction":"in","peer":"{B}","text":"x","state":"received"}}"#
        ))
        .expect("record");
        assert!(!r.verified);
        assert_eq!(r.marker(), "from them, NOT VERIFIED");
    }

    #[test]
    fn status_peers_and_names() {
        let s = temp("status");
        assert_eq!(s.status(100), Status::Stopped { address: None });
        fs::write(s.root.join("address"), format!("{A}\n")).expect("w");
        assert_eq!(
            s.status(100),
            Status::Stopped {
                address: Some(A.into())
            }
        );
        fs::write(
            s.root.join("status.json"),
            format!(r#"{{"address":"{A}","name":"n","pid":1,"time":90}}"#),
        )
        .expect("w");
        assert_eq!(s.status(100), Status::Running { address: A.into() });
        assert_eq!(
            s.status(90 + STATUS_FRESH_SECS + 1),
            Status::Stopped {
                address: Some(A.into())
            }
        );
        fs::write(s.root.join("status.json"), r#"{"address":"zz","time":99}"#).expect("w");
        assert_eq!(s.status(100).address(), Some(A));

        fs::write(
            s.root.join("peers.json"),
            format!(r#"{{"{B}":{{"name":"Marie\u001b[2J\n Curie","heard":50}},"{A}":{{"heard":70}},"nope":{{"name":"x","heard":99}}}}"#),
        )
        .expect("w");
        let p = s.peers();
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].address.as_str(), p[0].claimed_name.as_str()), (A, ""));
        assert_eq!(p[1].claimed_name, "Marie [2J Curie");

        let mut contacts = s.contacts();
        assert!(contacts.is_empty());
        assert_eq!(
            Name::of(B, &contacts, &p).display(B),
            "\"Marie [2J Curie\"?"
        );
        assert_eq!(Name::of(A, &contacts, &p).display(A), "01234567..");
        s.set_contact(B, "  Marie\x07 C. ").expect("set");
        contacts = s.contacts();
        assert_eq!(Name::of(B, &contacts, &p), Name::Contact("Marie C.".into()));
        let mode = fs::metadata(s.root.join("contacts.json"))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        s.set_contact(B, "").expect("remove");
        assert!(s.contacts().is_empty());
        assert_eq!(s.set_contact("x", "n"), Err(SendError::BadAddress));
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn queue_and_cancel_write_what_the_program_reads() {
        let s = temp("queue");
        assert_eq!(s.queue("nope", "hi", None), Err(SendError::BadAddress));
        assert_eq!(s.queue(B, " \n ", None), Err(SendError::Empty));
        assert_eq!(s.queue(B, "hi", Some(B)), Err(SendError::Own));
        let long = "é".repeat(MAX_TEXT_BYTES / 2 + 1);
        assert_eq!(
            s.queue(B, &long, None),
            Err(SendError::TooLong(MAX_TEXT_BYTES + 2))
        );
        assert!(
            !s.root.join("outbox").exists()
                || fs::read_dir(s.root.join("outbox"))
                    .expect("ls")
                    .next()
                    .is_none()
        );

        let id = s
            .queue(B, "Water at \"the\" school\n9h\n", Some(A))
            .expect("queue");
        let path = s.root.join("outbox").join(format!("{id}.json"));
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        assert_eq!(v["to"], B);
        assert_eq!(v["text"], "Water at \"the\" school\n9h");
        assert_eq!(
            fs::metadata(&path).expect("meta").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(s.root.join("outbox"))
                .expect("meta")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        // Only the finished file is there.
        assert_eq!(fs::read_dir(s.root.join("outbox")).expect("ls").count(), 1);
        let id2 = s.queue(B, "again", None).expect("queue");
        assert_ne!(id, id2);

        s.cancel(&id).expect("cancel");
        assert!(s.root.join("cancel").join(&id).is_file());
        for bad in ["", "../x", "a/b", "-x", &"a".repeat(65)] {
            assert!(s.cancel(bad).is_err(), "{bad}");
        }
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn times_are_said_roughly() {
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(100, 100 + 300), "5 min ago");
        assert_eq!(ago(100, 100 + 3 * 3600), "3 h ago");
        assert_eq!(ago(100, 100 + 2 * 86_400), "2 days ago");
        assert_eq!(ago(500, 100), "just now");
    }
}
