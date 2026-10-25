# 1:1 Porting / Parity Matrix

Source: `equalizerAPO64-hotkey` (C++17 / Qt 6 / MSVC, version 1.4.2).
Target: this workspace — a pure-Rust implementation with no C++/Qt/FFTW/
libsndfile/muparserx binaries. Crate boundaries deliberately mirror the C++
projects (`Common`, `EqualizerAPO`, `Editor`, `DeviceSelector`,
`UpdateChecker`, `VoicemeeterClient`, `Benchmark`, `Setup`, `filters`,
`helpers`, `parser`, `libHybridConv`).

Status legend: **ported** (code present), **verified** (covered by an automated
test or a runtime observation against the C++ build), **adapted** (Rust
replacement with the same externally visible behavior), **deviation** (see
the list at the end).

## Core engine and configuration

| Source | Rust | Status |
|---|---|---|
| `IFilter.h`, `IFilter.cpp` | `crates/filters/src/lib.rs` (`IFilter`) | verified |
| `IFilterFactory.h` | `crates/engine/src/factories.rs` (ordered dispatch) | verified |
| `FilterEngine.h/.cpp` | `crates/engine/src/filter_engine.rs` | verified |
| `FilterConfiguration.h/.cpp` | `crates/engine/src/filter_configuration.rs` | verified |
| `filters/DeviceFilterFactory.*` | `factories.rs::device_factory` | verified |
| `filters/IfFilterFactory.*` | `factories.rs::if_factory` | verified |
| `filters/ExpressionFilterFactory.*` | `factories.rs::expression_factory` | verified |
| `filters/IncludeFilterFactory.*` | `factories.rs::include_factory` | verified |
| `filters/StageFilterFactory.*` | `factories.rs::stage_factory` | verified |
| `filters/ChannelFilterFactory.*` | `factories.rs::channel_factory` | verified |
| `filters/IIRFilterFactory.*` | `factories.rs::iir_factory` | ported |
| `filters/BiQuadFilterFactory.*` | `factories.rs::biquad_factory` | verified |
| `filters/PreampFilterFactory.*` | `factories.rs::preamp_factory` | verified |
| `filters/DelayFilterFactory.*` | `factories.rs::delay_factory` | ported |
| `filters/CopyFilterFactory.*` | `factories.rs::copy_factory` | ported |
| `filters/ConvolutionFilterFactory.*` | `factories.rs::convolution_factory` | ported |
| `filters/GraphicEQFilterFactory.*` | `factories.rs::graphic_eq_factory` | verified |
| `filters/VSTPluginFilterFactory.*` | `factories.rs::vst_plugin_factory` | ported |
| `filters/loudnessCorrection/LoudnessCorrectionFilterFactory.*` | `factories.rs::loudness_correction_factory` | ported |

Engine tests cover: factory dispatch order, If/ElseIf/Else/EndIf nesting,
Device matching, Include recursion, Stage gating, Eval/assignments/inline
expressions, registry expression functions + watch keys, channel routing,
mono→stereo upmix, non-interleaved buffers, and the cosine crossfade.

## Filters (DSP, `f64`)

| Source | Rust | Status |
|---|---|---|
| `filters/BiQuad.*` | `crates/filters/src/biquad.rs` | verified (coefficients/response) |
| `filters/BiQuadFilter.*` | `biquad_filter.rs` | verified |
| `filters/IIRFilter.*` | `iir_filter.rs` | verified |
| `filters/GraphicEQFilter.*` | `graphic_eq_filter.rs` + `convolution_filter::build_graphic_eq_impulse` | verified (runtime differential) |
| `filters/ConvolutionFilter.*` | `convolution_filter.rs` | verified (impulse tests + WAV/FLAC differential) |
| `libHybridConv-0.1.1/libHybridConv*` | `hybrid_conv.rs` (rustfft) | verified (delta/partition tests) |
| `filters/ChannelFilter.*` | `channel_filter.rs` | verified |
| `filters/CopyFilter.*` | `copy_filter.rs` | verified |
| `filters/DelayFilter.*` | `delay_filter.rs` | verified |
| `filters/PreampFilter.*` | `preamp_filter.rs` | verified |
| `filters/loudnessCorrection/LoudnessCorrectionFilter.*` | `loudness_correction/mod.rs` | verified (parameter archive, shelf math) |
| `filters/loudnessCorrection/VolumeController.*` | `loudness_correction/volume_controller.rs` | ported (COM) |
| `filters/VSTPluginFilter.*` | `vst_plugin.rs` | ported |
| `helpers/aeffectx.h` subset | `vst_abi.rs` | verified (layout offsets) |
| `helpers/VSTPluginInstance.*`, `VSTPluginLibrary.*` | `vst_plugin_instance.rs`, `vst_plugin_library.rs` | verified (ABI/base64/shared instances) |
| libsndfile use in convolution/benchmark | `audio_file.rs` (hound + claxon + lewton) | adapted (see deviations) |

## Helpers / foundation (`crates/common`)

| Source | Rust | Status |
|---|---|---|
| `RegistryHelper.*` | `registry_helper.rs` | verified (25 tests incl. HKCU round-trip, ACL, export) |
| `StringHelper.*`, `ChannelHelper.*`, `GainIterator.*` | corresponding modules | verified |
| `LogHelper.*` | `log_helper.rs` | ported (same `%TEMP%\EqualizerAPO.log` format) |
| `MemoryHelper.*` | `memory_helper.rs` | adapted |
| `PrecisionTimer.h`, `ScopeGuard.h` | corresponding modules | ported |
| `AbstractLibrary.*`, `ServiceHelper.*`, `TaskSchedulerHelper.*` | corresponding modules | ported |
| `UncaughtExceptions.h`, `stdafx.*` | not applicable | adapted |

## Expression language / muparserx extensions

| Source | Rust | Status |
|---|---|---|
| muparserx packages (Common, NonCmplx, Str, Matrix) | `crates/expression` | verified (wiki examples, precedence, arrays, casts) |
| `parser/LogicalOperators.*` | `not` unary in lexer/parser | verified |
| `parser/StringOperators.*` | `+` concatenation | verified |
| `parser/RegexFunctions.*` | `regexSearch`, `regexReplace` | verified |
| `parser/RegistryFunctions.*` | `readRegString`, `readRegDWORD` + watcher | verified |

The interpreter is allowlisted (no I/O except explicit registry callbacks).

## APO / device integration

| Source | Rust | Status |
|---|---|---|
| `AbstractAPOInfo.*` | `crates/apo-info/src/abstract_apo_info.rs` | verified (live enumeration) |
| `DeviceAPOInfo.*` | `device_apo_info.rs` | verified (real endpoints loaded) |
| `VoicemeeterAPOInfo.*` | `voicemeeter_apo_info.rs` | ported |
| `EqualizerAPO/DllMain.cpp` | `crates/apo/src/lib.rs` (lazy module handle) | verified (4 exports, no DllMain export) |
| `EqualizerAPO/ClassFactory.*` | `crates/apo/src/class_factory.rs` | verified (COM instantiation test) |
| `EqualizerAPO/EqualizerAPO.*` | `crates/apo/src/apo.rs` (DSP/lifecycle) + `crates/apo/src/cxx/apo_shim.cpp` (WDK base class) | verified live in `audiodg.exe`: `Initialize` -> `IsInputFormatSupported` -> `LockForProcess successful` -> config load (see checklist) |
| `EqualizerAPO.def` | `DllCanUnloadNow`, `DllGetClassObject`, `DllRegisterServer`, `DllUnregisterServer` | verified with objdump (module definition file emitted by `build.rs`) |
| APO registration data | `crates/apo/src/registration.rs` | verified (GUIDs/flags/versions/interface) |
| `DeviceSelector/*` | `crates/device-selector/src/*` | ported (incl. `/i`, `/u`, named-pipe test, AudioSrv restart) |
| `VoicemeeterClient/*` | `crates/voicemeeter-client/src/main.rs` | ported |

Evidence: an opt-in (`#[ignore]`) end-to-end COM test creates the class via
`DllGetClassObject`, locks a 48 kHz stereo float stream, processes 1024 frames
through the real installed configuration and verifies that silent input is
zeroed and marked `BUFFER_SILENT`. It cannot pass outside a live audio graph:
the WDK base class rejects the synthetic connection descriptors, and the
reference C++ DLL returns the identical `E_INVALIDARG` for them, so the test
only documents the base-class contract instead of proving live behaviour. Live
device enumeration lists the machine's render/capture endpoints with the same
installed flags.

## Editor

| Source | Rust | Status |
|---|---|---|
| `main.cpp`, `MainWindow.*` | `main.rs`, `main_window.rs` | verified (GUI launch/screenshot, menus/actions, prefs) |

Startup file selection follows `main.cpp` exactly: the config directory is the
registry `ConfigPath` (falling back to the working directory), positional
arguments are opened relative to it, and with no arguments the editor opens
`<ConfigPath>\config.txt`. It deliberately does **not** restore the previous
session's tabs — an earlier port revision did, which could silently open a file
from another directory while the audio engine kept reading the live config.
`tests::commented_rows_keep_their_type_and_enable_cleanly` pins the other
editor-side contract that caused confusion: a leading `#` is the "row switched
off" marker (`CommentFilterGUI::store` prefixes it), the row keeps its filter
type, and switching the power on rewrites the active line verbatim.
| `FilterTable.*`, `Row`, `MimeData`, `Template` | `filter_table.rs` | verified (parse/save CRLF, drag reorder, templates) |
| `AnalysisThread.*`, `AnalysisPlot*`, `FrequencyPlot*` | `analysis.rs`, `plot.rs` | verified (peak gain 4.5 dB matches C++ for same config) |
| `guis/*` (all factories/GUIs) | `guis.rs` (+ `vst_editor.rs`, `calibration.rs`) | ported / partially verified |
| `dialogs/HotkeyEditDialog.*` | `hotkey_dialog.rs` | ported |
| `helpers/HotkeyManager.*` | `hotkey.rs` | verified (real Win32 round-trip test) |
| `keyseq` (Qt `QKeySequence` compatibility) | `keyseq.rs` | verified (round-trips, Qt text order) |
| `helpers/GUI*`, widgets | egui equivalents in the same files | adapted |

### Global hotkey contract

- Registry: `HKEY_CURRENT_USER\Software\EqualizerAPO\Hotkeys\<filterId>\keySequence` (`REG_SZ`).
- IDs: base `0x0100`, max `0xBFFF`, wrap-around.
- Modifiers: `MOD_ALT|MOD_CONTROL|MOD_SHIFT|MOD_WIN` plus `MOD_NOREPEAT`.
- Dedicated worker thread with `GetMessageW`; registration on that thread;
  wake via `PostThreadMessageW(WM_NULL)`; 2 s completion timeout.
- Conflict detection against all registered hotkeys.
- Pressing toggles the filter's enabled state and, in instant mode, saves the
  config immediately.
- Per-file preferences (`HKCU\...\Configuration Editor\file-specific\<path>`)
  store `{"hotkey":"..."}` exactly like the C++ `rowPrefs` format; this is the
  persistent source across restarts (the C++ Hotkeys key is rewritten on
  registration, like the original).
- Log: `%TEMP%\equalizerapo_hotkey.log`, same message catalogue.
- Verification: existing C++-created prefs were loaded by the Rust Editor
  (Alt+X restored and re-registered), and a real `Ctrl+Alt+Shift+F9` press
  was delivered through Win32 to the worker in a test.

## Other executables / installer

| Source | Rust | Status |
|---|---|---|
| `UpdateChecker/*` | `crates/update-checker/src/main.rs` | ported (URL, skip/lastCheck, task install, WinHTTP) |
| `Benchmark/Benchmark.cpp` | `crates/benchmark/src/main.rs` | verified (runtime differential) |
| `Setup/Setup.nsi` (+ 32/ARM64 variants) | `crates/setup` (`eapo-wizard` + `Setup`) | verified (31 tests, release manifest) |
| `Setup/config/*`, `Setup/*.url` | `assets/setup-config`, `assets/*.url` | copied |
| `build.bat`, CI | root build script/workflow | adapted |
| `README.md`, `CHANGELOG.md`, `License.txt` | mirrored/adapted | ported |

### Benchmark differential evidence

Run against the installed C++ 1.4.2 `Benchmark.exe` with the same
configuration:

| Input | C++ max / clipped | Rust max / clipped | PCM16 sample differences |
|---|---|---|---|
| generated sweep | 1.163816, 578 | 1.163816, 578 | ≤ 1 LSB except near-unity wrap points |
| WAV input | 0.522073, 0 | 0.522073, 0 | 11573/48000 samples differ by exactly 1 LSB |
| FLAC input | 0.090731, 0 | 0.090731, 0 | 1778/96000 samples differ by exactly 1 LSB |

The small differences come from rustfft vs FFTW arithmetic order; no
functional difference. libsndfile's default no-clipping wrap semantics for
PCM16 output is reproduced.

### Setup

The wizard mirrors the NSIS pages/components, registry values, file layout,
default configs, ACL grant, shortcuts, APO/DeviceSelector invocation, update
task, and uninstall order. `/S` silent install, `/D=<dir>` and `/S` uninstall
are supported; a `requireAdministrator` manifest is embedded into `Setup.exe`
only. `Publisher`/`DisplayIcon` (not written by the NSIS script) were removed.

## Deviations (explicit)

1. **GUI technology**: Qt forms are replaced by egui/Win32. Layout is
   functionally equivalent but not pixel-identical. Filter row labels/actions,
   plots, dialogs and analysis values are present.
2. **Localization**: the German/French/Chinese `.ts` translations of the
   Editor, DeviceSelector and UpdateChecker are generated into per-crate
   `i18n.rs` modules; standard Qt button labels were added manually. Strings
   added by the hotkey fork have no upstream translation and therefore stay
   English, exactly as the C++ build falls back. Setup is English/German;
   VoicemeeterClient has no upstream translations.
3. **VST GUI**: the native VST editor opens in a modal host window with
   Apply/OK/Cancel and auto-apply. The Qt "Embed inside the filter row"
   preference opens the same host window instead of embedding in the egui row.
   Plugin crashes implemented as Rust panics are caught; access violations
   (SEH) cannot be safely caught in stable Rust, unlike the C++ `__except`.
   The host ABI, chunk/parameter round-trip, double processing and editor
   open/close are covered by a purpose-built pure-Rust VST 2.4 fixture
   (`testdata/vst-test-plugin`).
4. **VST host time info**: per-instance instead of the C++ global `vstTime`
   (same values; avoids cross-instance races).
5. **Audio file formats**: WAV, FLAC and OGG/Vorbis are supported via pure-Rust
   decoders. libsndfile-only formats (AIFF, AU, WavPack, CAF, RAW, etc.) are
   not decoded and report "unsupported audio file format"/"Unsupported file
   format". File permissions for the audio service are checked like C++.
6. **Expression regex**: Rust `regex` is used for `regexSearch`/`regexReplace`
   and Device matching. Backreferences/look-around accepted by `std::regex`
   are not supported.
7. **APO registration**: `RegisterAPO` from the WDK static library is replaced
   by writing the same `HKLM\SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\{GUID}`
   values plus the CLSID `InprocServer32` keys. The WDK helper's internal RPC
   notification is not reproduced; DeviceSelector restarts `AudioSrv` after
   installation exactly like the C++ application.
8. **WDK APO base class (approved deviation from "pure Rust")**: the object the
   audio engine instantiates is a ~300-line C++ `CBaseAudioProcessingObject`
   subclass (`crates/apo/src/cxx/apo_shim.cpp`) that forwards every
   `IAudioProcessingObject`/`RT`/`Configuration`/`IAudioSystemEffects` call to
   the pure-Rust object and links against
   `audiobaseprocessingobject.lib`. The C++ build inherits the same base from
   the WDK, and the base's private state (`m_bIsLocked`, cached connection
   formats, connection validation) is not observable from Rust, so re-implementing
   it by hand was the remaining unknown for live activation. All DSP, engine,
   configuration, COM contract handling, registration and every executable stay
   in Rust; `build.rs` compiles the shim with the installed MSVC `cl.exe`/`lib.exe`
   and restricts the export table to the original four names through a generated
   `.def` file.
   The Rust object does not reimplement the base class anymore: when it is owned
   by the wrapper it forwards `IsInputFormatSupported`/`IsOutputFormatSupported`,
   `LockForProcess` and `UnlockForProcess` to the real
   `CBaseAudioProcessingObject` through `eapo_cxx_base_*` entry points, exactly
   where `EqualizerAPO.cpp` calls them (a null wrapper keeps the standalone Rust
   behaviour for unit tests). `tests::wrapper_reaches_the_wdk_base_class` proves
   the forwarding is live: unlocking a never-locked object reports
   `APOERR_ALREADY_UNLOCKED`, which only the WDK base returns.
   The wrapper is built with **one** registration blob, the post-mix one, for
   every class id — `CBaseAudioProcessingObject(baseProperties())`, the direct
   equivalent of `EqualizerAPO::EqualizerAPO : CBaseAudioProcessingObject(regPostMixProperties)`.
   That is why a *pre-mix* instance reports the **post-mix** CLSID from
   `GetRegistrationProperties`; the audio engine reads that blob, so the quirk is
   part of the contract and is asserted by
   `tests::instances_report_the_post_mix_properties_like_the_reference`. It was a
   real parity defect, but re-testing showed it was *not* the cause of the
   render-graph crash (that was the missing `pUnkOuter` delegation; see the
   checklist). Both blobs are
   `CRegAPOProperties<1>` statics, so — as in C++ — they are filled by a static
   initializer under the loader lock instead of a lazy first-call fill.
   The shim also accepts one extra, verification-only class id
   (`EQUALIZERAPO_PROBE_GUID`, `{D3A9E4B1-6C25-4F7A-9E83-1B5C7D2A4E60}`) whose
   *registration* key carries its own identity. It exists because the audio
   engine caches a class object per CLSID inside `audiodg.exe`: a brand new class
   id is the only way to load our DLL into an already running audio process
   without restarting the audio service (see the live checklist item).
   **It is compiled out of shipped builds**: everything about it sits behind the
   `live-probe` cargo feature (and `EAPO_LIVE_PROBE` for the C++ side), so
   `cargo build --release` — and therefore `installer-output` — contains no trace
   of it. Verified byte-level: the shipped `EqualizerAPO.dll` contains 0
   occurrences of that GUID versus 2 in the `--features live-probe` build, both
   export exactly the original four names, and
   `tests::rejects_the_verification_class_id_unless_the_feature_is_on` asserts the
   shipped binary answers `CLASS_E_CLASSNOTAVAILABLE` for it.
9. **APO live installation**: the code paths are implemented but not executed
   during verification, to avoid modifying the machine's installed APO
   registration and audio devices. COM behavior, registration data, exports
   and processing were validated without installation. The registry content our
   installer produces was meanwhile compared value-by-value against the keys the
   reference `RegisterAPO` wrote on this machine: same 12 value names in the same
   order, same types, `Flags=0xD`, `MaxInstances=0xFFFFFFFF`,
   `APOInterface0={FD7F2B29-24D0-4B5C-B177-592C39F9CA10}`.
10. **Engine-owned `PROPVARIANT`**: `EqualizerAPO::Initialize` never calls
    `PropVariantClear` on the variant it reads out of
    `pAPOEndpointProperties`, and the port now matches that exactly (it used to
    clear it). `pAPOEndpointProperties` is the engine's own store, so the string
    it hands back may be engine-owned memory and clearing it frees a live buffer.
11. **Numeric FFT differences**: rustfft vs FFTW can differ by one PCM16 LSB
   (see table). All levels, clipping counts and filter responses match.
12. **Editor menu**: only upstream actions are exposed (New/Open/Save/Save As/
    recent/Exit, Cut/Copy/Paste/Delete/Select All, Toolbar/Analysis, Language/
    reset preferences); temporary extra actions from development were removed.
13. **Exotic Windows APIs/format edge cases**: invalid/unsupported inputs are
    handled with safe errors where the C++ code relied on undefined behavior
    (e.g. convolutions with a partial final block, zero-channel VST plugins).

## Verification checklist

- [x] `cargo build --workspace` clean (no warnings in the final build).
- [x] `cargo test --workspace`: 151 passing, 8 opt-in tests ignored.
- [x] **Differential COM oracle against the reference binary.**
      `tests::com_surface_matches_the_reference_cpp_binary` (opt-in,
      `CPP_APO_PATH`) loads our DLL and the installed C++ `EqualizerAPO.dll` in
      one process and probes instantiated objects with an identical sequence: the
      `QueryInterface` answer for 14 interface ids, the full
      `GetRegistrationProperties` field set, `GetLatency`/`Reset`/
      `GetInputChannelCount` in the unlocked state, five `Initialize`
      argument-validation cases restricted to the paths that return before the
      init block is dereferenced, and `IsInputFormatSupported` +
      `IsOutputFormatSupported` for five format combinations including the
      contents of any returned media type. The sequence runs against a post-mix
      instance, then — added because the first version of the oracle only ever
      created the post-mix object — against a **pre-mix** and a second post-mix
      instance of both binaries, before and after a real-endpoint-store
      `Initialize`; 137 answer-sheet lines in total. Both sheets must be
      byte-identical; today all 137 probes match.
      It needs no audio device, no endpoint binding and no service restart.
      Running it found three real divergences that no unit test could see:
      - the C++ wrapper did not override `GetLatency`, so it inherited the WDK
        base class's `S_OK`/0 instead of `EqualizerAPO::GetLatency`'s
        `APOERR_ALREADY_UNLOCKED` (`0x887D0006`) for an unlocked object;
      - `Initialize` accepted any block of `>= 56` bytes where the C++ requires
        exactly `sizeof(APOInitSystemEffects)`; restored to the exact comparison;
      - **the wrapper passed the class id's own registration blob to the WDK base
        class, so a pre-mix instance reported the pre-mix CLSID while the
        reference reports the post-mix one.** Reverting just this line makes the
        oracle fail on exactly that field (`clsid=EACD2258` versus the reference's
        `clsid=EC1CC9CE`), which is how the fix was proven rather than assumed.
      After the earlier fixes the live capture-chain re-verification still passes
      (`difftest/apo-backup-20261024/child-apo-test8.txt`).
- [x] **Render-path crash in `audiodg.exe`: root-caused and fixed.** The APO was
      created, initialized and answered the format probe, then the engine stopped
      calling it and `audiodg.exe` faulted at `audioeng.dll+0xFE868` (exception
      `0xc0000005`, a jump to a non-executable `.rdata` address). The failure
      chain, in the order it was established:
      - `LocalDumps` + `cdb`, with the Microsoft symbols for `audiodg.exe`
        cached, name the stack: the fault is in
        `audiodg!CPipeInstance::InitializeAPOInterfaces`, called from
        `ConnectAPOs` -> `CStreamInstance::CreateStream` — the APO activation path
        when a stream starts, not the processing path;
      - the faulting instruction is
        `mov rbx,[rsi+20h]; mov rbx,[rbx+28h]; mov rax,[rbx]; mov rax,[rax+8];
        call <CFG dispatch>`, i.e. `pAPO->AddRef()` on the pointer stored in the
        pipe's APO record;
      - that pointer is `0x6617f8` — the **same address in three separate
        `audiodg` processes**, so it is not a random stale value; the block is not
        one of our objects (the probe build logged our wrappers at
        `0x11FA600`/`0x11FB7E0`/`0x11FA740`), and a heap search shows its vptr
        slot holding the address of an audioeng ATL `CComAggObject<CAudioMeter>`
        vtable field — recycled memory. With page heap enabled the identical site
        instead read a null vptr and dispatched through null, i.e. the block is
        simply not a live object;
      - dumping our wrapper from the same process
        (`0x11FB7E0`: `inner`/`rt`/`cfg` at `+0x80/+0x88/+0x90`, matching the
        probe log) put the pointer in the **`pUnkOuter`** field: the engine
        aggregates the APO (`CSystemEffectWrapper` passes itself as the outer
        unknown), and `0x6617f8` is that outer object, already destroyed.
      **Root cause**: `RustApo::AddRef`/`Release` used a private counter instead of
      `return pUnkOuter->AddRef();` / `Release();`, which is what
      `EqualizerAPO::AddRef`/`Release` do. Every reference the engine took through
      the primary interface therefore never reached the outer object, so the
      engine freed its `CSystemEffectWrapper` while our APO was still alive, and
      the next `AddRef` on that wrapper dispatched through freed memory. In the
      non-aggregated case the delegation is what the old code did anyway
      (`pUnkOuter` is the `INonDelegatingUnknown` sub-object, whose slots land on
      `NonDelegatingAddRef`/`NonDelegatingRelease`), so only the aggregated case
      changed — and that is the one the engine uses. `NonDelegatingAddRef` now
      increments the counter directly, as in the C++ original.
      Why the differential oracle missed it: it always passes `pUnkOuter = NULL`,
      so the aggregation path was never compared. `tests::references_on_the_primary_interface_are_forwarded_to_the_outer`
      now creates an aggregated instance with a fake outer and asserts that
      `QueryInterface`/`AddRef` through the primary interface reach it; it fails on
      the old code (the outer counter stayed 0).
      Live confirmation with the shipped build
      (sha256 `C18A3293E59BBECF890642F55A218BEDDC9EC3A2553C0C10AAC39F315400FE5E`;
      the published installer payload is a `--workspace` rebuild of the same
      source, sha256 `0D8AEF1B0F3AD3CC…`),
      `difftest/apo-backup-20261024/render-confirm.txt`: 88 log lines, seven
      instances (4 pre-mix + 3 post-mix), **4x `LockForProcess successful`**, 4x
      `Reallocating internal double-precision buffers for 480/528 frames and 2/2
      channels`, 4x `Finished loading configuration`, **0 error lines, no new
      crash dump, `audiodg.exe` still running**. The reference C++ build in the
      same configuration produces the same sequence (42 lines, `LockForProcess
      successful`), which is now the parity baseline for this path.
      Earlier hypotheses that this investigation ruled out (kept here so they are
      not re-tried): the pre-mix instance reporting its own CLSID (a real parity
      defect, fixed, but the crash survived it); `PropVariantClear` on the
      engine's endpoint variant (removed for parity, not the cause); our objects
      being freed (the probe build shows no `release0`/`dtor` — the freed object
      was the engine's outer); a `PROPVARIANT`/media-type ownership bug (the
      structs are POD and every out-parameter matched the reference); and a
      `QueryInterface` sub-object offset difference (the oracle now compares the
      offsets for 14 interface ids and they match exactly).
      Supporting facts collected on the way, all still true:
      - binding **post-mix only** loads our DLL into `audiodg.exe` (the engine
        creates no instance until a stream needs it) and never crashed;
      - the registry the engine reads is byte-for-byte the reference's own
        `RegisterAPO` output for both class ids (`Flags=0xD`,
        `MaxInstances=0xFFFFFFFF`, `NumAPOInterfaces=1`,
        `APOInterface0={FD7F2B29-24D0-4B5C-B177-592C39F9CA10}`, same 12 value names
        in the same order), and the shipped build leaves it untouched;
      - the `Application Error` events for the failing runs all report
        `audioeng.dll`, `0xc0000005`, offset `0xFE868`.
      The multi-hundred-megabyte `.dmp` files were deleted after the analysis
      above; the `difftest/apo-backup-20261024/cdb_*.txt` transcripts, the live
      log captures and the capture scripts under `%TEMP%\eapo_*.ps1` were kept.
- [x] `target/release/EqualizerAPO.dll` is current: the `audiodg.exe` process
      that had an older build mapped has since recycled, so the release build can
      overwrite the DLL again.
- [ ] To rebuild the verification DLL used by the live child-APO test:
      `CARGO_TARGET_DIR=target-live cargo build -p eapo-apo --release --features live-probe`,
      register its class id as in `difftest/apo-backup-20261024/child-apo-test7.txt`
      and read the trace from the audio service's `%TEMP%\EqualizerAPO.log`.
- [x] Seven of the eight opt-in tests pass (`cargo test --workspace --
      --ignored`, run per crate): real audio-device enumeration, Win32
      global-hotkey round-trip, VST fixture instance/chunk APIs, VST filter
      processing, the native VST editor, the differential COM oracle above and
      `lock_and_process_match_the_reference_cpp_binary` (which also confirms our
      object and the reference reject the same synthetic connection descriptor
      with the same `E_INVALIDARG`).
- [ ] The sixth, `lock_and_process_audio_through_com`, is expected to fail and is
      kept only as documentation of the base-class contract: the WDK
      `CBaseAudioProcessingObject` rejects hand-built connection descriptors with
      `E_INVALIDARG`, and the reference C++ DLL returns the identical code for the
      same synthetic inputs, so a real lock needs a live audio graph (see the
      live child-APO item above).
- [x] DLL export table has exactly the four original exports.
- [x] Release binaries for all components build.
- [x] Editor GUI launches, loads existing C++ prefs, registers hotkeys, renders
      analysis identical to C++ (peak gain).
- [x] Benchmark output matches C++ for generated/WAV/FLAC inputs.
- [x] Installer tests + silent flags + admin manifest.
- [x] VST 2.4 host round-trip with a compiled test plugin (double processing,
      opaque chunk data, parameter/editor APIs, native editor window).
- [x] Editor translations generated from the original `.ts` files and
      verified visually in Chinese (menus, toolbar, filter chain, analysis,
      dialogs, standard buttons).
- [x] **Live APO processing in `audiodg.exe`: verified, without touching the
      render path.** EqualizerAPO's own child-APO feature (`HKLM\SOFTWARE\
      EqualizerAPO\Child APOs\<endpoint>` -> `PreMixChild`) makes the *already
      loaded* C++ APO `CoCreateInstance` a second APO and chain audio through it.
      Pointing that value at a fresh probe class id registered to the Rust build
      (deviation 8) loads our DLL into the running `audiodg.exe` — no
      `AudioSrv` restart, no speaker binding, so the user's audio never stops.
      Captured trace (`difftest/apo-backup-20261024/cap7-live-apolog.txt`,
      report `child-apo-test7.txt`) shows our object completing the real engine
      lifecycle inside the audio process:
      `Initialize` -> `APO GUID`/`Endpoint GUID` (real capture endpoint) ->
      `Input format in LockForProcess = { 00000003, 2, 4, 32, 48000.000000,
      00000003, 528 }` -> `InputFormat accepted` -> **`LockForProcess
      successful`** (`crates\apo\src\apo.rs`) -> config load from
      `D:\zmq13\config` with the 20 Hz/45 Hz peaking filters and the 15-node
      graphic equalizer (`crates\engine\...`), interleaved with the C++ parent's
      own `Successfully created and initialized child APO` and
      `LockForProcess successful`. Frame count, format and ordering match the
      C++ lines in the same log. Repeated runs (pids 4916/35196/9972/16768)
      reproduce it, and audio flowed through the chain into the recorded WAVs.
      Two behaviours found while getting there:
      - a child APO receives the **parent's** init block, so `APOInit.clsid` is
        always the parent's class id; chaining the endpoint's own child back into
        itself recurses forever (the C++ build recurses identically — 1676
        self-recreating instances in the control run), so the probe class id
        deliberately does not read a child (`treats_as_pre_mix`/`self.clsid`
        guard in `crates/apo/src/apo.rs`);
      - the audio engine caches a class object per CLSID inside `audiodg`, so
        repointing `InprocServer32` cannot load a new DLL into a running process,
        and `HKLM\...\MMDevices\Audio\*\FxProperties` is not writable even for
        Administrators without taking ownership. A previously unverified
        consequence: earlier runs in which the *render* endpoint stopped after
        `IsInputFormatSupported` used a process that had already cached the C++
        module for those CLSIDs. The render-side engine decision path is therefore
        still not reproduced; verifying it needs a fresh `audiodg` with the Rust
        build bound to a live speaker, which the user has ruled out.
- [ ] A third-party (non-fixture) VST plugin was not available on the
      verification machine.
