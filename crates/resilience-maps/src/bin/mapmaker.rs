//! `resilience-mapmaker` — builds `resilience-1` map packs (ADR 0016 §2).
//! For build machines only: the Nix derivations of `nix/maps` run it
//! between osmium and tippecanoe. It is not installed in the system.
//!
//! ```text
//! resilience-mapmaker classify --poly COVERAGE.poly --rows ROWS.jsonl
//!                              [--output FEATURES.geojsonseq] [INPUT|-]
//!     osmium's GeoJSON sequence (a file, or standard input) to the
//!     features for tippecanoe (standard output by default) and the rows
//!     for finish.
//! resilience-mapmaker finish --tiles TIPPECANOE.mbtiles --rows ROWS.jsonl
//!                            --poly COVERAGE.poly --output PACK.mbtiles
//!                            --name NAME --data-date YYYY-MM-DD
//!                            --source TEXT --coverage TEXT
//!                            [--description TEXT] [--license SPDX]
//!                            [--attribution TEXT] [--tools TEXT]
//!     writes the pack (which must not exist) and checks it with the
//!     viewer's reader.
//! ```
//!
//! A summary goes to standard error. Exit status: 0 success, 1 failure,
//! 64 usage error.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use resilience_maps::mapmaker::{classify, finish, poly};

const USAGE: &str = "\
Usage:
  resilience-mapmaker classify --poly FILE --rows FILE [--output FILE] [INPUT|-]
  resilience-mapmaker finish --tiles FILE --rows FILE --poly FILE --output FILE
      --name NAME --data-date YYYY-MM-DD --source TEXT --coverage TEXT
      [--description TEXT] [--license SPDX] [--attribution TEXT] [--tools TEXT]";

const EXIT_USAGE: u8 = 64;

/// `--key value` pairs and positional arguments. Each key at most once.
fn parse(
    args: &[String],
    keys: &[&str],
) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let mut opts = BTreeMap::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if let Some(k) = a.strip_prefix("--") {
            if !keys.contains(&k) {
                return Err(format!("unknown option --{k}"));
            }
            let v = it.next().ok_or(format!("--{k} needs a value"))?;
            if opts.insert(k.to_string(), v.clone()).is_some() {
                return Err(format!("--{k} given twice"));
            }
        } else {
            rest.push(a.clone());
        }
    }
    Ok((opts, rest))
}

fn need<'a>(o: &'a BTreeMap<String, String>, k: &str) -> Result<&'a str, String> {
    o.get(k)
        .map(String::as_str)
        .ok_or(format!("--{k} is required"))
}

fn run_classify(args: &[String]) -> Result<(), String> {
    let (o, rest) = parse(args, &["poly", "rows", "output"])?;
    let text = std::fs::read_to_string(need(&o, "poly")?).map_err(|e| format!("--poly: {e}"))?;
    let rings = poly::parse(&text).map_err(|e| format!("--poly: {e}"))?;
    let opts = classify::Options {
        max_abs_lat: poly::max_abs_lat(&rings),
    };
    let rows =
        BufWriter::new(File::create_new(need(&o, "rows")?).map_err(|e| format!("--rows: {e}"))?);
    let features: Box<dyn Write> = match o.get("output") {
        Some(p) => Box::new(BufWriter::new(
            File::create_new(p).map_err(|e| format!("--output: {e}"))?,
        )),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    };
    let stats = match rest.as_slice() {
        [] => classify::run(io::stdin().lock(), features, rows, &opts),
        [p] if p == "-" => classify::run(io::stdin().lock(), features, rows, &opts),
        [p] => {
            let f = File::open(p).map_err(|e| format!("{p}: {e}"))?;
            classify::run(BufReader::with_capacity(1 << 20, f), features, rows, &opts)
        }
        _ => return Err("one input at most".into()),
    }
    .map_err(|e| format!("classify: {e}"))?;
    eprint!(
        "classify: minimum zooms for latitudes up to {:.2}\n{}",
        opts.max_abs_lat,
        stats.report()
    );
    Ok(())
}

fn run_finish(args: &[String]) -> Result<(), String> {
    let (o, rest) = parse(
        args,
        &[
            "tiles",
            "rows",
            "poly",
            "output",
            "name",
            "data-date",
            "source",
            "coverage",
            "description",
            "license",
            "attribution",
            "tools",
        ],
    )?;
    if !rest.is_empty() {
        return Err("finish takes no positional arguments".into());
    }
    let mut builder = format!("resilience-mapmaker {}", resilience_core::VERSION);
    if let Some(t) = o.get("tools") {
        builder.push_str(&format!(" ({t})"));
    }
    let d = finish::Describe {
        name: need(&o, "name")?.into(),
        description: o.get("description").cloned(),
        data_date: need(&o, "data-date")?.into(),
        source: need(&o, "source")?.into(),
        coverage: need(&o, "coverage")?.into(),
        license: o.get("license").cloned().unwrap_or("ODbL-1.0".into()),
        attribution: o
            .get("attribution")
            .cloned()
            .unwrap_or("(c) OpenStreetMap contributors".into()),
        builder,
    };
    let inputs = finish::Inputs {
        tiles: Path::new(need(&o, "tiles")?),
        rows: Path::new(need(&o, "rows")?),
        poly: Path::new(need(&o, "poly")?),
    };
    let stats = finish::run(&inputs, &d, Path::new(need(&o, "output")?))
        .map_err(|e| format!("finish: {e}"))?;
    eprint!("{}", stats.report());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("classify") => run_classify(&args[1..]),
        Some("finish") => run_finish(&args[1..]),
        Some("--version") => {
            println!("resilience-mapmaker {}", resilience_core::VERSION);
            return ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}
