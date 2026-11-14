//! Personal notes stored as plain text files in `~/Notes`.
//!
//! Note names are validated strictly so that a name can never escape the
//! notes directory or start a hidden file.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NoteError {
    #[error("the name is empty")]
    Empty,
    #[error("the name is longer than 64 characters")]
    TooLong,
    #[error("only letters, digits, spaces, '-', '_' and '.' are allowed")]
    BadCharacter,
    #[error("the name may not start with '.' or a space")]
    BadStart,
    #[error("a note with this name already exists")]
    Exists,
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct Note {
    pub name: String,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

pub fn notes_dir(home: &Path) -> PathBuf {
    home.join("Notes")
}

/// Validate a user-supplied note name and return the file name to use.
/// A `.txt` extension is added when the name has no `.txt`/`.md` extension.
pub fn file_name_for(name: &str) -> Result<String, NoteError> {
    let name = name.trim_end();
    if name.is_empty() {
        return Err(NoteError::Empty);
    }
    if name.chars().count() > 64 {
        return Err(NoteError::TooLong);
    }
    if name.starts_with('.') || name.starts_with(' ') {
        return Err(NoteError::BadStart);
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return Err(NoteError::BadCharacter);
    }
    if name.contains("..") {
        return Err(NoteError::BadCharacter);
    }
    Ok(if name.ends_with(".txt") || name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.txt")
    })
}

/// List notes, most recently modified first. A missing directory is empty.
pub fn list(dir: &Path) -> Vec<Note> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut notes: Vec<Note> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let name = e.file_name().to_string_lossy().into_owned();
            (meta.is_file() && !name.starts_with('.')).then(|| Note {
                name,
                path: e.path(),
                modified: meta.modified().ok(),
                size: meta.len(),
            })
        })
        .collect();
    notes.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    notes
}

/// Create an empty private note (mode 0600) and return its path.
pub fn create(dir: &Path, name: &str) -> Result<PathBuf, NoteError> {
    let file = file_name_for(name)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| NoteError::Io(e.to_string()))?;
    let path = dir.join(file);
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(_) => Ok(path),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(NoteError::Exists),
        Err(e) => Err(NoteError::Io(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert_eq!(
            file_name_for("shopping list").as_deref(),
            Ok("shopping list.txt")
        );
        assert_eq!(file_name_for("plan.md").as_deref(), Ok("plan.md"));
        assert_eq!(file_name_for("Médecine").as_deref(), Ok("Médecine.txt"));
        assert_eq!(file_name_for(""), Err(NoteError::Empty));
        assert_eq!(file_name_for("   "), Err(NoteError::Empty));
        assert_eq!(file_name_for(".hidden"), Err(NoteError::BadStart));
        assert_eq!(file_name_for("../etc/passwd"), Err(NoteError::BadStart));
        assert_eq!(file_name_for("a/b"), Err(NoteError::BadCharacter));
        assert_eq!(file_name_for("a..b"), Err(NoteError::BadCharacter));
        assert_eq!(file_name_for("tab\there"), Err(NoteError::BadCharacter));
        assert_eq!(file_name_for(&"x".repeat(65)), Err(NoteError::TooLong));
    }

    #[test]
    fn creates_private_notes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("resilience-notes-{}", std::process::id()));
        let p = create(&dir, "first").expect("create");
        assert_eq!(
            fs::metadata(&p).expect("m").permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(create(&dir, "first"), Err(NoteError::Exists));
        let l = list(&dir);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name, "first.txt");
        fs::remove_dir_all(&dir).expect("cleanup");
        assert!(list(&dir).is_empty());
    }
}
