# ProcessMic 智能体约束

给在本仓库改代码的 agent 用。先读本文件，再动代码。权威需求：`goal.md`。架构事实：`docs/step-01-architecture-analysis.md`、`docs/step-02-rust-architecture.md`。
重构计划（分片、行数预算、验收、删除清单）：`docs/refactor-plan.md`。UI 边界与契约面：`FRONTEND.md`。故障定位：`DEBUGGING.md`。

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
2. 读 `docs/step-02-rust-architecture.md` 和本文件「已知陷阱」。
3. 读将改模块的**完整源文件**，不是只读函数签名。改导出符号前查调用点。
4. 复述：要改什么、不改什么、怎么验证。范围不清就停，先问。
5. 新功能：对照 `docs/step-01-architecture-analysis.md` 里「可删 / DRG 特化 / 通用」分类。DRG、命名管道、TTS、Mod 安装不要进 `engine` / `audio`。

不要一次生成整个项目。功能轮次：一轮只推一条用户可感知的行为，按 `goal.md` 第五节「迭代方式」走；
已完成的 Step 见 `docs/step-*.md`，不要重做。

重构轮次（`docs/refactor-plan.md` 的 S0–S6）：一轮只做一个切片，只动该切片点名的文件；
验收标准是**行为保真**（基线截图 / CLI 输出 / WAV 比对 / 门禁全绿），不要求用户可感知行为。
重构期每个提交都必须是「能编译 + 行为不变」，禁止留下「一半在旧文件、一半在新文件」的中间态。

---

## 2. 硬禁止

```text
Electron / Tauri / WebView / Node / Chromium / React / Vue
FFmpeg / GStreamer / cpal / rodio / rubato / 大型音频框架
自己写虚拟声卡驱动
数据库 / 云 / 登录 / 自动更新 / 插件 / 网络 API
空壳模块（为「架构好看」建、没有实现内容的目录或文件）
单文件堆多职责（超过 3.1.2 硬指标）
Slint 回调里做同步 IO / 枚举设备 / join 线程（见 3.5.1）
音频热路径 unwrap / expect / panic / tracing 刷屏 / 文件 / 网络 / UI / 锁
把核心音频状态塞进 Slint
把 Deep Rock Galactic 硬编码进 AudioEngine
```

每加一个 Cargo 依赖必须能回答：为什么需要、能否 Win32 直接做、体积、内存、有无更轻替代。  
默认只允许：`windows`、`windows-core`、`slint`、`tracing`、`tracing-subscriber`、`serde`、`serde_json`
（后两者见 `goal.md` 第三节，已授权）。`slint` 会把 `windows` 拉到 **0.62**，禁止拆版本。
不拆 workspace（单 crate + 目录模块即可）、不引异步运行时；取舍结论见 `docs/refactor-plan.md` 第七节。

---

## 3. 代码规范

### 3.1 语言与模块

- Edition 2024。`clippy` 尽量干净。改完对碰过的文件跑 `cargo fmt`（通过 `scripts\build-dev.cmd`）。
- 模块按职责，能合并就合并。现树：

```text
src/main.rs          CLI + UI 入口（组合根：DPI、tracing、argv 派发）
src/engine.rs        AudioEngine：源线程 + 多输出 render + fan-out（音频状态权威）
src/error.rs         统一错误；UI 只用 user_message()
src/process.rs       进程枚举
src/config.rs        serde 配置模型 + 持久化 + 单实例互斥
src/hotkey.rs        RegisterHotKey（仅 System 模式；GlobalHook 未实现）
src/ui_bridge.rs     Slint 回调、UI 缓存、50ms 状态轮询（当前 1976 行，按 3.1.2 拆）
src/vbcable.rs       VB-CABLE 安装
src/audio/
  com.rs buffer.rs format.rs device.rs policy.rs peak.rs wav.rs
  capture.rs render.rs sfx.rs preview.rs decode.rs
ui/
  main.slint         窗口 + 顶栏/页面/状态栏 + 右键菜单覆盖层
  theme.slint        视觉 token
  components/        可复用控件（button、card、menu-item…）
  pages/             页面（route、player、settings）
```

目标树与每文件行数预算见 `docs/refactor-plan.md` 第三节，按 S0–S6 分片落地；本树描述的是**当前**代码。

C# / WPF 源码已从工作树移除，原始文件见 git 历史，分析结论留在 `docs/step-01-architecture-analysis.md`。不要「翻译」回 Rust 模块。

#### 3.1.1 模块标准化

- **先搜再写**：动手前 `grep` 一遍有没有现成的函数、组件、常量；有就复用或就地扩展，**不要再造第二套**。
- 同一段东西出现两次就抽出来：
  - Slint 控件放 `ui/components/*.slint`，用 `export component`，页面只组合不复制（例：菜单项 `components/menu-item.slint`）。
  - 覆盖层（右键菜单、遮罩）在 `main.slint` 顶层**只声明一次**；页面只上报坐标与开关，不许每个页面各写一份。
  - Rust 共用流程抽成模块级 `fn`（例：播放启动统一走 `ui_bridge::start_playback`，ring 写入统一走 `push_latest` / `push_paced`）。
- 状态放权威对象：音频状态在 `AudioEngine`，UI 缓存状态在 `UiCache`。不要在别处新造平行状态或第二份列表。
- 目录就是分层：新页面进 `ui/pages/`，新控件进 `ui/components/`，音频相关进 `src/audio/`。不要造空壳模块。

#### 3.1.2 分层与文件规模（不许都挤在一个 rs 里）

- **按职责拆文件，不按方便堆**：一个 `.rs` / `.slint` 只干一件事。
- 硬指标（超限即拆，不是「建议」）：单文件 ≤ **400** 行（`.slint` ≤ **300**）、单函数 ≤ **100** 行、单闭包 ≤ **30** 行。

- 拆法：`foo.rs` + `foo/` 目录模块（`foo/mod.rs`、`foo/<职责>.rs`），别再往里塞。
- 每个切片结束时超限清单只能变短：数一遍行数，超限文件数增加视为没做完。
- 分层照 Spring Boot 那套「各司其职、上层调下层」：

```text
ui/           表现层：slint 页面与组件，只画界面、只上报事件
ui_bridge/    桥接层：Slint 回调、UI 缓存（UiCache）、事件 → 命令翻译
engine/audio  领域层：AudioEngine、捕获/渲染/混音/解码，对外安全 API
config/device 基础设施层：配置读写、设备枚举、COM、WAV
```

- 跨层只允许**上层调下层**；下层不许反向 `use` 上层的类型。同层可互相调用。
- 模块只暴露必要接口：能用 `pub(crate)` 就别 `pub`；调用方走函数签名/结构体接口，不许跨模块直接改别人的内部状态。
- 新文件必须能用一句话说清职责（写进模块顶 `//!`）；说不清就并回既有模块，不要新建。
- 拆分一次到位：拆完立刻 `cargo build`，不留「一半在旧文件、一半在新文件」的中间态。

#### 3.1.3 排版与可读性（硬要求）

- **一行只放一件事**：一个属性、一个绑定、一个 `///` 字段说明。
- 单行超过 **100 字符必须换行**；`.rs` 交给 `cargo fmt`，`.slint` 手工保证（改完自查超长行）。
- 长表达式拆行：三元、`&&` / `||` 条件、多参数调用，一行一个操作数，不要把 `if (a && b && c) { x = y; z = w; }` 挤成一行。
- 嵌套三元必须换行加括号，禁止一行套三层（例：状态点颜色）。
- 缩进 4 空格，层级靠缩进表达；不要用空格做视觉对齐，不要为了省行把两个组件写在同一行。

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

- `unsafe` 按**边界**收敛，不按文件名单（旧名单与语言现实冲突，已废止）：一个模块就是一个 unsafe 边界，
  该模块内部的 `unsafe` 不得外泄，对外只暴露安全 API。
- 允许出现 `unsafe` 的边界：SPSC ring（`UnsafeCell`）、WASAPI capture/render、COM/MF 互操作、
  手写 vtable（`IPolicyConfig`）、Win32 对话框与热键消息循环。其余位置出现 `unsafe` 视为设计错误。
- 每块 `unsafe` 必须写 `// SAFETY:`：别名、生命周期、谁释放、哪个线程的 apartment。
- 上层 `AudioEngine` 保持安全 API。
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

### 3.5.1 UI 线程（Slint 回调）约束

- 回调里禁止同步 IO、解码探测、设备/进程枚举、`join` 线程、加载大文件；超过 16ms 的工作丢给线程，
  结果用 `slint::invoke_from_event_loop` 回写（现有例：Cable 安装、播放启动）。
- 配置写盘必须 debounce（≥ 300ms）并在退出时 flush；禁止在滑块/拖动类回调里逐事件 `save`。
- 回调只做「读 UI 属性 → 调领域 API → 写回 UI 属性」，不放业务判断；
  索引映射、时长格式化这类纯逻辑抽成函数并配单测。
- Slint 字符串插值语法是 `\{表达式}`（反斜杠 + 花括号）。写成 `{index}` 会被当字面量原样输出成 `{index`。

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

### 3.8 依赖红线与单一权威

```text
ui/           不得持有音频引擎、不得持有 PCM（只允许 32 点峰值历史）
ui_bridge/    不得 use crate::audio::{decode,sfx,preview}；只走 engine 的安全 API
engine/audio  不得 use Slint 类型
process/infra 不得 use crate::audio::*
config/       不得 use Slint 类型、不得产出 UI 文案（键位 ↔ 文本映射属桥接层）
```

- 权威对象只有两个：`AudioEngine`（音频运行时）、`AppConfig`（持久化）。
- UI 缓存（`UiCache`）只允许存派生/视图态（下拉下标、筛选词、菜单坐标、播放锚点）；
  config 里已有的字段不得在 UI 侧再存一份。
- 依赖只向下：`ui → ui_bridge → engine → audio`；`config` / `platform` 是可被上层调用的基础设施。
  新增反向依赖（如 `process → audio`）视为错误。

### 3.9 测试与门禁

- 纯逻辑必须有单元测试（本仓库首次引入）：PCM 编解码、WAV/RIFF 解析、分类路径语义、
  时长格式化、键位映射、配置迁移。**不写**需要真实设备的测试。
- 每次提交前：`scripts\build-dev.cmd fmt -- --check` 与 `scripts\build-dev.cmd build`；
  碰过纯逻辑再跑 `cargo test`。
- 测试只断言可观察行为与边界；不为了让测试过而改断言口径，也不写只验证实现细节的测试。

---

## 4. 开始后（实现中）

- 先改现有文件，少造新文件。
- 一次一条用户可感知的行为，不要「趁机重构」（重构轮次按第 1 节的重构规则：一轮一个切片）。
- 音频改动：先保证 Initialize 成功，再谈混音/UI。
- 设备列表改动：用 `mintcat-proc-mic.exe --list-devices` / `--list-processes` 看真实名称，不要靠猜测。
- 用户贴了 HRESULT / 日志：对着代码改那条路径，不要开新抽象。

---

## 5. 结束后（交付前）

每阶段都必须**真跑**，`cargo check` 不算完成。

1. `scripts\build-dev.cmd build`（或 `--release`，按 `goal.md` 第七节）。
2. 覆盖刚改的路径：启动 UI，或 `--list-devices` / `--list-processes` / `--capture` / `--route`。
3. UI 改动必须有**渲染证据**：`--snapshot-ui --out <file>`（软件渲染器出真 RGBA 图，可加 `--menu`）
   或真实窗口操作；音频链路改动用 `--probe-playback` 逐采样比对。
4. 进程退出、设备失效、Initialize 失败：UI 短句 + tracing 细节，线程不崩。
5. 阶段结束写 `docs/step-XX-*.md`：做了什么、怎么跑的、实测结果（RAM/CPU/体积见 `goal.md` 第七节）。
   重构片额外记录基线对比证据与超限清单变化。不要空模板。
6. 同步被你改掉的事实：`docs/step-02-rust-architecture.md`（线程/格式/目录/依赖）、`FRONTEND.md`（UI 边界与契约面）、
   `DEBUGGING.md`（定位路径）。过期描述比没文档更糟。
7. 排版与规模自查：`.slint` 无超 100 字符行、一行一件事；碰过的 `.rs` 已 `cargo fmt`；
   超限文件（3.1.2 硬指标）只减不增。
8. 不要提交用户没要的 md、测试垫片、脚手架；调试产物（`*.raw`、`*.wav` 比对文件、截图）不进仓库。

每轮验收（`goal.md` 第六节）：选进程 → 选设备 → 开始 → 目标设备有声 → 音量有效 → 停止即停。

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
| 索引列显示 `{index` | Slint 插值语法是 `\{...}`，写成 `{index}` 被当字面量原样输出 | UI 文案里的动态值一律写 `\{表达式}` |
| 单文件长到近 2000 行 | 「顺手加个回调」在同一个 990 行函数里累积 | 见 3.1.2 硬指标：超限即拆，回调按域分组 |
| 大重构后回归定位不到原因 | 改动前工作树未提交，没有可回退基线 | 重构前先把当前状态提交成基线，每片一个提交 |
| 配置/UI 上能设但功能不生效 | 只写了配置字段或控件、没注册实现（设置页「播放时按键」、音频条目热键；`Hotkeys.mode` 连控件都没有） | 要么接线要么从 UI 撤掉，不留假开关 |

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

重构片（S0–S6）在 `实测` 一节额外写：基线对比证据（截图 / CLI 输出 / WAV 比对）与超限文件数变化。

---

## 8. 优先级

```text
小、快、稳、原生、低内存、低磁盘
>
功能堆砌、UI 炫技、跨平台、复杂架构、多 crate
```
