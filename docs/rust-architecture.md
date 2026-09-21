# Step 2 — Rust 架构

基于 `docs/architecture-analysis.md`。这不是 C# 类图的翻译。

产品：Windows 专用、极轻的 **Process Audio Router**。  
栈：Rust 2024 + Slint + windows-rs + WASAPI。  
MVP 范围见文末；托盘 / 自动重连 / 配置文件 / 快捷键 / 多进程混音 **不做**。

---

## 1. 一句话

用户选一个进程和一个输出设备，把该进程的声音经 Process Loopback 实时送到该设备。

```text
Target PID
    → WASAPI Process Loopback (shared, event)
    → convert to f32 @ render format
    → SPSC ring (fixed)
    → volume
    → WASAPI Shared Render
    → IMMDevice (CABLE Input / 耳机 / 任何 render)
```

---

## 2. 目录

C# 源码已移除，分析结论见 `docs/architecture-analysis.md`，原始文件见 git 历史。Rust 工程在仓库根目录，不另起 monorepo。

```text
Cargo.toml
build.rs
ui/main.slint
src/
  main.rs              CLI + UI 入口
  error.rs             统一错误
  process.rs           进程枚举
  audio/
    mod.rs
    com.rs             COM 初始化
    device.rs          输出设备枚举
    format.rs          WAVEFORMATEX 解析 / 声道 / 线性重采样
    buffer.rs          SPSC f32 ring
    capture.rs         Process Loopback 线程
    render.rs          WASAPI render 线程
    wav.rs             开发验证：PCM16 WAV 写出
  engine.rs            Start/Stop，拥有双线程
  ui_bridge.rs         Slint 属性 / 回调
docs/
  architecture-analysis.md
  rust-architecture.md
  step-*.md
```

不建空的 `app/`、`router/`、`config/`、`monitor.rs`、`mixer.rs`。音量在 render 线程乘；进程退出由 capture 错误上报。

---

## 3. 线程

```text
UI / main（Slint）
    command: Start{pid, device_id} | Stop | SetVolume
        ↓  mpsc + atomics
AudioEngine
    ├─ capture thread   COM MTA，只 push ring
    └─ render thread    COM MTA，只 pop ring + 写设备
         SPSC ring 在两者之间
```

实时线程禁止：heap 扩容、文件、网络、UI、锁、tracing 刷屏。  
ring 预分配；转换用线程启动时拿好的暂存 `Vec<f32>`（固定 cap，不清 cap）。

关停：`AtomicBool` + `WaitForSingleObject` 超时 50–100ms，以便看到 stop。`Drop` join。

COM：音频线程 `CoInitializeEx(COINIT_MULTITHREADED)`。已初始化则忽略 `RPC_E_CHANGED_MODE`。UI 线程不强制改 apartment。

---

## 4. 音频细节

### 4.1 Capture

Windows 10 20348+：

- `ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, IAudioClient, PROPVARIANT(VT_BLOB of AUDIOCLIENT_ACTIVATION_PARAMS), handler)`
- `ActivationType = PROCESS_LOOPBACK`
- `TargetProcessId = pid`
- `ProcessLoopbackMode = INCLUDE_TARGET_PROCESS_TREE`
- `IAudioClient::Initialize(SHARED, LOOPBACK | EVENTCALLBACK, 0, 0, mixFormat, NULL)`
- `IAudioCaptureClient` + event handle

`unsafe` 只留在 `audio/capture.rs` 与 `audio/render.rs`、COM 回调 impl。

### 4.2 Render

- `IMMDeviceEnumerator::GetDevice(id)` → `Activate IAudioClient`
- mix format、SHARED + EVENTCALLBACK
- `IAudioRenderClient`
- ring 空：写静音；ring 满：capture 丢最旧，保活延迟

### 4.3 内部格式

不硬编码 48k / stereo / f32。  
ring 存 **render mix 的 f32 交错帧**。capture 线程做：

- PCM/float/extensible → f32
- 声道 upmix/downmix（mono↔N，>2 降到 render 声道，多出来的声道丢掉或折到 stereo）
- 采样率不同：线性插值，状态留在 capture 线程

MVP 不引入 FFmpeg / rubato。

### 4.4 音量

`AtomicU32` 存 `f32.to_bits()`。render 出设备前相乘，clamp 到 0..=1。不在 UI 存权威值。

---

## 5. 状态

```text
selected_pid: Option<u32>
selected_device_id: Option<String>    // WASAPI 设备 ID，不是 MME 下标
volume: f32
running: bool
status: Idle | Running | ProcessExited | DeviceGone | Error
error_message: String                  // 给 UI 的短句
```

权威在 `AudioEngine` / `AppModel`。Slint 只镜像。

UI 短句示例：

- `无法连接到音频设备`
- `进程已退出`
- `无法捕获该进程的音频`
- `设备可能已经被拔出。`

详细 HRESULT 走 tracing，不进音频线程热路径。

---

## 6. 错误

```rust
enum Error {
    Com(windows::core::Error),
    ProcessNotFound(u32),
    DeviceNotFound,
    CaptureInit(&'static str),
    RenderInit(&'static str),
    Format(&'static str),
    Io(std::io::Error),
    TimedOut(&'static str),
}
```

用户路径与音频循环禁止 `unwrap` / `expect` / `panic`。  
COM 失败映射为上述类型。

---

## 7. 依赖审查

| crate | 理由 | 能否 Win32 替代 | 体积 | 结论 |
|---|---|---|---|---|
| `windows` | WASAPI / COM / ToolHelp / 窗口标题 | 自己绑 FFI 更差 | 编译期为主 | **要** |
| `slint` + `slint-build` | 原生 UI，无 WebView | 纯 Win32 对话框能做但开发差 | 最大一档；用 software renderer 压 | **要** |
| `tracing` + `tracing-subscriber` | 开发日志 | `eprintln` 也能 | 小 | 开发要，音频热路径不用 |
| `serde` / `serde_json` | 配置文件 | 第二阶段 | — | **MVP 不引入** |
| `clap` / `anyhow` / `thiserror` / `rtrb` / `hound` / `cpal` / `wasapi` | 方便 | 标准库 + 手写 | 避免 | **不要** |
| `tokio` | 无网络异步需求 | — | 大 | **不要** |

Slint features（目标最小）：`std`、`backend-winit`、`renderer-software`、`compat-1-2`。不用 femtovg/skia/qt。

Release profile：

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Step 9 对比 `opt-level = "z"`；若体积收益小、CPU 变差则维持 `3`。

---

## 8. CLI（无 UI 验证）

无额外 CLI 库。`std::env::args`：

```text
mintcat-proc-mic.exe                     启动 UI
--list-processes
--list-devices
--capture <pid> --wav <path> --seconds N
--route <pid> --device <id>
```

Step 5 用 `--capture` 证 Process → WAV。  
Step 6 用 `--route` 证实时转发。

---

## 9. UI

`ui/main.slint` 单文件，不拆 components（三个控件不够拆）。

- 进程列表：`exe  [pid]  title`，有窗口的排前面
- 设备列表：FriendlyName，默认设备标注
- 音量 0–100
- 状态点 + 短句
- 主按钮：开始转发 / 停止
- 刷新：重枚举进程和设备

定时器 ~1s 从引擎抽状态（进程退出、设备失效）。不把 PCM 送 UI。

---

## 10. 进程枚举

`CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`。  
标题：`EnumWindows` + `GetWindowThreadProcessId` + `GetWindowTextW`，仅可见顶层。

跳过 PID 0/4、空名。不按 DRG 过滤。不硬编码游戏名。

---

## 11. 模块边界

```text
ui_bridge  →  engine  →  capture/render/buffer/format
                ↑
         process / device
```

`engine` 不引用 Slint。`capture` 不引用 UI。Windows 字符串/HRESULT 不出 `audio` 模块边界（除 `Error`）。

---

## 12. MVP 验收（Step 8 结束必须满足）

```text
启动 → 看到进程 → 选一个正在出声的进程
     → 看到设备 → 选目标
     → 开始 → 目标设备听到该进程
     → 音量有效
     → 停止后无声
```

额外手动：进程退出、无音频进程、高音量、短时间连续启停。  
热插拔自动恢复、开机启动、托盘：第二阶段。

---

## 13. 明确不做（当前阶段）

Electron / Tauri / WebView / FFmpeg / GStreamer / SQLite / 网络 API / 登录 / 自动更新 / 插件 / 自研虚拟声卡 / DRG Mod / TTS / 命名管道。

---

## 14. 工具链（本机）

已核实，不是计划：

| 项 | 值 |
|---|---|
| rustc | 1.98.1 (`stable-x86_64-pc-windows-msvc`) |
| rustup | 1.29.1 |
| 目标 | `x86_64-pc-windows-msvc` |
| MSVC | VS Community 2022 17.14，`cl` 19.44.35217，`D:\Program Files\Microsoft Visual Studio\2022\Community` |
| SDK | `D:\Windows Kits\10` `10.0.26100.0` |
| Edition | 2024（1.98 支持） |

Step 2 完成。下一步：最小 Cargo 工程 + 进程枚举。
