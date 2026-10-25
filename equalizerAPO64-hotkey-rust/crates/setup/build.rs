//! Embeds a `requireAdministrator` UAC manifest into Setup.exe without
//! involving a C/C++ compiler or resource script.
//!
//! `link.exe` is part of the MSVC toolchain that rustc already uses for the
//! `*-pc-windows-msvc` targets. `/MANIFESTUAC` changes the requested execution
//! level of the manifest that rustc's linker invocation emits. This is the
//! pure-Rust-toolchain equivalent of NSIS' `RequestExecutionLevel admin`.
//!
//! The runtime elevation fallback in `src/elevation.rs` remains active, so the
//! installer still works if a toolchain/linker ignores these flags (or if the
//! executable is started through a mechanism that strips the manifest).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // Merged into rustc's auto-generated manifest.  Restrict the flags to
        // the Setup binary so `cargo test` does not require elevation.
        println!("cargo:rustc-link-arg-bin=Setup=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=Setup=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'"
        );
    }
}
