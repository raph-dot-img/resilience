//! Finding and viewing files from the interface (Phase 3: file search,
//! document viewing, CSV).
//!
//! Everything here only reads. The index is a walk of the user's home
//! folder (and open USB sticks) that skips symbolic links, other file
//! systems below the root and, unless asked, hidden entries. Viewing never
//! runs anything from the file: text is read directly, PDF goes through
//! `pdftotext`, archives are listed (never extracted) with `bsdtar -t`.

use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Entries indexed at most (the walk stops there and says so).
pub const MAX_ENTRIES: usize = 100_000;
/// Results shown at most.
pub const MAX_HITS: usize = 300;
/// Bytes read at most when showing a text file.
pub const MAX_TEXT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// Path shown to the user, e.g. `~/Notes/x.md` or `STICK/Docs/y.pdf`.
    pub shown: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub entries: Vec<Entry>,
    /// The walk stopped at [`MAX_ENTRIES`] (files and folders seen).
    pub truncated: bool,
    /// Folders walked.
    pub visited: usize,
}

/// Index regular files below each `(root, label)`. The label replaces the
/// root in shown paths. Hidden files and folders are skipped; symbolic
/// links are never followed; the walk stays on the root's file system.
pub fn index(roots: &[(PathBuf, String)]) -> Index {
    let mut out = Index::default();
    for (root, label) in roots {
        let Ok(meta) = fs::symlink_metadata(root) else {
            continue;
        };
        let dev = meta.dev();
        walk(root, root, label, dev, 0, &mut out);
        if out.truncated {
            break;
        }
    }
    out.entries.sort_by(|a, b| a.shown.cmp(&b.shown));
    out
}

fn walk(root: &Path, dir: &Path, label: &str, dev: u64, depth: usize, out: &mut Index) {
    if depth > 16 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.entries.len() + out.visited >= MAX_ENTRIES {
            out.truncated = true;
            return;
        }
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = e.path();
        let Ok(m) = fs::symlink_metadata(&path) else {
            continue;
        };
        if m.dev() != dev {
            continue;
        }
        if m.is_dir() {
            out.visited += 1;
            walk(root, &path, label, dev, depth + 1, out);
        } else if m.is_file() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.entries.push(Entry {
                shown: format!("{label}/{}", rel.display()),
                size: m.len(),
                modified: m.modified().ok(),
                path,
            });
        }
    }
}

/// Entries whose shown path contains every word of `query` (any case).
pub fn search<'a>(idx: &'a Index, query: &str) -> Vec<&'a Entry> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    idx.entries
        .iter()
        .filter(|e| {
            let s = e.shown.to_lowercase();
            words.iter().all(|w| s.contains(w.as_str()))
        })
        .take(MAX_HITS)
        .collect()
}

/// How a file is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Table {
        separator: u8,
    },
    Pdf,
    Archive,
    Zim,
    /// Anything else: details and checksum only.
    Other,
}

fn lower_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Decide from the name, then from the first bytes for unknown names.
pub fn kind(path: &Path, head: &[u8]) -> Kind {
    let n = lower_name(path);
    let ends = |exts: &[&str]| exts.iter().any(|e| n.ends_with(e));
    if ends(&[".csv"]) {
        Kind::Table { separator: b',' }
    } else if ends(&[".tsv", ".tab"]) {
        Kind::Table { separator: b'\t' }
    } else if ends(&[".pdf"]) || head.starts_with(b"%PDF-") {
        Kind::Pdf
    } else if ends(&[".zim"]) {
        Kind::Zim
    } else if ends(&[
        ".zip", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".txz", ".tar.zst", ".tar.bz2", ".tbz2",
        ".7z", ".iso", ".cpio", ".rar", ".gz", ".xz", ".zst", ".bz2",
    ]) {
        Kind::Archive
    } else if looks_like_text(head) {
        Kind::Text
    } else {
        Kind::Other
    }
}

/// UTF-8 (allowing a character cut at the end) without NUL bytes.
pub fn looks_like_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && e.valid_up_to() + 4 > head.len(),
    }
}

/// The first bytes of a file, to decide its kind.
pub fn head(path: &Path) -> Vec<u8> {
    let mut buf = vec![0u8; 8192];
    let n = open_regular(path)
        .and_then(|mut f| f.read(&mut buf))
        .unwrap_or(0);
    buf.truncate(n);
    buf
}

/// Read a text file, at most [`MAX_TEXT`] bytes; says when it was cut.
/// Open a file for reading only if it is (still) a regular file: never a
/// symbolic link, and without blocking on a FIFO put in its place.
pub fn open_regular(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok(f)
}

pub fn read_text(path: &Path) -> std::io::Result<String> {
    let f = open_regular(path)?;
    let len = f.metadata()?.len();
    let mut bytes = Vec::new();
    f.take(MAX_TEXT).read_to_end(&mut bytes)?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if len > MAX_TEXT {
        text.push_str(&format!(
            "\n\n[Only the first {} of {} are shown.]",
            crate::human_bytes(MAX_TEXT),
            crate::human_bytes(len)
        ));
    }
    // Control characters other than tab and newline would move the cursor.
    Ok(text
        .chars()
        .map(|c| match c {
            '\t' | '\n' => c,
            c if c.is_control() => ' ',
            c => c,
        })
        .collect())
}

/// Parse CSV/TSV: quoted fields with `""` escapes, CRLF or LF. At most
/// `max_rows` rows; a malformed quote ends the field at the end of line.
/// Columns kept per row.
pub const MAX_COLUMNS: usize = 500;

pub fn parse_table(text: &str, separator: u8, max_rows: usize) -> Vec<Vec<String>> {
    let sep = separator as char;
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == sep => {
                let f = std::mem::take(&mut field);
                if row.len() < MAX_COLUMNS {
                    row.push(f);
                }
            }
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                if rows.len() >= max_rows {
                    return rows;
                }
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("res-files-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn index_skips_hidden_entries_and_links() {
        let d = tmp("index");
        fs::create_dir_all(d.join("Notes/sub")).expect("mkdir");
        fs::create_dir_all(d.join(".cache")).expect("mkdir");
        fs::write(d.join("Notes/Water plan.md"), "x").expect("w");
        fs::write(d.join("Notes/sub/budget.csv"), "a,b").expect("w");
        fs::write(d.join(".cache/secret.txt"), "x").expect("w");
        fs::write(d.join(".hidden.txt"), "x").expect("w");
        std::os::unix::fs::symlink("/etc", d.join("etc-link")).expect("ln");
        std::os::unix::fs::symlink("/etc/passwd", d.join("pw")).expect("ln");
        let idx = index(&[(d.clone(), "~".into())]);
        let shown: Vec<&str> = idx.entries.iter().map(|e| e.shown.as_str()).collect();
        assert_eq!(shown, ["~/Notes/Water plan.md", "~/Notes/sub/budget.csv"]);
        let hits: Vec<&str> = search(&idx, "WATER notes")
            .iter()
            .map(|e| e.shown.as_str())
            .collect();
        assert_eq!(hits, ["~/Notes/Water plan.md"]);
        assert!(search(&idx, "  ").is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn kinds() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(kind(&p("a.CSV"), b""), Kind::Table { separator: b',' });
        assert_eq!(kind(&p("a.tsv"), b""), Kind::Table { separator: b'\t' });
        assert_eq!(kind(&p("doc"), b"%PDF-1.7\n"), Kind::Pdf);
        assert_eq!(kind(&p("x.tar.zst"), b""), Kind::Archive);
        assert_eq!(kind(&p("w.zim"), b"ZIM"), Kind::Zim);
        assert_eq!(kind(&p("notes"), "été\n".as_bytes()), Kind::Text);
        assert_eq!(
            kind(&p("photo.jpg"), b"\xff\xd8\xff\xe0\0\x10JFIF"),
            Kind::Other
        );
        // A multi-byte character cut by the 8 KiB head is still text.
        assert!(looks_like_text(&"aé".as_bytes()[..2]));
        assert!(!looks_like_text(b"\xff\xfe\xfd text"));
    }

    #[test]
    fn text_is_capped_and_cleaned() {
        let d = tmp("text");
        fs::write(d.join("t.txt"), "a\x1b[2Jb\tc\n").expect("w");
        assert_eq!(read_text(&d.join("t.txt")).expect("read"), "a [2Jb\tc\n");
        let big = vec![b'x'; (MAX_TEXT + 10) as usize];
        fs::write(d.join("big.txt"), big).expect("w");
        let t = read_text(&d.join("big.txt")).expect("read");
        assert!(t.ends_with("are shown.]"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tables_with_quotes() {
        let t = "name,notes\r\n\"Smith, J\",\"said \"\"hi\"\"\"\nx,\"two\nlines\"\nlast";
        let rows = parse_table(t, b',', 100);
        assert_eq!(
            rows,
            vec![
                vec!["name", "notes"],
                vec!["Smith, J", "said \"hi\""],
                vec!["x", "two\nlines"],
                vec!["last"],
            ]
        );
        assert_eq!(parse_table("a\tb\n", b'\t', 100), vec![vec!["a", "b"]]);
        assert_eq!(parse_table("1\n2\n3\n", b',', 2).len(), 2);
        let wide = ",".repeat(100_000) + "\n";
        assert!(parse_table(&wide, b',', 10)[0].len() <= MAX_COLUMNS + 1);
    }
}
