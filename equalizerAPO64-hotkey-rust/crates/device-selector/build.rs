//! Embeds a `requireAdministrator` UAC manifest into DeviceSelector.exe.
//!
//! The C++ project uses `UACExecutionLevel=RequireAdministrator`; the device
//! selector needs administrator rights to register the APO and to restart the
//! audio service. `link.exe` merges these flags into the manifest that rustc's
//! linker invocation emits.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // Restrict the flags to the DeviceSelector binary so `cargo test`
        // does not require elevation.
        println!("cargo:rustc-link-arg-bin=DeviceSelector=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=DeviceSelector=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'"
        );
    }
}
