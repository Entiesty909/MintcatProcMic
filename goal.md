# Rust 原生轻量 SoundPad 重构任务

## 一、项目背景

当前项目需要将已有的 C# / WPF 项目：

D:\code\MintcatProcMic

重新实现为一个 **Rust 原生 Windows 桌面应用**。

原项目是一个面向 Deep Rock Galactic（DRG）的 SoundPad / 音频播放工具，当前使用 C# + WPF 实现。

不要简单地把 C# 代码逐行翻译成 Rust。

你的任务是：

> **先完整理解原 C# 项目的功能、数据流、音频处理方式和 UI 交互，再使用 Rust 重新设计一个轻量、低内存、低磁盘占用的 Windows 原生版本。**

原项目仅作为：

- 功能参考
- 行为参考
- 音频逻辑参考
- UI 交互参考
- 边界情况参考

Rust 版本应该重新设计架构，而不是机械移植。

------

# 二、核心目标

最终项目定位：

> 一个非常轻量的 Windows SoundPad / Process Audio Router。

目标不是制作一个庞大的 SoundPad，而是：

1. Windows 原生
2. Rust 实现
3. 极低内存占用
4. 尽可能小的磁盘占用
5. 启动速度快
6. 不依赖 Electron
7. 不依赖 Node.js
8. 不使用 WebView
9. 不使用 Tauri
10. 不使用大型桌面运行时
11. UI 简洁
12. 音频处理稳定
13. 尽可能减少第三方依赖

最终希望得到：

```text
Rust
 +
Slint
 +
windows-rs
 +
WASAPI
```

的轻量 Windows 应用。

------

# 三、重要原则

## 1. 不要机械翻译 C#

禁止：

```text
C# class → Rust struct
C# method → Rust function
WPF event → Rust callback
```

然后简单拼起来。

必须先理解：

```text
原项目为什么这样设计
↓
真正需要的功能是什么
↓
Rust 中最合适的实现是什么
```

------

## 2. 原项目是参考，不是架构模板

如果 C# 项目中存在：

- WPF 特有实现
- .NET 特有机制
- 不必要的抽象
- 不必要的线程
- 不必要的对象
- 可以删除的功能

Rust 版本不要照搬。

可以重新设计。

------

## 3. 优先考虑资源占用

每引入一个依赖，都要考虑：

- 是否真的需要
- 是否会增加最终体积
- 是否增加运行时内存
- 是否增加启动时间
- 是否可以使用 Windows 原生 API 替代

不要为了方便随便引入大型库。

------

# 四、技术栈

## 核心语言

```text
Rust
Edition 2024
```

------

## UI

优先使用：

```text
Slint
```

原因：

- Rust 原生
- 声明式 UI
- 不需要 Electron
- 不需要 Node.js
- 不需要 WebView
- 不需要 HTML/CSS/JavaScript
- UI 编译进应用
- 适合轻量桌面工具

不要使用：

```text
Electron
Tauri
Dioxus Desktop
WebView
Chromium
Node.js
```

除非后续经过明确评估后发现技术上确实无法实现目标。

------

# 五、Windows API

使用：

```text
windows-rs
```

优先直接调用 Windows API。

重点关注：

```text
Win32
COM
Windows Audio
WASAPI
Process API
```

------

# 六、音频架构

音频系统是整个项目最重要的部分。

优先采用：

```text
WASAPI
```

不要为了跨平台而引入复杂的跨平台音频抽象。

这是 Windows 专用软件。

允许充分使用 Windows 原生音频 API。

------

# 七、Process Audio Capture

项目的核心需求之一：

> 能够获取指定进程产生的声音。

优先研究并使用 Windows 的：

```text
WASAPI Process Loopback
```

重点研究：

```text
AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK
AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS
TargetProcessId
```

目标：

```text
指定进程
    ↓
Process Loopback
    ↓
PCM Audio
```

而不是默认捕获整个系统输出。

------

# 八、音频处理架构

建议设计为：

```text
┌─────────────────────┐
│ Process Audio       │
│ Capture             │
│ WASAPI Loopback     │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ Audio Buffer        │
│ SPSC Ring Buffer    │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ Audio Processing    │
│ Volume / Mixing     │
└──────────┬──────────┘
           │
           ▼
┌─────────────────────┐
│ WASAPI Renderer     │
└──────────┬──────────┘
           │
           ▼
      Target Device
```

------

# 九、线程设计

UI 线程和音频线程必须分离。

建议：

```text
UI Thread
    │
    │ command
    ▼
Audio Controller
    │
    ├───────────────┐
    ▼               ▼
Capture Thread   Render Thread
    │               │
    └───────┬───────┘
            ▼
        Ring Buffer
```

音频实时线程：

禁止：

- 大量内存分配
- 频繁创建对象
- 阻塞式锁
- 文件 IO
- 网络 IO
- UI 操作
- 日志刷屏

------

# 十、Ring Buffer

优先自己实现轻量的：

```text
SPSC Ring Buffer
```

即：

```text
Single Producer
Single Consumer
```

用于：

```text
Capture → Render
```

避免在音频实时路径中频繁使用：

```text
Mutex<Vec<_>>
```

避免：

```text
Vec 不断扩容
```

音频 buffer 应该尽可能固定大小。

------

# 十一、音频格式

内部音频格式尽量统一。

优先考虑：

```text
48 kHz
Stereo
f32
```

但不要硬编码。

必须能够根据 WASAPI 实际设备格式进行适配。

需要处理：

- Sample Rate
- Channel Count
- Sample Format
- Buffer Size

如果 Capture 和 Render 的格式不同，再考虑加入轻量 Resampler。

不要为了这个功能直接引入 FFmpeg。

------

# 十二、不要使用 FFmpeg

本项目当前不需要：

```text
FFmpeg
GStreamer
大型音频框架
```

如果只是：

```text
PCM
音量
混音
采样率处理
WASAPI
```

直接 Rust + Windows API 实现。

------

# 十三、进程管理

需要提供：

```text
ProcessManager
```

负责：

- 枚举进程
- PID
- Process Name
- Window Title
- Process Path（如果需要）
- 进程退出检测
- 进程重新启动检测

UI 可以显示：

```text
Minecraft.exe
Discord.exe
Chrome.exe
DeepRockGalactic.exe
```

如果进程已经退出：

```text
状态：进程已退出
```

而不是让音频线程崩溃。

------

# 十四、自动重连

例如：

```text
Minecraft.exe
     ↓
开始转发
     ↓
Minecraft 退出
     ↓
Audio Session 结束
     ↓
等待 Minecraft
     ↓
Minecraft 重新启动
     ↓
自动重新建立 Capture
```

这个功能应该从架构上预留。

------

# 十五、输出设备

需要枚举 Windows 音频设备。

例如：

```text
扬声器
耳机
CABLE Input
VoiceMeeter Input
其他 Render Device
```

用户可以选择：

```text
目标输出设备
```

音频最终：

```text
Process
   ↓
Process Loopback
   ↓
Rust Audio Engine
   ↓
WASAPI Render
   ↓
Selected Device
```

不要自行实现虚拟声卡驱动。

第一阶段直接支持已有的：

```text
VB-CABLE
VoiceMeeter
Virtual Audio Cable
```

等虚拟音频设备。

------

# 十六、UI

使用 Slint。

UI 不需要复杂。

目标：

```text
┌──────────────────────────────────┐
│ ProcessMic / DRG SoundPad       │
├──────────────────────────────────┤
│                                  │
│ 音频来源                         │
│ ┌──────────────────────────────┐ │
│ │ DeepRockGalactic.exe      ▼ │ │
│ └──────────────────────────────┘ │
│                                  │
│ 输出设备                         │
│ ┌──────────────────────────────┐ │
│ │ CABLE Input              ▼ │ │
│ └──────────────────────────────┘ │
│                                  │
│ 音量                             │
│ ━━━━━━━━━━━●━━━━━━━━━━━ 100%    │
│                                  │
│ 状态                             │
│ ● 未运行                         │
│                                  │
│       ┌────────────────┐         │
│       │    开始转发    │         │
│       └────────────────┘         │
│                                  │
└──────────────────────────────────┘
```

第一阶段不要做复杂动画。

不要做：

- 大量图片
- Web UI
- CSS
- GPU 特效
- 毛玻璃
- 大型主题系统

优先保证：

```text
简单
稳定
低内存
低 CPU
```

------

# 十七、架构目录

建议：

```text
src/
├── main.rs
│
├── app/
│   ├── mod.rs
│   └── state.rs
│
├── process/
│   ├── mod.rs
│   ├── manager.rs
│   └── monitor.rs
│
├── audio/
│   ├── mod.rs
│   ├── device.rs
│   ├── process_loopback.rs
│   ├── capture.rs
│   ├── renderer.rs
│   ├── buffer.rs
│   ├── format.rs
│   └── mixer.rs
│
├── router/
│   ├── mod.rs
│   └── audio_router.rs
│
├── config/
│   ├── mod.rs
│   └── settings.rs
│
└── ui/
    ├── mod.rs
    └── bridge.rs

ui/
├── main.slint
└── components/
    ├── process_selector.slint
    ├── device_selector.slint
    └── volume_control.slint
```

如果实际开发过程中发现某些模块过度设计，可以合并。

不要为了“架构好看”创建大量空壳模块。

------

# 十八、状态管理

建议 Rust 维护核心状态：

```text
AppState

selected_process
selected_device
volume
is_running
capture_status
render_status
error_message
```

Slint 只负责展示和交互。

不要把核心音频状态塞进 UI。

------

# 十九、错误处理

不要：

```rust
unwrap()
expect()
panic!()
```

出现在长期运行路径和用户操作路径中。

必须正确处理：

- 进程不存在
- 进程退出
- 音频设备不存在
- 音频设备被占用
- WASAPI 初始化失败
- COM 初始化失败
- Capture 初始化失败
- Render 初始化失败
- 设备被拔出
- 采样率不兼容
- Audio Session 结束

UI 给出简单错误：

```text
无法连接到音频设备
设备可能已经被拔出。
```

详细错误写入 debug 日志。

------

# 二十、日志

开发阶段允许使用：

```text
tracing
```

但 Release 版本：

- 不要刷屏
- 不要在音频线程频繁日志
- 不要记录 PCM 数据
- 不要输出大量 debug 信息

------

# 二十一、配置

使用：

```text
serde
serde_json
```

例如：

```json
{
  "process": "DeepRockGalactic.exe",
  "output_device": "...",
  "volume": 1.0
}
```

配置尽量简单。

不要数据库。

不要 SQLite。

不要注册中心。

不要服务器。

------

# 二十二、第一阶段 MVP

不要一次实现全部功能。

第一阶段只实现：

```text
1. 枚举进程
2. 枚举音频设备
3. 选择进程
4. 选择输出设备
5. Process Loopback Capture
6. WASAPI Render
7. 音量控制
8. 开始/停止
9. 基本错误处理
```

最终验证：

```text
Minecraft.exe
       ↓
Process Loopback
       ↓
Rust Audio Engine
       ↓
VB-CABLE
       ↓
Discord
```

------

# 二十三、第二阶段

加入：

```text
进程退出检测
自动重连
设备拔插检测
配置保存
系统托盘
开机启动
```

------

# 二十四、第三阶段

再考虑：

```text
多进程混音
每个进程独立音量
多个输出设备
快捷键
音频路由
```

但不要提前实现。

------

# 二十五、性能目标

项目必须以低资源为重要指标。

重点关注：

```text
Idle CPU
Active CPU
Idle RAM
Active RAM
Startup Time
Executable Size
```

开发过程中定期测量。

不要只说：

> “Rust 很轻。”

必须实际测量。

------

# 二十六、Release 构建

研究并使用：

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

根据实际测试决定：

```text
opt-level = "z"
```

还是：

```text
opt-level = 3
```

不要为了追求 exe 小而明显牺牲运行性能。

最终需要实际对比：

```text
体积
启动速度
CPU
RAM
```

------

# 二十七、依赖控制

每增加一个 Cargo dependency，都回答：

```text
1. 为什么需要？
2. 能不能 Windows API 直接实现？
3. 会不会增加最终体积？
4. 会不会增加运行时内存？
5. 有没有更轻的替代方案？
```

优先：

```text
windows
slint
serde
serde_json
```

其他依赖必须有明确理由。

不要为了方便堆 crate。

------

# 二十八、不要做的事情

当前阶段禁止：

```text
❌ Electron
❌ Tauri
❌ Node.js
❌ WebView
❌ React
❌ Vue
❌ FFmpeg
❌ GStreamer
❌ 数据库
❌ 云端服务
❌ 登录系统
❌ 自动更新
❌ 插件系统
❌ 网络 API
❌ 自己开发虚拟声卡驱动
```

------

# 二十九、关于原项目的处理

先完整阅读：

```text
App.xaml
App.xaml.cs
MainWindow.xaml
MainWindow.xaml.cs
Speaker.cs
MessageQueue.cs
TTSServer.cs
ModInstaller.cs
README.md
DRGSoundPad.csproj
```

尤其关注：

```text
Speaker.cs
MainWindow.xaml.cs
MessageQueue.cs
TTSServer.cs
```

分析：

```text
音频从哪里来
怎么播放
怎么进入麦克风链路
有没有消息队列
线程如何通信
哪些功能属于 DRG 特化
哪些功能是通用 SoundPad 功能
```

把分析结果写入：

```text
docs/architecture-analysis.md
```

然后再开始 Rust 实现。

------

# 三十、DRG 特化与通用功能分离

原项目是：

```text
DRG SoundPad
```

Rust 新项目应该尽量设计成：

```text
通用 Audio Router / SoundPad Core
```

例如：

```text
core
├── process capture
├── audio routing
├── audio output
└── volume

drg
├── DRG specific integration
└── DRG specific features
```

不要把：

```text
Deep Rock Galactic
```

硬编码到 Audio Engine。

------

# 三十一、代码质量

要求：

- Rust 2024 Edition
- clippy 尽量无 warning
- cargo fmt
- 合理的错误类型
- 模块职责单一
- 不滥用 unsafe
- Windows API 的 unsafe 封装在少量模块内
- 对 unsafe 代码写清楚原因
- 音频实时线程避免 panic
- 不复制大型数据
- 不产生无意义 clone

特别注意：

> `unsafe` 不是禁止，而是集中管理。

例如：

```text
audio/windows.rs
```

负责 Windows API / COM / WASAPI 的 unsafe。

上层：

```text
AudioEngine
```

尽量保持安全 Rust API。

------

# 三十二、验证方式

每完成一个阶段，都必须实际运行。

不要只保证：

```text
cargo check
```

必须：

```text
cargo build --release
```

然后实际测试。

------

# 三十三、第一阶段验收标准

必须能够做到：

```text
启动程序
↓
看到当前进程
↓
选择一个正在播放声音的进程
↓
看到音频设备
↓
选择目标设备
↓
点击开始
↓
目标设备能够收到该进程声音
↓
调整音量有效
↓
停止后音频停止
```

然后测试：

```text
进程退出
设备拔出
进程重新启动
输出设备切换
无音频进程
高音量
长时间运行
```

------

# 三十四、开发工作方式

不要一次生成整个项目。

按照以下顺序执行：

## Step 1

分析 C# 原项目。

输出：

```text
docs/architecture-analysis.md
```

包含：

- 功能列表
- 类职责
- 音频链路
- 线程模型
- 数据流
- UI 功能
- DRG 特化逻辑
- 可以删除的逻辑
- Rust 重构建议

------

## Step 2

设计 Rust 架构。

输出：

```text
docs/rust-architecture.md
```

------

## Step 3

建立最小 Cargo 项目。

先实现：

```text
Process Enumeration
```

------

## Step 4

实现：

```text
Audio Device Enumeration
```

------

## Step 5

实现：

```text
WASAPI Process Loopback
```

先做到：

```text
Process → Capture → WAV
```

用于验证。

------

## Step 6

实现：

```text
Capture → RingBuffer → WASAPI Render
```

实现实时音频转发。

------

## Step 7

加入 Slint UI。

------

## Step 8

加入：

```text
Volume
Start / Stop
Status
Error Handling
```

------

## Step 9

性能优化。

重点测量：

```text
RAM
CPU
Startup
EXE Size
```

------

## Step 10

最后再处理：

```text
Tray
Auto Reconnect
Config
Hotkey
Multi Process
```

------

# 三十五、最重要的一句话

不要把这个项目做成：

> “一个 Rust 版 C# 项目”。

而应该做成：

> **“理解 C# 项目的功能以后，重新设计的轻量 Rust Windows 音频工具。”**

最终目标：

```text
小
快
稳
原生
低内存
低磁盘
```

优先级高于：

```text
功能堆砌
UI 炫技
跨平台
复杂架构
大量第三方库
```

现在开始工作。

第一步不要写代码。

先读取并分析原项目全部相关源码，确认现有功能、音频链路和依赖关系，然后输出迁移方案；确认方案后再开始 Rust 项目实现。