//! The built-in Resilience handbook: plain-text pages shipped in the system
//! closure (`share/resilience/handbook`). Readable with no network and no
//! external viewer.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub title: String,
    pub path: PathBuf,
}

/// Maximum page size read into memory. Handbook pages are small text files;
/// anything bigger is refused rather than loaded.
pub const MAX_PAGE_BYTES: u64 = 512 * 1024;

pub fn title_of(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// List pages (`*.txt`, `*.md`) sorted by file name, so authors control the
/// order with numeric prefixes (`10-first-steps.md`).
pub fn list(dir: &Path) -> std::io::Result<Vec<Page>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file() && matches!(p.extension().and_then(|e| e.to_str()), Some("txt" | "md"))
        })
        .collect();
    files.sort();
    Ok(files
        .into_iter()
        .map(|path| {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let head = read(&path).unwrap_or_default();
            Page {
                title: title_of(&head, &stem),
                path,
            }
        })
        .collect())
}

pub fn read(path: &Path) -> std::io::Result<String> {
    let len = fs::metadata(path)?.len();
    if len > MAX_PAGE_BYTES {
        return Err(std::io::Error::other(format!(
            "page too large ({len} bytes)"
        )));
    }
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title_of("\n# First Steps\nbody", "x"), "First Steps");
        assert_eq!(title_of("Plain title\n", "x"), "Plain title");
        assert_eq!(title_of("", "fallback"), "fallback");
        assert_eq!(title_of("#\n", "fallback"), "fallback");
    }

    #[test]
    fn lists_sorted_pages() {
        let dir = std::env::temp_dir().join(format!("resilience-hb-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join("20-b.md"), "# Second\n").expect("w");
        fs::write(dir.join("10-a.txt"), "# First\n").expect("w");
        fs::write(dir.join("notes.bin"), "ignored").expect("w");
        let pages = list(&dir).expect("list");
        assert_eq!(
            pages.iter().map(|p| p.title.as_str()).collect::<Vec<_>>(),
            vec!["First", "Second"]
        );
        fs::remove_dir_all(&dir).expect("cleanup");
        assert!(list(&dir).is_err());
    }
}
