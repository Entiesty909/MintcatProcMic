# Step 1 — C# 原项目架构分析

分析对象：`D:\code\MintcatProcMic`（DRGSoundPad，.NET 6 WPF）。  
分析范围：`App.xaml`、`App.xaml.cs`、`MainWindow.xaml`、`MainWindow.xaml.cs`、`Speaker.cs`、`MessageQueue.cs`、`TTSServer.cs`、`ModInstaller.cs`、`README.md`、`DRGSoundPad.csproj`。  
结论：原项目不是进程音频路由器，而是 **DRG 游戏内聊天 TTS SoundPad**。Rust 版本按 `goal.md` 重新设计，不机械移植。

---

## 1. 项目定位

| 项 | 事实 |
|---|---|
| 产品名 | DRG SoundPad |
| UI | WPF，窗口标题 `DRG SoundPad`，600×450 |
| 运行时 | `net6.0-windows`，`UseWPF=true` |
| 依赖 | NAudio 2.2.1，Newtonsoft.Json 13.0.3 |
| 发布 | `dotnet publish -r win-x64` 单文件 + Trim |
| 许可证 | MIT（Copyright 2024 Iris） |

原项目解决的问题：

> Deep Rock Galactic 游戏内聊天文本 → 游戏 Mod 经命名管道送到本进程 → TTS 合成 WAV → 同时播到 VB-CABLE 和默认扬声器，使 Discord 等软件能从虚拟麦克风听到语音。

它 **没有**：

- 进程枚举
- WASAPI
- Process Loopback
- 把某个进程的输出转发到另一设备
- 音量滑条 / 开始-停止路由

`goal.md` 要做的是另一类产品：**通用 Process Audio Router**。原项目只提供交互习惯、VB-CABLE 目标和“把声音送进麦克风链路”的产品意图。

---

## 2. 功能列表

### 2.1 实际存在的功能

1. **启动检查 VB-CABLE**  
   读注册表 `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*`，`DisplayName` 含 `"CABLE"` 即视为已安装。
2. **一键启动 VB-CABLE 安装包**  
   运行 `{BaseDirectory}\VBCABLE\Setup.exe`，`Verb=runas` 提权。安装包不在仓库内。
3. **检查 / 安装 DRG UE4SS Mod**  
   在所有盘符扫描 `SteamLibrary\steamapps\common\Deep Rock Galactic\FSD\Binaries\Win64\`，把 `.\UE4SS` 目录拷进去。用 `main.dll` MD5 判断是否已安装。
4. **枚举输出设备**  
   NAudio `WaveOut.DeviceCount` / `WaveOut.GetCapabilities`（MME，产品名截断到 32 字符）。填入 ComboBox。
5. **命名管道收游戏消息**  
   管道名 `drg_named_pipe`，JSON `{ player, msg }`。
6. **TTS 下载并缓存 WAV**  
   默认 URL：`https://dds.dui.ai/runtime/v1/synthesize?...&text=`。文件名 `MD5(msg).wav`，目录 `Sound/`。
7. **双设备播放**  
   - 固定设备：名称包含 `CABLE Input (VB-Audio Virtual C` 的 WaveOut 设备  
   - 同时播放到 `DeviceNumber = 0`（系统默认 WaveOut）  
   音量硬编码 `1.0`。
8. **日志框**  
   追加 `[player]msg`。
9. **TTS URL 可改**  
   文本框改 `TTSServer.DefaultUrl`，不落盘。

### 2.2 UI 有、逻辑未接上的功能

- **输出设备 ComboBox**：填充设备名，**从未读取 `SelectedItem`**。播放目标不跟 ComboBox 走。
- `_vbDevice` 只在构造函数算一次。VB-CABLE 后装则一直是 `-1`。
- `PlayAudioToSpecificDevice(..., stopCurrent, ...)` 的 `stopCurrent==true` 分支会停掉双设备，但调用处永远传 `false`。
- `PlayAudioToSpecificDevice` 在 `currentOutputDevice != null` 时直接忽略新请求（不排队、不打断）。重叠聊天会丢。
- `MessageQueue.Read()` 用 `using StreamReader` 包住 pipe，读一行后 **关闭并拆掉管道**。每条消息都要重连。
- `GetGamePath()` 跳过了 `C:\` 的 TODO，只拼 `X:\SteamLibrary\...`，默认 Steam 库在 `C:\Program Files (x86)\Steam\...` 时找不到。

### 2.3 仓库里没有、运行期依赖的资产

- `VBCABLE\Setup.exe`
- `UE4SS\Mods\DRGSoundPad\dlls\main.dll` 及 UE4SS 树
- 第三方 TTS HTTP 服务可用性

---

## 3. 类职责

| 类型 | 文件 | 职责 | 问题 |
|---|---|---|---|
| `App` | App.xaml.cs | 空壳，`StartupUri=MainWindow.xaml` | 无 |
| `MainWindow` | MainWindow.xaml(.cs) | 组装检查、管道线程、TTS 播放、UI | UI 线程 `async void PlaySound`；管道线程 `Dispatcher.Invoke` 写日志 |
| `SpeakServer` | Speaker.cs | VB 检测、设备枚举、NAudio 播放 | 静态可变设备指针；非线程安全；MME 不是 WASAPI |
| `MessageQueue` | MessageQueue.cs | 命名管道服务端循环 | 读完即拆 pipe；`callback` 未做 null 防护 |
| `DRGMessage` | MessageQueue.cs | `player` + `msg` | 公共字段，无校验 |
| `TTSServer` | TTSServer.cs | HTTP GET 下载 WAV | 每次 `new HttpClient`；错误只 `Console.Write` |
| `ModInstaller` | ModInstaller.cs | 找 DRG 目录、拷 UE4SS | 盘符扫描脆弱 |

没有独立的音频引擎、设备热插拔、进程监视、配置层。

---

## 4. 音频链路（原项目）

```text
DRG.exe
  └─ UE4SS Mod (仓库外)
       └─ Named Pipe "drg_named_pipe"  JSON {player,msg}
            └─ MessageQueue.MainLoop  (独立 Thread)
                 └─ callback = MainWindow.PlaySound
                      ├─ MD5(msg) 缓存命中? 否则 HTTP TTS → Sound/*.wav
                      └─ lock
                           ├─ WaveOutEvent → VB-CABLE Input   (进 Discord 麦克风)
                           └─ WaveOutEvent → WaveOut device 0 (本地监听)
```

要点：

- 音频 **来自 TTS 文件**，不是游戏进程渲染的 PCM。
- 进入麦克风链路的方式是 **播放到 VB-CABLE 的输入端**，由用户在 Discord 里选 CABLE Output 当麦克风。没有虚拟声卡驱动，也没有进程环回。
- NAudio `WaveOutEvent` = Windows MME `waveOut*`，不是 WASAPI，延迟和设备名都差一截。
- 内部没有 ring buffer、没有格式协商、没有采样率转换。NAudio 在 `AudioFileReader` 里做文件解码。

---

## 5. 线程模型（原项目）

```text
WPF UI 线程
  ├─ Window_Loaded: 启动管道线程
  └─ Dispatcher.Invoke: 写日志

Thread (MessageQueue.MainLoop)
  └─ WaitForConnection 阻塞
       └─ PlaySound（在管道线程上跑）
            ├─ await HTTP（async void，同步上下文是线程池）
            └─ lock + WaveOutEvent.Play()  （NAudio 自己再开播放线程）
```

- UI 与播放未分离设计，只是 NAudio 内部碰巧有后台线程。
- `lock (_lockObject)` 包住整个 `Play` 调用，但 `Play` 立即返回；真正的重叠发生在 NAudio 回调里，锁保护不到。
- 管道线程从不退出，窗口关闭后仍阻塞在 `WaitForConnection`。
- 没有音频实时线程约束（分配、锁、IO 都在播放路径上：读文件、HTTP、MD5）。

---

## 6. 数据流

```text
游戏文本
  → JSON 一行
  → DRGMessage
  → 字符串 MD5
  → 本地 WAV 路径
  → AudioFileReader (解码任意 NAudio 支持格式，目标仍是文件)
  → PCM 到 MME 设备
```

配置：无文件、无 JSON、无注册表写入。TTS URL 只活在内存。

持久化仅有：`Sound/<md5>.wav` 缓存（只增不删）。

---

## 7. UI 功能对照

原 UI：

```text
虚拟声卡  [已启用/未启用]  [安装]
DRG Mod   [已启用/未启用]  [安装]
输出设备  [ComboBox]          ← 未接线
TTS API   [URL TextBox]
日志      [多行 TextBox]
```

`goal.md` 目标 UI：

```text
音频来源  [进程 ComboBox]
输出设备  [设备 ComboBox]
音量      [Slider]
状态      [未运行 / 运行中 / 错误]
          [开始转发]
```

可保留的交互：单窗、下拉选目标、状态文字、少按钮。  
不可保留：Mod 安装、VB 安装器、TTS URL、聊天日志。

---

## 8. DRG 特化 vs 通用

| 逻辑 | 分类 | Rust MVP |
|---|---|---|
| 命名管道 `drg_named_pipe` | DRG | 删除 |
| UE4SS / `main.dll` MD5 / 拷贝安装 | DRG | 删除 |
| 扫描 SteamLibrary DRG 路径 | DRG | 删除 |
| DUI TTS HTTP | DRG | 删除 |
| `Sound/` WAV 缓存 + MD5 | DRG | 删除（仅开发期 capture→WAV 验证） |
| 聊天日志 `[player]msg` | DRG | 删除 |
| VB-CABLE 注册表检测 + Setup.exe | 环境辅助 | MVP 不做安装器；设备列表里若有 CABLE 可选 |
| 播到虚拟线缆让 Discord 听到 | 通用意图 | **保留为产品目标**，用 WASAPI render 到用户选的设备 |
| 输出设备列表 | 通用 | 保留，改 WASAPI 枚举 |
| 同时听本地 + 进虚拟麦 | 原行为 | MVP 只渲染到 **一个** 用户选择的设备（简化；第二输出留到第三阶段） |

Audio Engine 不得出现 `DeepRockGalactic` 字符串。

---

## 9. 可以删除、不要带进 Rust 的东西

- WPF / `async void` / `Dispatcher`
- NAudio / MME WaveOut
- Newtonsoft.Json
- HttpClient TTS
- Named Pipe 协议
- UE4SS 安装器与盘符扫描
- 注册表扫 Uninstall 猜 VB
- 静态 `currentOutputDevice` 双实例
- `ComputeMD5Hash` 文本哈希
- 硬编码设备名截断 `"CABLE Input (VB-Audio Virtual C"`
- 每次消息重建管道
- `Console.WriteLine` 当错误处理
- .NET 单文件发布模型（改 cargo release）

---

## 10. 原设计为什么那样，Rust 该怎么做

| 原设计动机 | 本质需求 | Rust 做法 |
|---|---|---|
| 游戏听不到外部程序，用 Mod+管道出文本 | 需要“游戏相关的声音”进麦 | 不再做 TTS；捕获 **指定进程的音频输出** |
| 播到 VB-CABLE | 让 Discord 选虚拟麦 | WASAPI 渲染到用户选的 render 设备（CABLE Input / Voicemeeter / 耳机） |
| 同时 WaveOut 0 | 用户自己也要听见 | MVP 单出口；用户把出口选成耳机或后面再加监听 |
| NAudio 图省事 | 播 WAV 文件 | 实时 PCM，无文件热路径 |
| 后台 Thread + lock | 别卡 UI | UI 线程 / capture 线程 / render 线程 + SPSC ring |
| 静态设备对象 | 简单 | `AudioEngine` 拥有生命周期，Start/Stop 明确 |

---

## 11. 边界情况（原项目已暴露，Rust 必须处理）

| 原项目表现 | Rust 要求 |
|---|---|
| VB 未装，`_vbDevice=-1`，播放 catch 后静默 | 设备缺失 → UI 错误，不崩 |
| 进程/游戏退出，管道只是等下一次连接 | 捕获会话结束 → 状态“进程已退出”，音频线程不 panic |
| TTS HTTP 失败，文件可能空/不存在，播放 catch | 捕获/渲染初始化失败 → 可读错误 |
| ComboBox 设备被拔掉仍显示旧名 | 热插拔检测（第二阶段）；MVP 至少 Start 时校验 |
| 窗口关了管道线程还在 | Drop/退出时 stop 引擎、join 线程 |
| 设备名 MME 截断导致匹配失败 | WASAPI 用设备 ID + 完整 FriendlyName |
| `PlayAudioToSpecificDevice` 忙时丢包 | 实时路由不走文件队列；ring 满丢最旧，保活延迟 |

---

## 12. Rust 重构建议（给 Step 2 的输入）

1. **产品改名定位**：通用 Windows Process Audio Router，不是 DRG SoundPad 克隆。
2. **音频核心**：WASAPI Process Loopback（`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`）→ SPSC ring → WASAPI Shared Render。
3. **UI**：Slint，四个控件：进程、设备、音量、开始/停止 + 状态。
4. **DRG 特化**：独立模块，MVP **不实现**。
5. **依赖白名单**：`windows`、`slint`、`tracing`（开发日志）。配置用标准库 + 少量 `serde`/`serde_json`（第二阶段才真正读写文件）。
6. **验证阶梯**：枚举进程 → 枚举设备 → 捕获写 WAV → 实时转发 → UI → 音量/启停/错误 → 测体积与内存。

Step 1 完成。下一步：`docs/rust-architecture.md`。
