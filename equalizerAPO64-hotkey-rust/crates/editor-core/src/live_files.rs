//! Which configuration files the editor publishes as a device's live set.
//!
//! The audio engine reads the files named in the manifest that lives next to
//! the configuration files (see `eapo_common::device_files`). This module only
//! decides *which* open documents belong in that manifest; writing it is the
//! frontend's job so the policy stays testable without a window.

use std::path::PathBuf;

/// How the editor keeps a device's live configuration in sync with its tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveMode {
    /// The manifest is never touched; the engine reads `config.txt`.
    Off,
    /// Only the frontmost saved tab is live: switching tabs switches presets.
    ActiveTab,
    /// Every saved tab is live, stacked in tab order.
    AllTabs,
}

impl LiveMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LiveMode::Off => "off",
            LiveMode::ActiveTab => "active",
            LiveMode::AllTabs => "all",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" => Some(LiveMode::Off),
            "active" => Some(LiveMode::ActiveTab),
            "all" => Some(LiveMode::AllTabs),
            _ => None,
        }
    }
}

/// The files to publish for `mode`, in load order.
///
/// Tabs that were never saved have no path and cannot be loaded by the audio
/// engine, so they are skipped. A file open in two tabs appears only once,
/// otherwise the engine would apply that configuration twice.
pub fn live_files(mode: LiveMode, tab_paths: &[Option<PathBuf>], active: usize) -> Vec<PathBuf> {
    let candidates: Vec<PathBuf> = match mode {
        LiveMode::Off => return Vec::new(),
        LiveMode::ActiveTab => tab_paths.get(active).cloned().flatten().into_iter().collect(),
        LiveMode::AllTabs => tab_paths.iter().flatten().cloned().collect(),
    };
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> Option<PathBuf> {
        Some(PathBuf::from(name))
    }

    #[test]
    fn off_publishes_nothing() {
        let tabs = [path("a.txt"), path("b.txt")];
        assert_eq!(live_files(LiveMode::Off, &tabs, 0), Vec::<PathBuf>::new());
    }

    #[test]
    fn active_tab_publishes_only_that_file() {
        let tabs = [path("a.txt"), path("b.txt"), path("c.txt")];
        assert_eq!(
            live_files(LiveMode::ActiveTab, &tabs, 1),
            vec![PathBuf::from("b.txt")]
        );
    }

    #[test]
    fn unsaved_and_out_of_range_tabs_are_skipped() {
        let tabs = [path("a.txt"), None];
        assert_eq!(live_files(LiveMode::ActiveTab, &tabs, 1), Vec::<PathBuf>::new());
        assert_eq!(live_files(LiveMode::ActiveTab, &tabs, 9), Vec::<PathBuf>::new());
        assert_eq!(
            live_files(LiveMode::AllTabs, &tabs, 0),
            vec![PathBuf::from("a.txt")]
        );
    }

    #[test]
    fn all_tabs_stack_in_order_without_duplicates() {
        let tabs = [path("b.txt"), path("a.txt"), path("b.txt"), None];
        assert_eq!(
            live_files(LiveMode::AllTabs, &tabs, 0),
            vec![PathBuf::from("b.txt"), PathBuf::from("a.txt")]
        );
    }

    #[test]
    fn mode_code_round_trips() {
        for mode in [LiveMode::Off, LiveMode::ActiveTab, LiveMode::AllTabs] {
            assert_eq!(LiveMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(LiveMode::parse(""), None);
        assert_eq!(LiveMode::parse("ACTIVE"), Some(LiveMode::ActiveTab));
        assert_eq!(LiveMode::parse("nonsense"), None);
    }
}
