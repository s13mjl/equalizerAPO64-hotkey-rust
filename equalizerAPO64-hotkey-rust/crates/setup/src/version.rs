//! Version handling mirrored from `version.h` and `Setup.nsi`.
//!
//! `Setup.nsi` reads `MAJOR`, `MINOR` and `REVISION` from `version.h` and drops
//! the revision from the display version when it is zero. Cargo exposes the
//! same information through `CARGO_PKG_VERSION` (1.4.2 in this workspace).

pub const APP_NAME: &str = "Equalizer APO";
pub const TARGET_ARCH: &str = "x64";

/// The package version as written by Cargo (`MAJOR.MINOR.REVISION`).
pub const CARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Port of the NSIS compile-time expression
/// `!if ${REVISION} == 0 ... ${MAJOR}.${MINOR} ... !else ...`.
pub fn nsis_version(major: u32, minor: u32, revision: u32) -> String {
    if revision == 0 {
        format!("{major}.{minor}")
    } else {
        format!("{major}.{minor}.{revision}")
    }
}

/// Applies [`nsis_version`] to the Cargo package version.
pub fn display_version() -> String {
    let parts: Vec<u32> = CARGO_VERSION
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0))
        .collect();
    match parts.as_slice() {
        [major, minor, revision, ..] => nsis_version(*major, *minor, *revision),
        [major, minor] => nsis_version(*major, *minor, 0),
        _ => CARGO_VERSION.to_string(),
    }
}

/// `Name "Equalizer APO ${VERSION}"`.
pub fn full_name() -> String {
    format!("{APP_NAME} {}", display_version())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_zero_is_dropped() {
        assert_eq!(nsis_version(1, 4, 0), "1.4");
        assert_eq!(nsis_version(1, 4, 2), "1.4.2");
        assert_eq!(nsis_version(2, 0, 10), "2.0.10");
    }

    #[test]
    fn workspace_version_matches_upstream_version_header() {
        // version.h in the C++ tree contains MAJOR 1, MINOR 4, REVISION 2.
        assert_eq!(display_version(), nsis_version(1, 4, 2));
        assert_eq!(display_version(), "1.4.2");
        assert_eq!(full_name(), "Equalizer APO 1.4.2");
    }
}
