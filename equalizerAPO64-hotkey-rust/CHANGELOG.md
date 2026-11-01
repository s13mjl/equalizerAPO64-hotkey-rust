# Changelog

All notable changes to this project are documented here. The format is loosely based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed
- The built executables carried the generic Windows icon. Each one now embeds
  the icon of its C++ counterpart (`Editor.ico`, `DeviceSelector.ico`,
  `UpdateChecker.ico`, copied from the original project into `assets/icons/`),
  through a new build-dependency crate `crates/winicon` that drives the SDK
  `rc.exe` and passes the resulting `.res` to the linker. `VoicemeeterClient`,
  `Benchmark` and `Setup` are unchanged because the original project defines no
  icon for them.

### Added
- Per-device live configuration files. The native editor publishes the selected
  endpoint's file list to `<ConfigPath>\device-files.cfg` and the engine loads
  those files in order instead of `config.txt`. Following the frontmost tab is
  on by default — opening a second document and switching to it switches the
  live configuration, no menu needed; Settings/Device menus also offer
  *stack all open tabs* and *none (use config.txt)*. The manifest lives in the
  configuration directory (which the installer already grants to Users), so
  switching presets needs no elevation and is picked up by the existing reload
  watcher. Without a manifest entry the engine behaviour is byte-for-byte the
  original `config.txt` loading. New `crates/common/src/device_files.rs`
  (format) and `crates/editor-core/src/live_files.rs` (which tabs count), with
  tests.

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
