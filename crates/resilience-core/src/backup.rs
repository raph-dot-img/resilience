//! Encrypted backups of the home folder (ADR 0008).
//!
//! A backup is `tar | zstd | age` written to `Resilience-backups/` on a
//! removable volume, with a `sha256sum`-format checksum file next to it. No
//! cryptography of our own: `age` encrypts (passphrase mode in normal use;
//! it reads the passphrase from the terminal itself, so it never passes
//! through this code) and `sha256sum` computes the checksum.
//!
//! Safety rules:
//! - a backup is written as `*.partial` and renamed only when complete, so a
//!   half-written file never looks like a backup;
//! - restoring never overwrites anything: files go into a new folder
//!   `~/Restored/backup-<date>`; a failed restore leaves the folder marked
//!   `-incomplete` (or removes it when empty);
//! - a copy whose checksum does not match is refused for restore.
//!
//! User-data restore is deliberately separate from system rollback
//! (ORIGINAL-SPEC §20): nothing here touches the system.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;

/// Folder on the removable volume that holds the backups.
pub const BACKUP_DIR: &str = "Resilience-backups";
/// Folder in the home directory that receives restored files.
pub const RESTORED_DIR: &str = "Restored";
/// Extension of a complete backup.
pub const EXTENSION: &str = ".tar.zst.age";
/// Extension of the checksum file (appended to the backup's name).
pub const CHECKSUM_EXTENSION: &str = ".sha256";
/// Paths (relative to home) that are not backed up.
pub const EXCLUDED: [&str; 1] = [".cache"];

/// Absolute paths of the programs used (from `/etc/resilience/system.toml`).
#[derive(Debug, Clone)]
pub struct Tools {
    pub tar: String,
    pub zstd: String,
    pub age: String,
    pub sha256sum: String,
}

/// `2026-11-14T10:30:00Z` → `20261114-103000`.
pub fn compact_stamp(rfc3339: &str) -> String {
    let digits: String = rfc3339.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 14 {
        format!("{}-{}", &digits[..8], &digits[8..14])
    } else {
        digits
    }
}

/// File name of a backup made at `rfc3339`.
pub fn archive_name(rfc3339: &str) -> String {
    format!("home-{}{EXTENSION}", compact_stamp(rfc3339))
}

/// `home-20261114-103000.tar.zst.age` → `20261114-103000`.
pub fn parse_archive_name(name: &str) -> Option<String> {
    let stamp = name.strip_prefix("home-")?.strip_suffix(EXTENSION)?;
    let b = stamp.as_bytes();
    let ok = b.len() >= 15
        && b[8] == b'-'
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15..].iter().all(|c| c.is_ascii_digit() || *c == b'-');
    ok.then(|| stamp.to_string())
}

/// `20261114-103000` → `2026-11-14 10:30 UTC`.
pub fn human_stamp(stamp: &str) -> String {
    let b = stamp.as_bytes();
    if b.len() < 15 {
        return stamp.to_string();
    }
    format!(
        "{}-{}-{} {}:{}:{} UTC",
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[9..11],
        &stamp[11..13],
        &stamp[13..15]
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub path: PathBuf,
    pub name: String,
    pub stamp: String,
    pub size: u64,
    pub has_checksum: bool,
}

/// Complete backups in `<mount>/Resilience-backups`, newest first.
/// Partial files and anything else in the folder are ignored.
pub fn list_backups(mount: &Path) -> Vec<BackupFile> {
    let dir = mount.join(BACKUP_DIR);
    let mut out: Vec<BackupFile> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let stamp = parse_archive_name(&name)?;
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            Some(BackupFile {
                has_checksum: dir.join(format!("{name}{CHECKSUM_EXTENSION}")).is_file(),
                path: e.path(),
                size: meta.len(),
                name,
                stamp,
            })
        })
        .collect();
    out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
    out
}

/// One line in `sha256sum` format, so `sha256sum -c` works anywhere.
pub fn checksum_line(hex: &str, name: &str) -> String {
    format!("{hex}  {name}\n")
}

/// The digest recorded for `name` in a checksum file.
pub fn parse_checksum(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (hex, rest) = l.split_once(' ')?;
        let file = rest.trim_start_matches([' ', '*']);
        (file == name && is_sha256(hex)).then(|| hex.to_ascii_lowercase())
    })
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Bytes in `home` that a backup would read: regular files, not following
/// symbolic links, staying on one file system, skipping [`EXCLUDED`].
pub fn estimate_size(home: &Path) -> u64 {
    fn walk(dir: &Path, dev: u64, depth: usize, total: &mut u64) {
        if depth > 64 {
            return;
        }
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(m) = e.path().symlink_metadata() else {
                continue;
            };
            if m.is_dir() && m.dev() == dev {
                walk(&e.path(), dev, depth + 1, total);
            } else if m.is_file() {
                *total += m.len();
            }
        }
    }
    let Ok(m) = home.symlink_metadata() else {
        return 0;
    };
    let mut total = 0;
    for e in fs::read_dir(home).into_iter().flatten().flatten() {
        if EXCLUDED.iter().any(|x| e.file_name() == *x) {
            continue;
        }
        let Ok(em) = e.path().symlink_metadata() else {
            continue;
        };
        if em.is_dir() && em.dev() == m.dev() {
            walk(&e.path(), m.dev(), 1, &mut total);
        } else if em.is_file() {
            total += em.len();
        }
    }
    total
}

// ----- pipelines --------------------------------------------------------------

/// One program of a pipeline.
pub struct Stage {
    pub argv: Vec<String>,
    /// Capture stderr (shown only if this stage is the cause of a failure,
    /// or as warnings on success). `false` leaves it on the terminal.
    pub capture_stderr: bool,
}

impl Stage {
    pub fn new(argv: Vec<String>, capture_stderr: bool) -> Stage {
        Stage {
            argv,
            capture_stderr,
        }
    }

    fn program(&self) -> String {
        Path::new(self.argv.first().map(String::as_str).unwrap_or("?"))
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct PipelineError {
    /// Program that caused the failure.
    pub program: String,
    pub status: Option<ExitStatus>,
    /// Its captured error output (empty when it went to the terminal).
    pub stderr: String,
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.status {
            Some(st) => write!(f, "{} failed ({st})", self.program),
            None => write!(f, "{} could not be started", self.program),
        }?;
        let msg = self.stderr.trim();
        if !msg.is_empty() {
            write!(f, ": {msg}")?;
        }
        Ok(())
    }
}

/// Run `stages` connected by pipes, with `stdin` feeding the first and the
/// last writing to `stdout`. Never uses a shell.
///
/// On failure the reported cause is the first stage that failed by itself;
/// stages killed by SIGPIPE only because a later stage stopped reading are
/// not the cause. Returns the captured warnings on success.
pub fn run_pipeline(
    stages: &[Stage],
    stdin: Stdio,
    stdout: Stdio,
) -> Result<String, PipelineError> {
    let mut children: Vec<(Child, Option<thread::JoinHandle<String>>)> = Vec::new();
    let mut next_in = Some(stdin);
    let mut final_out = Some(stdout);
    let kill_all = |children: &mut Vec<(Child, Option<thread::JoinHandle<String>>)>| {
        for (c, _) in children.iter_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
    };
    for (i, s) in stages.iter().enumerate() {
        let last = i + 1 == stages.len();
        let Some((prog, args)) = s.argv.split_first() else {
            kill_all(&mut children);
            return Err(PipelineError {
                program: "?".into(),
                status: None,
                stderr: "empty command".into(),
            });
        };
        let mut cmd = Command::new(prog);
        cmd.args(args)
            .stdin(next_in.take().unwrap_or_else(Stdio::null))
            .stdout(if last {
                final_out.take().unwrap_or_else(Stdio::inherit)
            } else {
                Stdio::piped()
            })
            .stderr(if s.capture_stderr {
                Stdio::piped()
            } else {
                Stdio::inherit()
            });
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                kill_all(&mut children);
                return Err(PipelineError {
                    program: s.program(),
                    status: None,
                    stderr: e.to_string(),
                });
            }
        };
        if !last {
            next_in = child.stdout.take().map(Stdio::from);
        }
        let reader = child.stderr.take().map(|mut err| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = err.by_ref().take(64 * 1024).read_to_end(&mut buf);
                // Drain the rest so the program never blocks on stderr.
                let _ = io::copy(&mut err, &mut io::sink());
                String::from_utf8_lossy(&buf).into_owned()
            })
        });
        children.push((child, reader));
    }
    let mut results = Vec::new();
    for (mut c, reader) in children {
        let status = c.wait().ok();
        let err = reader.and_then(|r| r.join().ok()).unwrap_or_default();
        results.push((status, err));
    }
    let failed = |st: &Option<ExitStatus>| !st.is_some_and(|s| s.success());
    let sigpipe = |st: &Option<ExitStatus>| st.and_then(|s| s.signal()) == Some(libc::SIGPIPE);
    let cause = results
        .iter()
        .position(|(st, _)| failed(st) && !sigpipe(st))
        .or_else(|| results.iter().position(|(st, _)| failed(st)));
    match cause {
        Some(i) => Err(PipelineError {
            program: stages[i].program(),
            status: results[i].0,
            stderr: results[i].1.clone(),
        }),
        None => Ok(results
            .into_iter()
            .map(|(_, e)| e)
            .filter(|e| !e.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

// ----- operations -------------------------------------------------------------

#[derive(Debug)]
pub enum BackupError {
    /// Refused before anything was written.
    Refused(String),
    Io(String, io::Error),
    Pipeline(PipelineError),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::Refused(m) => write!(f, "{m}"),
            BackupError::Io(what, e) => write!(f, "{what}: {e}"),
            BackupError::Pipeline(e) => write!(f, "{e}"),
        }
    }
}

fn io_err(what: impl Into<String>) -> impl FnOnce(io::Error) -> BackupError {
    let what = what.into();
    move |e| BackupError::Io(what, e)
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// SHA-256 of a file, computed by `sha256sum`.
pub fn sha256_of(tools: &Tools, file: &Path) -> Result<String, BackupError> {
    let out = Command::new(&tools.sha256sum)
        .arg("--")
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .map_err(io_err("sha256sum"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let hex = text.split_whitespace().next().unwrap_or_default();
    if !out.status.success() || !is_sha256(hex) {
        return Err(BackupError::Refused(format!(
            "cannot read {}: {}",
            file.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(hex.to_ascii_lowercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    /// tar warnings (for example unreadable files that were skipped).
    pub warnings: String,
}

/// Back up `home` into `<mount>/Resilience-backups/home-<stamp>.tar.zst.age`.
/// `encrypt` are age's arguments: `["--passphrase"]` normally.
pub fn create(
    tools: &Tools,
    home: &Path,
    mount: &Path,
    rfc3339: &str,
    encrypt: &[String],
) -> Result<Created, BackupError> {
    let home = fs::canonicalize(home).map_err(io_err(home.display().to_string()))?;
    let mount = fs::canonicalize(mount).map_err(io_err(mount.display().to_string()))?;
    if mount.starts_with(&home) {
        return Err(BackupError::Refused(
            "the destination is inside your home folder; choose a removable stick".into(),
        ));
    }
    let dir = mount.join(BACKUP_DIR);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(io_err(format!("cannot create {}", dir.display())))?;
    let name = archive_name(rfc3339);
    let path = dir.join(&name);
    if path.exists() {
        return Err(BackupError::Refused(format!(
            "{} already exists; try again in a moment",
            path.display()
        )));
    }
    let partial = dir.join(format!("{name}.partial"));
    let out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)
        .map_err(io_err(format!("cannot write {}", partial.display())))?;
    let mut tar = vec![
        tools.tar.clone(),
        "--create".into(),
        "--file=-".into(),
        "--one-file-system".into(),
        "--ignore-failed-read".into(),
        "--warning=no-file-changed".into(),
        format!("--directory={}", home.display()),
    ];
    tar.extend(EXCLUDED.iter().map(|x| format!("--exclude=./{x}")));
    tar.push(".".into());
    let zstd = vec![tools.zstd.clone(), "-q".into(), "-3".into(), "-c".into()];
    let mut age = vec![tools.age.clone()];
    age.extend(encrypt.iter().cloned());
    let result = run_pipeline(
        &[
            Stage::new(tar, true),
            Stage::new(zstd, true),
            Stage::new(age, false),
        ],
        Stdio::null(),
        Stdio::from(out.try_clone().map_err(io_err("cannot write the backup"))?),
    );
    let finish = || -> Result<(String, u64), BackupError> {
        let warnings = result.map_err(BackupError::Pipeline)?;
        out.sync_all().map_err(io_err("cannot write the backup"))?;
        let size = out.metadata().map(|m| m.len()).unwrap_or(0);
        fs::rename(&partial, &path).map_err(io_err("cannot finish the backup"))?;
        sync_dir(&dir);
        Ok((warnings, size))
    };
    let (warnings, size) = match finish() {
        Ok(v) => v,
        Err(e) => {
            let _ = fs::remove_file(&partial);
            sync_dir(&dir);
            return Err(e);
        }
    };
    let sha256 = sha256_of(tools, &path)?;
    let sum_path = dir.join(format!("{name}{CHECKSUM_EXTENSION}"));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&sum_path)
        .map_err(io_err(format!("cannot write {}", sum_path.display())))?;
    f.write_all(checksum_line(&sha256, &name).as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(io_err(format!("cannot write {}", sum_path.display())))?;
    sync_dir(&dir);
    Ok(Created {
        path,
        size,
        sha256,
        warnings,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The file matches its recorded checksum.
    Intact,
    /// The file differs from what was written.
    Damaged { expected: String, actual: String },
    /// No (readable) checksum file: cannot be checked without the passphrase.
    NoChecksum,
}

/// Compare a backup with its checksum file.
pub fn check(tools: &Tools, file: &Path) -> Result<Check, BackupError> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sum_path = file.with_file_name(format!("{name}{CHECKSUM_EXTENSION}"));
    let Some(expected) = fs::read_to_string(&sum_path)
        .ok()
        .and_then(|t| parse_checksum(&t, &name))
    else {
        return Ok(Check::NoChecksum);
    };
    let actual = sha256_of(tools, file)?;
    Ok(if actual == expected {
        Check::Intact
    } else {
        Check::Damaged { expected, actual }
    })
}

/// A new, empty folder `~/Restored/backup-<stamp>[-N]` (created 0700).
pub fn new_restore_dir(home: &Path, stamp: &str) -> io::Result<PathBuf> {
    let parent = home.join(RESTORED_DIR);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&parent)?;
    for n in 1..100 {
        let name = if n == 1 {
            format!("backup-{stamp}")
        } else {
            format!("backup-{stamp}-{n}")
        };
        let dir = parent.join(name);
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many restored copies of this backup",
    ))
}

#[derive(Debug)]
pub enum RestoreError {
    /// Nothing was extracted.
    Refused(String),
    /// Extraction failed; `partial` holds what was extracted, if anything.
    Failed {
        error: BackupError,
        partial: Option<PathBuf>,
    },
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreError::Refused(m) => write!(f, "{m}"),
            RestoreError::Failed { error, .. } => write!(f, "{error}"),
        }
    }
}

/// Restore `file` into a new folder under `~/Restored`. `decrypt` are extra
/// age arguments (none for a passphrase; age asks on the terminal).
pub fn restore(
    tools: &Tools,
    file: &Path,
    home: &Path,
    decrypt: &[String],
) -> Result<PathBuf, RestoreError> {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = parse_archive_name(&name)
        .ok_or_else(|| RestoreError::Refused(format!("{name} is not a Resilience backup")))?;
    match check(tools, file).map_err(|e| RestoreError::Refused(e.to_string()))? {
        Check::Damaged { .. } => {
            return Err(RestoreError::Refused(
                "this copy is damaged (it differs from what was written); \
                 nothing was restored. Use another copy."
                    .into(),
            ));
        }
        Check::Intact | Check::NoChecksum => {}
    }
    let dest = new_restore_dir(home, &stamp)
        .map_err(|e| RestoreError::Refused(format!("cannot create the destination: {e}")))?;
    let mut age = vec![tools.age.clone(), "--decrypt".into()];
    age.extend(decrypt.iter().cloned());
    age.push("--".into());
    age.push(file.display().to_string());
    let zstd = vec![tools.zstd.clone(), "-q".into(), "-d".into(), "-c".into()];
    let tar = vec![
        tools.tar.clone(),
        "--extract".into(),
        "--file=-".into(),
        "--no-same-owner".into(),
        "--no-same-permissions".into(),
        "--no-overwrite-dir".into(),
        format!("--directory={}", dest.display()),
    ];
    let result = run_pipeline(
        &[
            Stage::new(age, false),
            Stage::new(zstd, true),
            Stage::new(tar, true),
        ],
        Stdio::inherit(),
        Stdio::null(),
    );
    match result {
        Ok(_) => Ok(dest),
        Err(e) => {
            let empty = fs::read_dir(&dest)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false);
            let partial = if empty {
                let _ = fs::remove_dir(&dest);
                None
            } else {
                let marked = dest.with_file_name(format!(
                    "{}-incomplete",
                    dest.file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default()
                ));
                Some(if fs::rename(&dest, &marked).is_ok() {
                    marked
                } else {
                    dest
                })
            };
            Err(RestoreError::Failed {
                error: BackupError::Pipeline(e),
                partial,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("resilience-bk-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("mkdir");
        d
    }

    #[test]
    fn names_and_stamps() {
        assert_eq!(compact_stamp("2026-11-14T10:30:05Z"), "20261114-103005");
        let n = archive_name("2026-11-14T10:30:05Z");
        assert_eq!(n, "home-20261114-103005.tar.zst.age");
        assert_eq!(parse_archive_name(&n).as_deref(), Some("20261114-103005"));
        assert_eq!(
            parse_archive_name("home-20261114-103005.tar.zst.age.partial"),
            None
        );
        assert_eq!(parse_archive_name("home-2026.tar.zst.age"), None);
        assert_eq!(parse_archive_name("home-20261114-10300x.tar.zst.age"), None);
        assert_eq!(
            parse_archive_name("other-20261114-103005.tar.zst.age"),
            None
        );
        assert_eq!(human_stamp("20261114-103005"), "2026-11-14 10:30:05 UTC");
    }

    #[test]
    fn checksum_files() {
        let h = "a".repeat(64);
        let line = checksum_line(&h, "x.age");
        assert_eq!(line, format!("{h}  x.age\n"));
        assert_eq!(parse_checksum(&line, "x.age"), Some(h.clone()));
        assert_eq!(
            parse_checksum(&format!("{h} *x.age\n"), "x.age"),
            Some(h.clone())
        );
        assert_eq!(parse_checksum(&line, "y.age"), None);
        assert_eq!(parse_checksum("abc  x.age\n", "x.age"), None);
    }

    #[test]
    fn lists_only_complete_backups_newest_first() {
        let d = tmp("list");
        let b = d.join(BACKUP_DIR);
        fs::create_dir_all(&b).expect("mkdir");
        for n in [
            "home-20261101-080000.tar.zst.age",
            "home-20261114-103000.tar.zst.age",
            "home-20261115-103000.tar.zst.age.partial",
            "notes.txt",
        ] {
            fs::write(b.join(n), "x").expect("write");
        }
        fs::write(b.join("home-20261101-080000.tar.zst.age.sha256"), "x").expect("write");
        let l = list_backups(&d);
        assert_eq!(
            l.iter().map(|f| f.stamp.as_str()).collect::<Vec<_>>(),
            vec!["20261114-103000", "20261101-080000"]
        );
        assert!(!l[0].has_checksum && l[1].has_checksum);
        assert!(list_backups(&d.join("missing")).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn size_estimate_skips_cache_and_symlinks() {
        let d = tmp("size");
        fs::create_dir_all(d.join("Notes")).expect("mkdir");
        fs::create_dir_all(d.join(".cache")).expect("mkdir");
        fs::write(d.join("Notes/a"), vec![0u8; 1000]).expect("write");
        fs::write(d.join("b"), vec![0u8; 24]).expect("write");
        fs::write(d.join(".cache/big"), vec![0u8; 5000]).expect("write");
        std::os::unix::fs::symlink("/usr", d.join("link")).expect("symlink");
        assert_eq!(estimate_size(&d), 1024);
        let _ = fs::remove_dir_all(&d);
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".into(), "-c".into(), script.into()]
    }

    #[test]
    fn pipeline_reports_the_real_cause() {
        // Data flows through every stage.
        let d = tmp("pipe");
        let out = fs::File::create(d.join("out")).expect("create");
        run_pipeline(
            &[
                Stage::new(sh("printf hello"), true),
                Stage::new(vec!["tr".into(), "a-z".into(), "A-Z".into()], true),
            ],
            Stdio::null(),
            Stdio::from(out),
        )
        .expect("pipeline");
        assert_eq!(fs::read_to_string(d.join("out")).expect("read"), "HELLO");
        // A downstream stage stops reading: the producer's SIGPIPE is not
        // the cause, the consumer's failure is.
        let e = run_pipeline(
            &[
                Stage::new(sh("yes"), true),
                Stage::new(sh("head -c 10 >/dev/null; echo broken >&2; exit 3"), true),
            ],
            Stdio::null(),
            Stdio::null(),
        )
        .expect_err("fails");
        assert_eq!(e.program, "sh");
        assert_eq!(e.status.and_then(|s| s.code()), Some(3));
        assert_eq!(e.stderr.trim(), "broken");
        // An upstream failure is reported even though later stages fail too.
        let e = run_pipeline(
            &[
                Stage::new(sh("echo bad >&2; exit 5"), true),
                Stage::new(sh("cat >/dev/null; exit 7"), true),
            ],
            Stdio::null(),
            Stdio::null(),
        )
        .expect_err("fails");
        assert_eq!(e.status.and_then(|s| s.code()), Some(5));
        let e = run_pipeline(
            &[Stage::new(vec!["/nonexistent/x".into()], true)],
            Stdio::null(),
            Stdio::null(),
        )
        .expect_err("fails");
        assert!(e.status.is_none());
        let _ = fs::remove_dir_all(&d);
    }

    /// age and age-keygen are needed (provided in the Nix build); the test
    /// is skipped with a message on hosts without them.
    fn tools() -> Option<(Tools, String)> {
        let find = |p: &str| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|d| d.join(p))
                    .find(|f| f.is_file())
                    .map(|f| f.display().to_string())
            })
        };
        let t = Tools {
            tar: find("tar")?,
            zstd: find("zstd")?,
            age: find("age")?,
            sha256sum: find("sha256sum")?,
        };
        Some((t, find("age-keygen")?))
    }

    #[test]
    fn backup_check_and_restore_round_trip() {
        let Some((t, keygen)) = tools() else {
            eprintln!("SKIPPED: age, age-keygen, tar, zstd or sha256sum not in PATH");
            return;
        };
        let d = tmp("round");
        let home = d.join("home");
        let stick = d.join("stick");
        fs::create_dir_all(home.join("Notes")).expect("mkdir");
        fs::create_dir_all(home.join(".cache")).expect("mkdir");
        fs::create_dir_all(&stick).expect("mkdir");
        fs::write(home.join("Notes/first note.md"), "SECRET-MARKER-123\n").expect("write");
        fs::write(home.join("été.txt"), "accents\n").expect("write");
        fs::write(home.join(".cache/junk"), "cache\n").expect("write");
        let key = d.join("key");
        let kg = Command::new(&keygen)
            .arg("-o")
            .arg(&key)
            .output()
            .expect("age-keygen");
        assert!(kg.status.success());
        let keytext = fs::read_to_string(&key).expect("key");
        let recipient = keytext
            .lines()
            .find_map(|l| l.strip_prefix("# public key: "))
            .expect("public key")
            .to_string();
        let enc = ["-r".to_string(), recipient];
        let dec = ["-i".to_string(), key.display().to_string()];

        // Refuses a destination inside home.
        assert!(matches!(
            create(&t, &home, &home.join("Notes"), "2026-11-14T10:30:00Z", &enc),
            Err(BackupError::Refused(_))
        ));

        let c = create(&t, &home, &stick, "2026-11-14T10:30:00Z", &enc).expect("create");
        assert!(
            c.path
                .ends_with("Resilience-backups/home-20261114-103000.tar.zst.age")
        );
        let bytes = fs::read(&c.path).expect("read");
        assert!(bytes.starts_with(b"age-encryption.org/v1"));
        assert!(!String::from_utf8_lossy(&bytes).contains("SECRET-MARKER"));
        assert_eq!(c.size, bytes.len() as u64);
        let sums = fs::read_dir(stick.join(BACKUP_DIR)).expect("dir").count();
        assert_eq!(sums, 2, "archive and checksum, no partial file left");
        assert_eq!(check(&t, &c.path).expect("check"), Check::Intact);
        // Never overwrites an existing backup.
        assert!(create(&t, &home, &stick, "2026-11-14T10:30:00Z", &enc).is_err());

        // Restore into a new folder; .cache was not saved.
        let r1 = restore(&t, &c.path, &home, &dec).expect("restore");
        assert_eq!(r1, home.join("Restored/backup-20261114-103000"));
        assert_eq!(
            fs::read_to_string(r1.join("Notes/first note.md")).expect("read"),
            "SECRET-MARKER-123\n"
        );
        assert!(r1.join("été.txt").is_file());
        assert!(!r1.join(".cache").exists());
        let mode = fs::metadata(&r1).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // A second restore never touches the first.
        let r2 = restore(&t, &c.path, &home, &dec).expect("restore");
        assert_eq!(r2, home.join("Restored/backup-20261114-103000-2"));

        // Wrong key: nothing is restored and no folder is left behind.
        let other = d.join("other");
        assert!(
            Command::new(&keygen)
                .arg("-o")
                .arg(&other)
                .status()
                .expect("keygen")
                .success()
        );
        let wrong = ["-i".to_string(), other.display().to_string()];
        match restore(&t, &c.path, &home, &wrong) {
            Err(RestoreError::Failed {
                error,
                partial: None,
            }) => {
                assert!(error.to_string().starts_with("age failed"), "{error}");
            }
            other => panic!("expected a failed restore, got {other:?}"),
        }
        assert!(!home.join("Restored/backup-20261114-103000-3").exists());

        // A damaged copy is detected and refused.
        let mut b = bytes.clone();
        let mid = b.len() / 2;
        b[mid] ^= 0x01;
        fs::write(&c.path, &b).expect("write");
        assert!(matches!(
            check(&t, &c.path).expect("check"),
            Check::Damaged { .. }
        ));
        assert!(matches!(
            restore(&t, &c.path, &home, &dec),
            Err(RestoreError::Refused(_))
        ));
        // Without its checksum, age's authentication still catches it.
        fs::remove_file(c.path.with_file_name(format!(
            "{}.sha256",
            c.path.file_name().expect("name").to_string_lossy()
        )))
        .expect("rm");
        assert_eq!(check(&t, &c.path).expect("check"), Check::NoChecksum);
        match restore(&t, &c.path, &home, &dec) {
            Err(RestoreError::Failed { .. }) => {}
            other => panic!("expected a failed restore, got {other:?}"),
        }
        let left: Vec<_> = fs::read_dir(home.join("Restored"))
            .expect("dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            left.iter().all(|n| n == "backup-20261114-103000"
                || n == "backup-20261114-103000-2"
                || n.ends_with("-incomplete")),
            "{left:?}"
        );
        let _ = fs::remove_dir_all(&d);
    }
}
