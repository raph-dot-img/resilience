//! Application catalog.
//!
//! The catalog is generated at build time by the NixOS modules
//! (`resilience.apps.<id>`) and installed read-only as
//! `/etc/resilience/apps.toml`. Commands are absolute Nix store paths with a
//! fixed argument vector: the TUI never builds a shell command line.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::mode::Mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Knowledge,
    Maps,
    Communications,
    Networking,
    Radio,
    Security,
    Productivity,
    Entertainment,
    System,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Knowledge,
        Category::Maps,
        Category::Communications,
        Category::Networking,
        Category::Radio,
        Category::Security,
        Category::Productivity,
        Category::Entertainment,
        Category::System,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Knowledge => "KNOWLEDGE & DOCUMENTATION",
            Category::Maps => "MAPS & NAVIGATION",
            Category::Communications => "COMMUNICATIONS",
            Category::Networking => "NETWORKING & MESH",
            Category::Radio => "RADIO & SDR",
            Category::Security => "SECURITY & CRYPTOGRAPHY",
            Category::Productivity => "PRODUCTIVITY",
            Category::Entertainment => "ENTERTAINMENT",
            Category::System => "SYSTEM MANAGEMENT",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub category: Category,
    pub description: String,
    /// Program and arguments. `command[0]` should be an absolute path.
    pub command: Vec<String>,
    /// Alternative argv used in Ultra-Stable mode (e.g. an editor's restricted mode).
    #[serde(default)]
    pub restricted_command: Option<Vec<String>>,
    /// Least-capable mode in which the entry is offered.
    #[serde(default)]
    pub min_mode: Mode,
    /// Wait for a key press after the program exits (for programs that print and exit).
    #[serde(default)]
    pub pause_after_exit: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready(PathBuf),
    Missing(String),
}

impl AppEntry {
    /// The argv to run in `mode`, or `None` if the mode may not use this entry.
    pub fn command_for(&self, mode: Mode) -> Option<&[String]> {
        if !mode.allows(self.min_mode) {
            return None;
        }
        match (&self.restricted_command, mode) {
            (Some(r), Mode::UltraStable) => Some(r),
            _ => Some(&self.command),
        }
    }

    pub fn availability(&self, mode: Mode) -> Availability {
        let Some(prog) = self.command_for(mode).and_then(|c| c.first()) else {
            return Availability::Missing("not permitted in this mode".into());
        };
        match resolve_program(prog) {
            Some(p) => Availability::Ready(p),
            None => Availability::Missing(prog.clone()),
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        [&self.name, &self.description, &self.id]
            .into_iter()
            .chain(self.keywords.iter())
            .any(|s| s.to_lowercase().contains(&q))
    }
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("cannot read catalog {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid catalog {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid catalog entry '{id}': {reason}")]
    Invalid { id: String, reason: String },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(default)]
    pub apps: Vec<AppEntry>,
}

impl Catalog {
    pub fn parse(text: &str, origin: &Path) -> Result<Catalog, CatalogError> {
        let mut cat: Catalog = toml::from_str(text).map_err(|source| CatalogError::Parse {
            path: origin.to_path_buf(),
            source,
        })?;
        cat.validate()?;
        cat.apps
            .sort_by_key(|a| (a.category, a.name.to_lowercase()));
        Ok(cat)
    }

    pub fn load(path: &Path) -> Result<Catalog, CatalogError> {
        let text = std::fs::read_to_string(path).map_err(|source| CatalogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    fn validate(&self) -> Result<(), CatalogError> {
        let mut seen = std::collections::HashSet::new();
        for app in &self.apps {
            let bad = |reason: &str| CatalogError::Invalid {
                id: app.id.clone(),
                reason: reason.into(),
            };
            if app.id.is_empty()
                || !app
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(bad("id must be non-empty [a-z0-9-]"));
            }
            if !seen.insert(app.id.as_str()) {
                return Err(bad("duplicate id"));
            }
            for argv in std::iter::once(&app.command).chain(app.restricted_command.iter()) {
                match argv.first() {
                    None => return Err(bad("command is empty")),
                    Some(p) if p.is_empty() => return Err(bad("program path is empty")),
                    _ => {}
                }
                if argv.iter().any(|a| a.contains('\0')) {
                    return Err(bad("argument contains NUL"));
                }
            }
        }
        Ok(())
    }

    pub fn in_category(
        &self,
        cat: Category,
        mode: Mode,
    ) -> impl Iterator<Item = (usize, &AppEntry)> {
        self.apps
            .iter()
            .enumerate()
            .filter(move |(_, a)| a.category == cat && mode.allows(a.min_mode))
    }
}

/// Resolve a program to an executable file: absolute/relative paths are
/// checked directly, bare names are searched in `$PATH`.
pub fn resolve_program(prog: &str) -> Option<PathBuf> {
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return is_executable(&p).then_some(p);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(prog))
            .find(|p| is_executable(p))
    })
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[apps]]
id = "nano"
name = "Nano"
category = "productivity"
description = "Simple editor"
command = ["/bin/sh", "-c", "true"]
restricted_command = ["/bin/sh", "-r"]

[[apps]]
id = "shell-tool"
name = "Admin tool"
category = "system"
description = "Advanced only"
command = ["/nonexistent/admin"]
min_mode = "advanced"
keywords = ["sudo"]
"#;

    fn parse(t: &str) -> Result<Catalog, CatalogError> {
        Catalog::parse(t, Path::new("test.toml"))
    }

    #[test]
    fn parses_and_filters_by_mode() {
        let c = parse(SAMPLE).expect("valid");
        assert_eq!(c.apps.len(), 2);
        assert_eq!(
            c.in_category(Category::System, Mode::UltraStable).count(),
            0
        );
        assert_eq!(c.in_category(Category::System, Mode::Advanced).count(), 1);
        let nano = &c.apps[0];
        assert_eq!(
            nano.command_for(Mode::UltraStable).expect("allowed")[1],
            "-r"
        );
        assert_eq!(nano.command_for(Mode::Stable).expect("allowed")[1], "-c");
        let admin = &c.apps[1];
        assert!(admin.command_for(Mode::Stable).is_none());
        assert_eq!(
            admin.availability(Mode::Advanced),
            Availability::Missing("/nonexistent/admin".into())
        );
        assert!(matches!(
            nano.availability(Mode::Stable),
            Availability::Ready(_)
        ));
        assert!(admin.matches("SUDO"));
        assert!(!admin.matches("nano"));
    }

    #[test]
    fn rejects_invalid_entries() {
        let empty_cmd = SAMPLE.replace(r#"command = ["/nonexistent/admin"]"#, "command = []");
        assert!(matches!(
            parse(&empty_cmd),
            Err(CatalogError::Invalid { .. })
        ));
        let bad_id = SAMPLE.replace(r#"id = "nano""#, r#"id = "Nano Editor""#);
        assert!(matches!(parse(&bad_id), Err(CatalogError::Invalid { .. })));
        let dup = SAMPLE.replace(r#"id = "shell-tool""#, r#"id = "nano""#);
        assert!(matches!(parse(&dup), Err(CatalogError::Invalid { .. })));
        let unknown_field = format!("{SAMPLE}shell = true\n");
        assert!(matches!(
            parse(&unknown_field),
            Err(CatalogError::Parse { .. })
        ));
        let bad_mode = SAMPLE.replace(r#"min_mode = "advanced""#, r#"min_mode = "root""#);
        assert!(matches!(parse(&bad_mode), Err(CatalogError::Parse { .. })));
        assert!(parse("").expect("empty ok").apps.is_empty());
    }

    #[test]
    fn resolves_programs() {
        assert!(resolve_program("/bin/sh").is_some() || resolve_program("sh").is_some());
        assert!(resolve_program("/etc/passwd").is_none()); // not executable
        assert!(resolve_program("definitely-not-a-program-xyz").is_none());
    }
}
