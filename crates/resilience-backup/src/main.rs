//! `resilience-backup` — encrypted backup and restore of your home folder.
//!
//! Runs as the user, never with privileges (ADR 0008). The interface starts
//! it in the foreground so that `age` can ask for the passphrase on the
//! terminal; it can also be used from a shell.
//!
//! ```text
//! resilience-backup create MOUNTPOINT   save ~ to MOUNTPOINT/Resilience-backups
//! resilience-backup check FILE          compare a backup with its checksum
//! resilience-backup restore FILE        restore into ~/Restored/backup-<date>
//! ```

use std::path::Path;
use std::process::ExitCode;

use resilience_core::backup::{self, BackupError, Check, RestoreError};
use resilience_core::config::{self, SystemConfig};
use resilience_core::human_bytes;

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;

const USAGE: &str = "\
Usage:
  resilience-backup create MOUNTPOINT   Save your home folder, encrypted, to a stick
  resilience-backup check FILE          Check that a backup is intact (no passphrase)
  resilience-backup restore FILE        Restore into ~/Restored (never overwrites)";

enum Failure {
    Usage,
    Refused(String),
    Error(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match argv.as_slice() {
        ["create", mount] => tools().and_then(|t| create(&t, Path::new(mount))),
        ["check", file] => tools().and_then(|t| check(&t, Path::new(file))),
        ["restore", file] => tools().and_then(|t| restore(&t, Path::new(file))),
        ["--version"] => {
            println!("resilience-backup {}", resilience_core::VERSION);
            Ok(())
        }
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => Err(Failure::Usage),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(EXIT_USAGE)
        }
        Err(Failure::Refused(m)) => {
            eprintln!("\n{m}");
            ExitCode::from(EXIT_REFUSED)
        }
        Err(Failure::Error(m)) => {
            eprintln!("\n{m}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn tools() -> Result<backup::Tools, Failure> {
    SystemConfig::load()
        .map(|(c, _, _)| c.tools.backup_tools())
        .map_err(|e| Failure::Error(format!("Cannot read the system configuration: {e}")))
}

fn create(tools: &backup::Tools, mount: &Path) -> Result<(), Failure> {
    if !mount.is_dir() {
        return Err(Failure::Refused(format!(
            "{} is not available. Is the stick still plugged in?",
            mount.display()
        )));
    }
    let home = config::home_dir();
    println!("BACKUP OF YOUR FILES");
    println!();
    println!("Your home folder will be saved to {}.", mount.display());
    println!(
        "About {} to read (the cache folder is skipped).",
        human_bytes(backup::estimate_size(&home))
    );
    println!();
    println!("Choose a passphrase for this backup. Without it, nobody - not even you -");
    println!("can open the backup. Write it down and keep it apart from the stick.");
    println!("Leave it empty to have a strong passphrase generated and shown to you.");
    println!();
    let now = resilience_core::admin::now_rfc3339();
    match backup::create(tools, &home, mount, &now, &["--passphrase".to_string()]) {
        Ok(c) => {
            if !c.warnings.trim().is_empty() {
                println!("\nSome files could not be read and were skipped:");
                println!("{}", c.warnings.trim_end());
            }
            println!();
            println!("Backup complete: {}", c.path.display());
            println!("Size: {}   SHA-256: {}", human_bytes(c.size), c.sha256);
            println!();
            println!("Eject the stick from the interface before unplugging it.");
            Ok(())
        }
        Err(BackupError::Refused(m)) => Err(Failure::Refused(format!("Refused: {m}."))),
        Err(e) => Err(Failure::Error(format!(
            "The backup failed: {e}\nNo backup file was kept. Your files are unchanged."
        ))),
    }
}

fn check(tools: &backup::Tools, file: &Path) -> Result<(), Failure> {
    println!("Checking {} ...", file.display());
    match backup::check(tools, file) {
        Ok(Check::Intact) => {
            println!("\nThis copy is intact: it is exactly what was written.");
            Ok(())
        }
        Ok(Check::NoChecksum) => {
            println!(
                "\nThis copy has no checksum file, so it cannot be checked without the\n\
                 passphrase. Restoring it will still detect any damage."
            );
            Ok(())
        }
        Ok(Check::Damaged { expected, actual }) => Err(Failure::Refused(format!(
            "This copy is DAMAGED: it differs from what was written.\n\
             expected {expected}\n\
             found    {actual}\n\
             Do not rely on it. Use another copy, and consider replacing the stick."
        ))),
        Err(e) => Err(Failure::Error(format!("The check failed: {e}"))),
    }
}

fn restore(tools: &backup::Tools, file: &Path) -> Result<(), Failure> {
    let home = config::home_dir();
    println!("RESTORE A BACKUP");
    println!();
    println!("Restoring {}", file.display());
    println!(
        "into a new folder in {}.",
        home.join(backup::RESTORED_DIR).display()
    );
    println!("Nothing that exists is changed. Checking the copy first ...");
    println!();
    match backup::restore(tools, file, &home, &[]) {
        Ok(dest) => {
            println!();
            println!("Your files were restored into {}", dest.display());
            println!("Move what you need from there; the folder is yours to keep or delete.");
            Ok(())
        }
        Err(RestoreError::Refused(m)) => Err(Failure::Refused(format!("Refused: {m}"))),
        Err(RestoreError::Failed { error, partial }) => {
            let mut m = format!(
                "The restore failed: {error}\n\
                 The passphrase may be wrong, or the copy may be damaged."
            );
            match partial {
                None => m.push_str("\nNothing was restored. Your files are unchanged."),
                Some(p) => m.push_str(&format!(
                    "\nSome files were extracted into {} and may be incomplete.\n\
                     Your other files are unchanged.",
                    p.display()
                )),
            }
            Err(Failure::Error(m))
        }
    }
}
