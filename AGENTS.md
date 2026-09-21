# ProcessMic 智能体约束

给在本仓库改代码的 agent 用。先读本文件，再动代码。权威需求：`goal.md`。架构事实：`docs/architecture-analysis.md`、`docs/rust-architecture.md`。

产品：Windows 专用、极轻的 **Process Audio Router**（ProcessMic）。  
栈：Rust 2024 + Slint + windows-rs 0.62 + WASAPI。  
不是「Rust 版 C# 项目」。C# / WPF 源码只作功能与边界参考，禁止 class→struct 机械翻译。

---

## 0. 用户怎么说就怎么做

- 用户的话是绝对的。报告的错误、现象、文件状态直接当真，不要复跑去「确认」用户已经贴出来的日志。
- **只做被要求的事。** 不要顺手改交互、改文案、改设备分类、改默认麦、改主题、加映射、加提示。
- 没要求「映射」「配对展示」「过滤某类设备」就不要做。输出列表曾经被擅自写成 `扬声器 → 麦 …`，用户明确反对。
- 有多种做法且取舍差很大时再问；能从仓库惯例得出的默认直接做。
- 被否决后立刻改回去，不要再解释一遍为什么你那版更好。

---

## 1. 开始前（禁止直接写业务代码）

按顺序：

1. 读 `goal.md` 对应章节（原则、禁止项、当前阶段）。
2. 读 `docs/rust-architecture.md` 和本文件「已知陷阱」。
3. 读将改模块的**完整源文件**，不是只读函数签名。改导出符号前查调用点。
4. 复述：要改什么、不改什么、怎么验证。范围不清就停，先问。
5. 新功能：对照 `docs/architecture-analysis.md` 里「可删 / DRG 特化 / 通用」分类。DRG、命名管道、TTS、Mod 安装不要进 `engine` / `audio`。

不要一次生成整个项目。按 `goal.md` 第三十四节的 Step 推进；已完成的 Step 见 `docs/step-*.md`，不要重做。

---

## 2. 硬禁止

```text
Electron / Tauri / WebView / Node / Chromium / React / Vue
FFmpeg / GStreamer / cpal / rodio / rubato / 大型音频框架
自己写虚拟声卡驱动
数据库 / 云 / 登录 / 自动更新 / 插件 / 网络 API
空壳模块（app/ router/ monitor.rs 等「好看」目录）
音频热路径 unwrap / expect / panic / tracing 刷屏 / 文件 / 网络 / UI / 锁
把核心音频状态塞进 Slint
把 Deep Rock Galactic 硬编码进 AudioEngine
```

每加一个 Cargo 依赖必须能回答：为什么需要、能否 Win32 直接做、体积、内存、有无更轻替代。  
默认只允许：`windows`、`windows-core`、`slint`、`tracing`、`tracing-subscriber`。`slint` 会把 `windows` 拉到 **0.62**，禁止拆版本。

---

## 3. 代码规范

### 3.1 语言与模块

- Edition 2024。`clippy` 尽量干净。改完对碰过的文件跑 `cargo fmt`（通过 `scripts\build-dev.cmd`）。
- 模块按职责，能合并就合并。现树：

```text
src/main.rs          CLI + UI 入口
src/engine.rs        Start/Stop，拥有捕获/渲染线程
src/error.rs         统一错误；UI 只用 user_message()
src/process.rs       进程枚举
src/config.rs        热键等小配置
src/hotkey.rs        RegisterHotKey
src/ui_bridge.rs     Slint 属性与回调
src/vbcable.rs       VB-CABLE 安装
src/audio/
  com.rs device.rs format.rs buffer.rs
  capture.rs render.rs peak.rs policy.rs wav.rs
ui/main.slint
```

C# / WPF 源码已从工作树移除，原始文件见 git 历史，分析结论留在 `docs/architecture-analysis.md`。不要「翻译」回 Rust 模块。

### 3.2 注释与命名

- 模块顶：`//!` 一句话职责。
- 每个 `pub struct` / `enum` 变体 / `pub fn` / 结构体字段：`///` 说明**不变量或用途**，不要复述标识符。
- 注释用中文（本仓库惯例）。
- 标识符英文。WASAPI 设备 ID 当不透明字符串，不当 MME 下标。

### 3.3 错误

- 用户路径、音频路径禁止 `unwrap` / `expect` / `panic`。
- 细节走 `Error` 的 `Display` + `tracing::warn!`；界面只显示 `user_message()` 短句。
- HRESULT 常见映射：
  - `0x80070057` `E_INVALIDARG`：格式块不合法（常见：WAVEFORMATEXTENSIBLE 只拷了 18 字节头）。
  - `0x88890021` `AUDCLNT_E_UNSUPPORTED_FORMAT`：Initialize 格式/标志不被接受。
  - `0x88890004` / `0x88890013`：设备失效或进程会话结束 → `ProcessExited` / `DeviceGone`。

### 3.4 unsafe 与 COM

- `unsafe` 只放 `audio/capture.rs`、`audio/render.rs`、`audio/com.rs`、COM 回调、热键窗口过程。
- 上层 `AudioEngine` 保持安全 API。
- 每块 `unsafe` 写清：别名、生命周期、谁释放、哪个线程的 apartment。
- 音频线程：`CoInitializeEx(COINIT_MULTITHREADED)`。已初始化则忽略 `RPC_E_CHANGED_MODE`。
- Process Loopback 的 `PROPVARIANT(VT_BLOB)` 必须 `ManuallyDrop`，否则 Drop 会 `CoTaskMemFree` 栈上 blob。
- `IAudioClient::Initialize` 失败后该 client **不能复用**，必须重新 `ActivateAudioInterfaceAsync`。

### 3.5 音频线程

```text
UI 线程  --命令-->  AudioEngine
                      ├ capture（进程环回）  ─┐
                      ├ capture（可选物理麦）─┼→ SPSC ring（预分配 f32）
                      └ render               ←┘  音量/混音后写设备
```

实时线程禁止：heap 扩容、`Vec` 增长、文件、网络、UI、阻塞锁、打日志、克隆大缓冲。  
ring 固定容量；转换缓冲在线程启动时分配，循环内只 `clear`/`extend` 到已有 cap。  
关停：`AtomicBool` + `WaitForSingleObject` 50–100ms 超时，`Drop` 里 `join`。

内部：ring 存 **render mix 的交错 f32**。采样率/声道以设备为准，不硬编码 48k。重采样只做轻量线性插值，不引入 FFmpeg。

物理麦克风 **不能** 当 WASAPI 播放终点。要给人声 + 进程一起进 Discord：进程环回 + 麦捕获 → 混音 → 虚拟线缆的 **播放端**；Discord 选对应 **录音端**。

### 3.6 UI

- Slint 只展示。权威状态在 `AudioEngine` / `ui_bridge` 的 `Rc`/`Arc`。
- 横版、浅色、控件少。不要动画、毛玻璃、主题系统、图片包。
- 输出下拉：**每只虚拟扬声器一条、每只虚拟麦克风一条**，文案用 Windows 友好名称（用户改名后就是新名字）。禁止拼 `A → 麦 B`。
- 虚拟设备判定看 **驱动/适配器/DeviceDesc**（`VB-Audio`、`CABLE`、`Voicemeeter`、`Steam Streaming`），不要只看用户改过的 FriendlyName。
- 选中虚拟麦克风时，内部才解析「该往哪只播放端写」；这不是 UI 映射。
- 「设为系统默认麦克风」是可选项，默认不要改系统默认麦，除非用户勾了。
- 不要过滤用户没要求过滤的设备（包括 Steam 虚拟设备）。

### 3.7 依赖与构建

本机无独立 rustup PATH 时，用：

```bat
scripts\build-dev.cmd build
scripts\build-dev.cmd build --release
```

该脚本先 `vcvars64.bat` 再 `cargo`，避免 Git 的 `link.exe` 抢 PATH。  
`windows-rs` 新 Win32 模块要在 `Cargo.toml` features 里打开。  
目标 exe 正在运行时 `cargo` 无法覆盖，报 `拒绝访问 (os error 5)`：先让用户关掉再编。

---

## 4. 开始后（实现中）

- 先改现有文件，少造新文件。
- 一次一条用户可感知的行为，不要「趁机重构」。
- 音频改动：先保证 Initialize 成功，再谈混音/UI。
- 设备列表改动：用 `mintcat-proc-mic.exe --list-devices` / `--list-processes` 看真实名称，不要靠猜测。
- 用户贴了 HRESULT / 日志：对着代码改那条路径，不要开新抽象。

---

## 5. 结束后（交付前）

每阶段都必须**真跑**，`cargo check` 不算完成。

1. `scripts\build-dev.cmd build`（或 `--release`，按 `goal.md` 第三十二节）。
2. 覆盖刚改的路径：启动 UI，或 `--list-devices` / `--capture` / `--route`。
3. 进程退出、设备失效、Initialize 失败：UI 短句 + tracing 细节，线程不崩。
4. 阶段结束写 `docs/step-XX-*.md`：做了什么、怎么跑的、实测结果（RAM/CPU/体积仅 Step 9 需要）。不要空模板。
5. 同步 `docs/rust-architecture.md` 里被你改掉的事实（线程、格式、目录）。过期描述比没文档更糟。
6. 不要提交用户没要的 md、测试垫片、脚手架。

第一阶段验收（`goal.md` 三十三）：选进程 → 选设备 → 开始 → 目标设备有声 → 音量有效 → 停止即停。

---

## 6. 已知陷阱（本仓库踩过）

| 现象 | 原因 | 处理 |
|---|---|---|
| `0x80070057` 参数错误 | 把 render `GetMixFormat` 的 WAVEFORMATEXTENSIBLE 只 `*src` 成 `WAVEFORMATEX`，`cbSize=22` 尾巴丢失 | `sanitize_wave`：做成 `cbSize=0` 的 PCM 或 IEEE float |
| `0x88890021` 无法初始化音频系统 | 进程环回 Initialize 去掉 `AUDCLNT_STREAMFLAGS_LOOPBACK` | 本机可用：`44100 PCM16` + `LOOPBACK\|EVENTCALLBACK\|AUTOCONVERTPCM\|SRC_DEFAULT_QUALITY` |
| 虚拟麦从列表消失 | 按友好名里的 `CABLE Output` 匹配，用户改名后失败 | 用驱动名/DeviceDesc 认虚拟设备；列表用 FriendlyName |
| 朋友只听到真麦 | Discord 仍选物理麦，或 Krisp 把进程声当噪音 | 选虚拟麦克风那一项；默认麦是勾选才改；提示关噪音抑制 |
| 输出出现 `扬声器 → 麦` | 擅自做设备映射文案 | **禁止**。扬声器一条、麦克风一条 |
| `os error 5` 无法删 exe | 旧进程占用 | 先退出 UI 再 build |

Process Loopback：`ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, … INCLUDE_TARGET_PROCESS_TREE)`。不要用普通 endpoint 的 loopback 去「猜」进程。

---

## 7. 阶段文档模板

`docs/step-XX-short-name.md`：

```text
# Step X — 标题
范围：做了 / 明确没做
改动文件：
怎么跑：命令 + 预期
实测：日期、机器、结果（成功/失败/HRESULT）
下一步：
```

---

## 8. 优先级

```text
小、快、稳、原生、低内存、低磁盘
>
功能堆砌、UI 炫技、跨平台、复杂架构、多 crate
```
