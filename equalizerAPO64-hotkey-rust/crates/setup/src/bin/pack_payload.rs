//! Packaging tool for the single-file installer.
//!
//! Not shipped: `build.bat` uses it to append the `files\` payload to
//! `Setup.exe`, producing one self-extracting executable.

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::process::ExitCode;

use eapo_wizard::payload;

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  PackPayload build <files-dir> <setup-exe> <out-exe>\n  PackPayload list <exe>\n  PackPayload extract <exe> <dir>"
    );
    ExitCode::from(2)
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("build") if args.len() == 4 => {
            let files = PathBuf::from(&args[1]);
            let setup = PathBuf::from(&args[2]);
            let out = PathBuf::from(&args[3]);
            if !files.join("EqualizerAPO.dll").is_file() {
                return Err(format!("{} does not look like a payload directory", files.display()));
            }
            if out.exists() {
                fs::remove_file(&out).map_err(|error| error.to_string())?;
            }
            fs::copy(&setup, &out).map_err(|error| error.to_string())?;
            let mut file = OpenOptions::new()
                .append(true)
                .open(&out)
                .map_err(|error| error.to_string())?;
            let stats = payload::append_archive(&mut file, &files).map_err(|error| error.to_string())?;
            let total = fs::metadata(&out).map_err(|error| error.to_string())?.len();
            println!(
                "{}: {} files, {} payload bytes, {} index bytes, {} total bytes",
                out.display(),
                stats.files,
                stats.payload_bytes,
                stats.index_bytes,
                total
            );
            Ok(())
        }
        Some("list") if args.len() == 2 => {
            let exe = PathBuf::from(&args[1]);
            let entries = payload::read_entries(&exe).map_err(|error| error.to_string())?;
            match entries {
                Some(entries) => {
                    for entry in entries {
                        println!("{}\t{}\t{}", entry.raw_len, entry.comp_len, entry.path);
                    }
                }
                None => println!("no payload in {}", exe.display()),
            }
            Ok(())
        }
        Some("extract") if args.len() == 3 => {
            let exe = PathBuf::from(&args[1]);
            let dest = PathBuf::from(&args[2]);
            let count = payload::extract(&exe, &dest).map_err(|error| error.to_string())?;
            match count {
                Some(count) => {
                    println!("extracted {count} files to {}", dest.display());
                    Ok(())
                }
                None => Err(format!("no payload in {}", exe.display())),
            }
        }
        _ => Err("invalid arguments".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return usage();
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("PackPayload: {message}");
            ExitCode::FAILURE
        }
    }
}
