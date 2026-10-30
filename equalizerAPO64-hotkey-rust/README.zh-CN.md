[English](README.md) | 简体中文

# EqualizerAPO64-Hotkey —— 纯 Rust 移植版

本项目将
[EqualizerAPO64-Hotkey](https://github.com/s13mjl/equalizerAPO64-hotkey)
（EqualizerAPO 的一个分支，在编辑器中提供**每个过滤器的全局热键**）
**1:1 地按功能移植**为 **100% Rust**，不依赖 C++、Qt、FFTW、libsndfile
或 muparserx。

Windows 音频处理对象（APO）、编辑器界面、设备选择器、安装程序、更新程序、
Voicemeeter 客户端和基准测试工具均已用 Rust 重写，并保留原版的行为、
配置格式、注册表布局和过滤器语义。

## 功能

- **每个过滤器的全局热键** —— 可为任意过滤器分配系统级快捷键（例如
  `Alt+X`），从任何应用程序中切换其启用状态，即使编辑器处于后台。
  - 通过 Win32 `RegisterHotKey`（`MOD_NOREPEAT`）进行全局注册。
  - 使用经典 `GetMessage` 循环的独立工作线程；这是必需的，因为现代
    GUI 事件循环不会把 `WM_HOTKEY` 派发给应用的本地事件过滤器。
  - 每个过滤器的快捷键存储在
    `HKCU\Software\EqualizerAPO\Hotkeys\<filterId>\keySequence`。
  - 冲突检测与设置持久化。
  - 热键诊断日志位于 `%TEMP%\equalizerapo_hotkey.log`。
- 保留全部上游功能：
  - 64 位双精度音频处理（全程 `f64`）。
  - 完整的 EqualizerAPO 配置格式：
    `Preamp`、`Filter`（PK/PEQ/Modal/LP/HP/LPQ/HPQ/BP/LS/HS/LSC/HSC/NO/AP）、
    `IIR`、`GraphicEQ`、`Convolution`、`Copy`、`Delay`、`Channel`、`Stage`、
    `Device`、`If`/`ElseIf`/`Else`/`EndIf`、`Eval` 以及行内表达式、
    `Include`、`VSTPlugin`、`LoudnessCorrection`。
  - 与 EqualizerAPO 所用 muparserx 子集兼容的表达式语言（变量、数组、
    字符串、正则表达式、注册表访问、三目运算符、赋值），以安全 Rust 实现。
  - 文件/注册表变更监控，并在配置切换时无缝交叉淡化。
  - 编辑器支持过滤器链编辑、分析曲线、各过滤器的图形界面、拖放、
    全局热键，以及原有的德语/法语/中文翻译。
  - VST 2.4 插件宿主（双精度、chunk/参数状态、原生插件编辑窗口）。
  - WAV、FLAC 和 OGG/Vorbis 脉冲响应及基准测试输入。
  - DeviceSelector、UpdateChecker、VoicemeeterClient、Benchmark 和 Setup。

## 工作区结构

| Crate | 原版对应部分 |
|-------|----------------------|
| `crates/common` | `helpers/`（`RegistryHelper`、`StringHelper`、`ChannelHelper`、`GainIterator`、`LogHelper`、`ServiceHelper`、`TaskSchedulerHelper`、`AbstractLibrary`、VST 宿主辅助代码） |
| `crates/expression` | muparserx + `parser/`（`LogicalOperators`、`StringOperators`、`RegexFunctions`、`RegistryFunctions`） |
| `crates/filters` | `filters/` 和 `libHybridConv-0.1.1/` |
| `crates/engine` | `FilterEngine`、`FilterConfiguration`、所有 `*FilterFactory` |
| `crates/apo-info` | `AbstractAPOInfo`、`DeviceAPOInfo`、`VoicemeeterAPOInfo` |
| `crates/apo` | `EqualizerAPO/`（COM APO DLL） |
| `crates/editor` | `Editor/`（GUI + 全局热键管理器） |
| `crates/device-selector` | `DeviceSelector/` |
| `crates/update-checker` | `UpdateChecker/` |
| `crates/voicemeeter-client` | `VoicemeeterClient/` |
| `crates/benchmark` | `Benchmark/` |
| `crates/setup` | `Setup/`（NSIS 安装程序与卸载程序） |

## 构建

需要 Windows 10/11 x64，以及带有 `x86_64-pc-windows-msvc` 目标的 Rust
工具链。

```cmd
cargo build --release --workspace
```

生成的二进制位于 `target\release`：

- `EqualizerAPO.dll`（COM APO，`crates/apo`）
- `Editor.exe`（原生 Win32/GDI 前端，`crates/editor-native`；原 egui 前端
  仍以 `EditorEgui.exe` 提供，见 `PARITY.md`）
- `DeviceSelector.exe`
- `UpdateChecker.exe`
- `VoicemeeterClient.exe`
- `Benchmark.exe`
- `Setup.exe`

运行 `build.bat` 可构建整个工作区，将所有二进制暂存到
`installer-output\files`，并生成**单文件安装程序**
`installer-output\Setup.exe`（即在 `Setup.exe` 镜像后追加暂存载荷；
`PackPayload.exe` 负责构建和检查该文件）。发布版可执行文件会在安装期间
把载荷解压到 `%TEMP%`，安装结束后删除。

## 使用

1. 使用 `Setup.exe` 安装（或复制二进制文件，并用 DeviceSelector 注册 APO）。
2. 打开编辑器。
3. 在过滤器链中右击某个过滤器 → **设置热键**（或使用该过滤器界面中的控件）。
4. 按下所需的组合键（例如 `Alt+X`）。
5. 让编辑器在后台运行，按下热键——该过滤器即被切换。

热键存储在
`HKCU\Software\EqualizerAPO\Hotkeys\<filterId>\keySequence`。

## 实现说明

原版 C++ 使用 WDK 的 `CBaseAudioProcessingObject`。Rust 版 APO 直接实现
COM 接口（`IAudioProcessingObject`、`IAudioProcessingObjectRT`、
`IAudioProcessingObjectConfiguration`、`IAudioSystemEffects`），并提供与
原版 ABI 兼容的虚表。

DSP 核心以 `f64` 样本处理，并复现原版每种双二阶滤波器、动态响度滤波器、
分块 FFT 卷积、复制/路由与延迟语义的系数公式。配置重载在音频线程之外构建，
并以 10 ms 交叉淡化切换，与原版完全一致。

热键功能使用专用的 Win32 消息循环线程。这是必需的，因为 GUI 事件循环不用
`GetMessage` 取消息，否则 `WM_HOTKEY` 永远不会到达用户代码。工作线程通过
投递的线程消息注册/注销热键，并同步返回结果。

## 许可证

GPL-2.0-or-later。参见 [License.txt](License.txt)。原 EqualizerAPO 版权归
Jonas Thedering 及贡献者所有；热键分支新增部分的版权归 EqualizerAPO
Contributors 所有。
