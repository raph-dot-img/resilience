//! `resilience-maps` — the offline map viewer of Resilience OS (ADR 0016).
//!
//! Runs as the user, in the foreground, never with privileges. Map files
//! are untrusted; this process lowers its own address-space limit to
//! 1 GiB before it opens one.
//!
//! ```text
//! resilience-maps info [--check] FILE   what a map pack says about itself
//!                                       (format: src/info.rs); --check also
//!                                       runs SQLite's quick_check
//! ```
//!
//! Exit status: 0 success; 1 the file cannot be read (missing, permission)
//! or another error; 2 the file is refused (not a map pack, damaged or
//! unsafe); 64 usage error.

use std::path::Path;
use std::process::ExitCode;

use resilience_maps::info;
use resilience_maps::pack::Pack;
use resilience_maps::rlimit::{ADDRESS_SPACE_LIMIT, limit_address_space};
use resilience_maps::text::clean;

const EXIT_ERROR: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_USAGE: u8 = 64;

const USAGE: &str = "\
Usage:
  resilience-maps info [--check] FILE   Show what a map pack says about itself";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    match argv.as_slice() {
        ["info", file] => run_info(Path::new(file), false),
        ["info", "--check", file] => run_info(Path::new(file), true),
        ["--version"] => {
            println!("resilience-maps {}", resilience_core::VERSION);
            ExitCode::SUCCESS
        }
        ["--help"] | ["-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn run_info(file: &Path, check: bool) -> ExitCode {
    if let Err(e) = limit_address_space(ADDRESS_SPACE_LIMIT) {
        eprintln!("Error: cannot limit memory use: {e}");
        return ExitCode::from(EXIT_ERROR);
    }
    let name = clean(&file.to_string_lossy());
    match Pack::open(file) {
        Ok(pack) => {
            let result = check.then(|| pack.quick_check());
            print!("{}", info::render(&pack, result.as_ref()));
            match result {
                Some(Err(e)) => {
                    eprintln!("Refused: {name}: {e}");
                    ExitCode::from(EXIT_REFUSED)
                }
                _ => ExitCode::SUCCESS,
            }
        }
        Err(e) if e.is_refusal() => {
            eprintln!("Refused: {name}: {e}");
            ExitCode::from(EXIT_REFUSED)
        }
        Err(e) => {
            eprintln!("Error: {name}: {e}");
            ExitCode::from(EXIT_ERROR)
        }
    }
}
