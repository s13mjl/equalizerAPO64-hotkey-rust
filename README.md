# EqualizerAPO64-hotkey — pure-Rust port

`equalizerAPO64-hotkey-rust/` is a pure-Rust reimplementation of the
[EqualizerAPO](https://sourceforge.net/projects/equalizerapo/) fork
`equalizerAPO64-hotkey`, ported line by line for identical behaviour.

Everything lives in that directory:

- `crates/apo` — the APO COM object (with a thin WDK `CBaseAudioProcessingObject`
  shim) and `EqualizerAPO.dll`
- `crates/engine`, `crates/filters`, `crates/expression` — configuration parser,
  filter chain and DSP
- `crates/editor`, `crates/device-selector`, `crates/setup`, … — the applications
- `PARITY.md` — per-file comparison with the C++ original, deviations, and the
  verification evidence (COM differential against the reference binary, live
  `audiodg.exe` captures, crash analyses)

```powershell
cd equalizerAPO64-hotkey-rust
cargo build --release          # binaries in target\release
cargo test  --workspace        # unit + integration tests
```

Building the APO requires the MSVC toolset and the Windows SDK; `build.rs`
compiles the small C++ base-class shim with `cl.exe`/`lib.exe` and restricts the
export table to the original four entry points.

The upstream C++ sources are not included. Licensed under the GNU General Public
License v2 — see `equalizerAPO64-hotkey-rust/License.txt`.
