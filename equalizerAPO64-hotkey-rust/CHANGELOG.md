# Changelog

All notable changes to this project are documented here. The format is loosely based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.4.2] - 2026-10-25

Pure-Rust release of the C++ `equalizerAPO64-hotkey` 1.4.2 fork (upstream release
date 2026-07-01). Behaviour, configuration format, registry layout and the global
hotkey implementation are ported 1:1; see `PARITY.md` for the comparison and the
verification evidence.

### Added
- **Global hotkeys for filters** — assign a system-wide shortcut to any filter in the Editor
  to toggle its enabled state from any application.
- `HotkeyManager` and `HotkeyWorker` equivalents (`crates/editor/src/hotkey.rs`).
- `CommentFilterGUI` hotkey control for picking a hotkey per filter.
- Persistent hotkey storage in `HKCU\Software\EqualizerAPO\Hotkeys`.
- Destructor and lifecycle logging (`%TEMP%\equalizerapo_hotkey.log`).

### Changed
- The hotkey manager is owned independently of filter GUI lifetime, so rebuilding the filter
  table does not destroy registered hotkeys.
- The whole application (APO, Editor, DeviceSelector, Setup, UpdateChecker, VoicemeeterClient,
  Benchmark) is implemented in Rust with no C++/Qt runtime dependencies.

### Notes
- Configuration format, filter behavior, registry paths and the global hotkey implementation
  are compatible with the C++ 1.4.2 release.
