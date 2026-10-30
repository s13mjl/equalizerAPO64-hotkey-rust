# Changelog

All notable changes to this project are documented here. The format is loosely based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.4.2-r2] - 2026-10-30

Native Win32 editor refresh. Configuration format, registry layout and filter
semantics stay compatible with EqualizerAPO 1.4.2; only the applications changed.

### Added
- Native Win32/GDI editor (`Editor.exe`, `crates/editor-native`) on a shared
  UI-agnostic core (`crates/editor-core`). Idle memory drops from roughly 115 MB
  (egui) to single-digit MB while keeping every filter editor, the analysis dock,
  per-filter hotkeys and the original en/de/fr/zh translations.
- GraphicEQ: editable Freq./Gain node table, anti-aliased curves, wheel zoom and
  left-drag panning over the full 1 Hz–30 kHz / ±100 dB scene.
- Click-to-type inline editors for numeric fields (Biquad, Delay, Loudness,
  Preamp and GraphicEQ nodes).
- Hover feedback for custom-drawn controls, View → Toolbar toggle and
  Settings → Language / Reset preferences entries matching the original menus.
- Chinese READMEs (`README.zh-CN.md` in both the repository root and the port).

### Changed
- `EditorEgui.exe` remains available next to the new default `Editor.exe`.
- Device Selector: elevation manifest/relaunch, larger adaptive layout and a
  configuration-path guard that offers to run Setup.
- The editor detects a missing APO installation and offers to run Setup; engine
  analysis accepts an explicit configuration even without the registry value.

### Fixed
- Hotkey dialog focus/activation, per-channel analysis menus and localized menus.
- Preamp slider no longer disables an enabled filter; menu dropdowns open below
  their controls and dismiss on a second click.
- Flicker-free double-buffered painting; analysis curve smoothing; i18n for the
  filter rows and startup prompts.

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
