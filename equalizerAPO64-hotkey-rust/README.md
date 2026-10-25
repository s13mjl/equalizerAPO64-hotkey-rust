# EqualizerAPO64-Hotkey — pure Rust port

A 1:1 functional port of
[EqualizerAPO64-Hotkey](https://github.com/s13mjl/equalizerAPO64-hotkey)
(a fork of EqualizerAPO with **global per-filter hotkeys** in the Editor) to
**100% Rust**. No C++, no Qt, no FFTW, no libsndfile, no muparserx.

The Windows audio processing object (APO), the Editor GUI, the device
selector, the installer, the updater, the Voicemeeter client and the
benchmark are all reimplemented in Rust and keep the original behavior,
configuration format, registry layout and filter semantics.

## Features

- **Per-filter global hotkeys** — assign a system-wide shortcut (e.g.
  `Alt+X`) to any filter to toggle its enabled state from any application,
  even while the Editor is in the background.
  - Global registration through Win32 `RegisterHotKey` (`MOD_NOREPEAT`).
  - Dedicated worker thread with a classic `GetMessage` loop, which is
    required because modern GUI event loops do not dispatch `WM_HOTKEY` to
    the application's native event filter.
  - Per-filter shortcuts stored in
    `HKCU\Software\EqualizerAPO\Hotkeys\<filterId>\keySequence`.
  - Conflict detection and persistent settings.
  - Hotkey diagnostic log at `%TEMP%\equalizerapo_hotkey.log`.
- All upstream features preserved:
  - 64-bit double precision audio processing (`f64` end to end).
  - Full EqualizerAPO configuration format:
    `Preamp`, `Filter` (PK/PEQ/Modal/LP/HP/LPQ/HPQ/BP/LS/HS/LSC/HSC/NO/AP),
    `IIR`, `GraphicEQ`, `Convolution`, `Copy`, `Delay`, `Channel`, `Stage`,
    `Device`, `If`/`ElseIf`/`Else`/`EndIf`, `Eval` and inline expressions,
    `Include`, `VSTPlugin`, `LoudnessCorrection`.
  - Expression language compatible with the muparserx subset used by
    EqualizerAPO (variables, arrays, strings, regular expressions, registry
    access, ternary operator, assignments) implemented in safe Rust.
  - File/registry change monitoring with seamless crossfades between
    configurations.
  - Editor with filter chain editing, analysis plot, per-filter GUIs,
    drag & drop, global hotkeys, and the original German/French/Chinese
    translations.
  - VST 2.4 plugin hosting (double precision, chunk/parameter state, native
    plugin editor window).
  - WAV, FLAC and OGG/Vorbis impulse responses and benchmark input.
  - DeviceSelector, UpdateChecker, VoicemeeterClient, Benchmark and Setup.

## Workspace layout

| Crate | Original counterpart |
|-------|----------------------|
| `crates/common` | `helpers/` (`RegistryHelper`, `StringHelper`, `ChannelHelper`, `GainIterator`, `LogHelper`, `ServiceHelper`, `TaskSchedulerHelper`, `AbstractLibrary`, VST host helpers) |
| `crates/expression` | muparserx + `parser/` (`LogicalOperators`, `StringOperators`, `RegexFunctions`, `RegistryFunctions`) |
| `crates/filters` | `filters/` and `libHybridConv-0.1.1/` |
| `crates/engine` | `FilterEngine`, `FilterConfiguration`, all `*FilterFactory` |
| `crates/apo-info` | `AbstractAPOInfo`, `DeviceAPOInfo`, `VoicemeeterAPOInfo` |
| `crates/apo` | `EqualizerAPO/` (COM APO DLL) |
| `crates/editor` | `Editor/` (GUI + global hotkey manager) |
| `crates/device-selector` | `DeviceSelector/` |
| `crates/update-checker` | `UpdateChecker/` |
| `crates/voicemeeter-client` | `VoicemeeterClient/` |
| `crates/benchmark` | `Benchmark/` |
| `crates/setup` | `Setup/` (NSIS installer and uninstaller) |

## Building

Requirements: Windows 10/11 x64 and a Rust toolchain with the
`x86_64-pc-windows-msvc` target.

```cmd
cargo build --release --workspace
```

The binaries are produced under `target\release`:

- `EqualizerAPO.dll` (COM APO, `crates/apo`)
- `Editor.exe`
- `DeviceSelector.exe`
- `UpdateChecker.exe`
- `VoicemeeterClient.exe`
- `Benchmark.exe`
- `Setup.exe`

Run `build.bat` to build the workspace, stage all binaries into
`installer-output\files` and (optionally) assemble the installer payload.

## Usage

1. Install with `Setup.exe` (or copy the binaries and register the APO with
   DeviceSelector).
2. Open the Editor.
3. Right-click a filter in the filter chain → **Set Hotkey** (or use the
   control in the filter's GUI).
4. Press the desired key combination (e.g. `Alt+X`).
5. With the Editor running in the background, press the hotkey — the filter
   toggles.

Hotkeys are stored under
`HKCU\Software\EqualizerAPO\Hotkeys\<filterId>\keySequence`.

## Implementation notes

The original C++ uses a WDK `CBaseAudioProcessingObject`. The Rust APO
implements the COM interfaces (`IAudioProcessingObject`,
`IAudioProcessingObjectRT`, `IAudioProcessingObjectConfiguration`,
`IAudioSystemEffects`) directly with ABI-compatible vtables.

The DSP core processes `f64` samples and reproduces the original coefficient
formulas for every biquad type, the dynamic loudness filter, the partitioned
FFT convolution, copying/routing and delay semantics. Configuration reloads
are built off the audio thread and crossfaded over 10 ms, exactly like the
original.

The hotkey feature uses a dedicated Win32 message-loop thread. This is
required because the GUI event loop does not retrieve messages with
`GetMessage`, so `WM_HOTKEY` would otherwise never reach user code. The
worker registers/unregisters through posted thread messages and reports
back synchronously.

## License

GPL-2.0-or-later. See [License.txt](License.txt). Original EqualizerAPO
copyright (C) Jonas Thedering and contributors; hotkey fork additions
copyright (C) EqualizerAPO Contributors.
