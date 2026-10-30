[English](README.md) | 简体中文

# EqualizerAPO64-hotkey —— 纯 Rust 移植版

`equalizerAPO64-hotkey-rust/` 是对 EqualizerAPO 分支
[equalizerAPO64-hotkey](https://sourceforge.net/projects/equalizerapo/) 的
纯 Rust 重实现，逐行移植，行为保持一致。

所有内容都在该目录下：

- `crates/apo` —— APO COM 对象（含一层精简的 WDK
  `CBaseAudioProcessingObject` 中间层）以及 `EqualizerAPO.dll`
- `crates/engine`、`crates/filters`、`crates/expression` —— 配置解析器、
  过滤器链与 DSP
- `crates/editor`、`crates/device-selector`、`crates/setup` 等 —— 各个应用程序
- `PARITY.md` —— 与 C++ 原版的逐文件对比、差异说明及验证证据（针对参考二进制
  的 COM 差分测试、`audiodg.exe` 实时抓取、崩溃分析）

```powershell
cd equalizerAPO64-hotkey-rust
cargo build --release          # 生成的二进制位于 target\release
cargo test  --workspace        # 单元测试 + 集成测试
```

构建 APO 需要 MSVC 工具链和 Windows SDK；`build.rs` 通过
`cl.exe`/`lib.exe` 编译小型 C++ 基类中间层，并将导出表限制为原有的四个入口点。

上游 C++ 源码未包含在内。本项目按 GNU General Public License v2 授权——
参见 `equalizerAPO64-hotkey-rust/License.txt`。
