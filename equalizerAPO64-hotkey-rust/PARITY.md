# Native low-memory Editor — parity notes

This branch replaces the egui-based Editor frontend with a native Win32/GDI
implementation (`crates/editor-native`, shipped as `Editor.exe`). The old egui
frontend is kept during the transition as `EditorEgui.exe`
(`crates/editor`), and the UI-agnostic logic it shared now lives in
`crates/editor-core` (model, i18n, settings, analysis worker, hotkeys,
calibration, VST host, plot math, config file I/O).

## Measured memory (K32GetProcessMemoryInfo medians, `tools/mem-probe`)

| Scenario | Native `Editor.exe` | old egui Editor | C++/Qt Editor |
|---|---:|---:|---:|
| Small live config, idle | 3.7 MB private / 20.4 MB WS | ~115 MB private / ~145 MB WS | ~31 MB private / ~105 MB WS |
| 10,000-line config, idle | 5.1 MB private / 18.5 MB WS | — | — |

GDI/USER objects stay flat (13/22) across scrolling and editing; there is no
per-frame allocation or full-window backbuffer.

## What the native Editor implements

- Tabs (new/open/save/save-as/recent files, dirty markers, close), inline
  line editing with IME-friendly child `EDIT` overlay, multi-select,
  cut/copy/paste/delete, drag reorder with autoscroll, row enable/disable,
  per-row hotkeys (`RegisterHotKey` worker + JSON row prefs).
- All 12 filter row editors: Preamp, Biquad (type/freq/gain/Q + kind menu),
  Delay, Stage, Include, Convolution, VST (path edit, Select, status line,
  Open panel, Options with Embed/Auto apply), LoudnessCorrection (drag fields
  + full calibration dialog with pink-noise playback), Channel and Device
  (native check-list dialogs with "all" and installed-only filter), Copy
  (modal routing editor with Graph and Assignments tabs), GraphicEQ
  (interactive nodes, 15/31/variable bands, invert/normalize/reset).
- Analysis panel: background worker (single `FilterEngine` + FFT plan reuse),
  peak/latency/init/CPU readouts, start-from/channel/resolution menus,
  memory-budgeted resolution cap with a visible warning, logarithmic plot
  with wheel zoom, right-drag pan and a crosshair readout, all sharing
  `editor-core::plot_math::PlotState`.
- DeviceSelector integration, startup checks, master-volume polling for
  LoudnessCorrection, i18n (en/de/fr/zh via View > Language, applied after
  restart), PerMonitorV2 DPI handling, and `--scroll-test` (10k synthetic
  rows) / `--no-checks` switches.

## Known deviations / gaps

- The analysis resolution preflight caps extreme values at ~1 GB and shows
  "resolution reduced … to limit memory"; this is the one intentional
  behavior change (the old UI allowed resolutions that allocate >1 GB).
- Filter row titles (Preamp/Filter/…) are still English; menus, toolbar,
  dialogs and the analysis panel are translated. Filter-catalog names follow
  the config syntax and are not translated.
- The engine's analysis path does not apply FIR-based GraphicEQ responses
  (pre-existing engine issue, tracked by an ignored test in `editor-core`).
- VST "Embed" is stored but not yet used by the host (same as the egui
  frontend, which also only implements the non-embedded path).
- Accessibility (UIA) is not implemented in the native frontend.

## Verification

- `cargo test --workspace` passes (unit tests for model round-trips, i18n
  catalogues, hotkey registry format, calibration WAV building, plot math,
  analysis preamp response, settings/row prefs).
- Dialog flows were verified end-to-end with scripted Win32 messages
  (PostMessage/SendMessage): channel/device selection persistence, Copy
  editing/removal, calibration state handling, VST panel open/close, plot
  zoom/pan/hover.
- `installer-output\Setup.exe` builds via `build.bat` and the
  shipped `Editor.exe` has no VC runtime dependencies (checked with
  `dumpbin /dependents`).
