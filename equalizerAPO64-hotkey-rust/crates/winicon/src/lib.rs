//! Embeds the original C++ project's application icons into the Rust binaries.
//!
//! A minimal resource script (`1 ICON "<file>"`) is compiled with the Windows
//! SDK's `rc.exe` and the resulting `.res` file is handed to `link.exe` as an
//! extra input, which is exactly how the upstream `.rc` files attach
//! `IDI_ICON1` to each executable.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Embeds `icon` (path relative to the calling crate's manifest directory)
/// as the application icon of the rustc binary target named `bin`.
pub fn embed_icon(bin: &str, icon: &str) {
    println!("cargo:rerun-if-changed={icon}");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon = manifest
        .join(icon)
        .canonicalize()
        .unwrap_or_else(|error| panic!("icon {icon} not found: {error}"));
    let rc = find_rc().expect(
        "rc.exe from the Windows SDK is required to embed application icons; \
         install the SDK or set WindowsSdkDir",
    );
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let script_path = out_dir.join(format!("{bin}.rc"));
    let res_path = out_dir.join(format!("{bin}.res"));
    // rc.exe treats backslashes inside quoted strings as escapes.
    let script = format!("1 ICON \"{}\"\n", escape_rc(&icon));
    std::fs::write(&script_path, script).expect("write resource script");
    let output = Command::new(&rc)
        .arg("/nologo")
        .arg(format!("/fo{}", res_path.display()))
        .arg(&script_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", rc.display()));
    assert!(
        output.status.success(),
        "rc.exe failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("cargo:rustc-link-arg-bin={bin}={}", res_path.display());
}

fn escape_rc(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn find_rc() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("WindowsSdkDir") {
        let dir = PathBuf::from(dir);
        roots.push(dir.join("bin"));
        // The variable may point inside a versioned bin directory already.
        roots.push(dir);
    }
    for program_files in [
        r"C:\Program Files (x86)\Windows Kits\10",
        r"C:\Program Files\Windows Kits\10",
        r"D:\Windows Kits\10",
        r"E:\Windows Kits\10",
        r"F:\Windows Kits\10",
    ] {
        roots.push(PathBuf::from(program_files).join("bin"));
    }
    let mut candidates: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let rc = entry.path().join("x64").join("rc.exe");
            if rc.is_file() {
                let version = entry
                    .file_name()
                    .to_string_lossy()
                    .split('.')
                    .filter_map(|part| part.parse::<u32>().ok())
                    .collect();
                candidates.push((version, rc));
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next_back().map(|(_, rc)| rc)
}
