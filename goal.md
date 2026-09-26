# ProcessMic 目标（持续迭代）

产品：Windows 专用、极轻的 **Process Audio Router**（ProcessMic），原 DRG SoundPad 的 Rust 重写。

C# / WPF 原项目源码已从工作树移除（见 git 历史），功能与边界分析结论在 `docs/step-01-architecture-analysis.md`，
Rust 架构事实在 `docs/step-02-rust-architecture.md`。本文件是**权威需求**；与本文件冲突的旧文档以本文件为准。

**移植与技术选型阶段已结束**（Step 1–32，逐阶段记录见 `docs/step-*.md`）。
现在进入持续迭代：一轮一个用户可感知的改动，做完就真跑、写文档、提交。

---

## 一、产品定位

一个**轻量**的进程音频路由器：

```text
指定进程 → WASAPI Process Loopback → 混音 → 输出到虚拟设备（可多路）→ 游戏/语音软件
```

同时带一个**音频库**（本地音频文件双击即播，发给虚拟麦或本机试听），用于游戏内放音。

不是「Rust 版 C# 项目」，也不是通用 DAW。C# 原项目只作功能与边界参考，禁止 class→struct 机械翻译。

---

## 二、技术栈与硬禁止

栈固定：

```text
Rust 2024 + Slint + windows-rs 0.62 + WASAPI
```

禁止（不是「不推荐」，是禁止）：

```text
Electron / Tauri / WebView / Node / Chromium / React / Vue
FFmpeg / GStreamer / cpal / rodio / rubato / 大型音频框架
自己写虚拟声卡驱动
数据库 / 云 / 登录 / 自动更新 / 插件 / 网络 API
空壳模块（为了「架构好看」建的目录）
音频热路径 unwrap / expect / panic / tracing 刷屏 / 文件 / 网络 / UI / 锁
把核心音频状态塞进 Slint
把 Deep Rock Galactic 硬编码进 AudioEngine
```

音频一律走 Windows 原生 API：

```text
Win32 / COM / Windows Audio / WASAPI / Process API
```

进程声源必须用 **WASAPI Process Loopback**：

```text
ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, … INCLUDE_TARGET_PROCESS_TREE)
```

不要用普通 endpoint loopback 去「猜」进程。

---

## 三、依赖控制

默认只允许：`windows`、`windows-core`、`slint`、`tracing`、`tracing-subscriber`、`serde`、`serde_json`。

每加一个依赖必须能回答：为什么需要、能否 Win32 直接做、体积、内存、有无更轻替代。

`slint` 会把 `windows` 拉到 **0.62**，禁止拆版本。

---

## 四、现状（已完成，不要重做）

```text
engine        常驻输出、多进程输入、可选物理麦、SFX 混音、多输出 fan-out（含格式转换）
process       进程枚举（PID / 名称 / 窗口标题）
device        输出与录音设备枚举、默认设备角色、VB-CABLE 安装与检测
audio         buffer(SPSC ring) / capture / render / format / peak / policy / wav
              decode(Media Foundation：mp3/mp4/m4a/flac/wma，WAV 自解析) / preview / sfx
config        serde JSON 配置、单实例互斥、热键模型
hotkey        RegisterHotKey（多模式）
ui            三页：路由（进程 + 混音 + 电平）、音频库、设置；Windows 11 风格右键菜单
audio library 分类树（多层、重命名、删除）、条目时长、双击播放/再双击停止、
              进度条拖动跳转、本地试听、流式解码长文件
```

细节与实参：`docs/step-02-rust-architecture.md`；每阶段做了什么见 `docs/step-*.md`。

---

## 五、迭代方式（每轮怎么做）

1. **一轮一个用户可感知的行为**，只做被点名的需求，不顺手改交互/文案/默认值。
2. 动手前读该模块**完整源文件**与 `docs/step-02-rust-architecture.md` 相关段落；改导出符号前查调用点。
3. 按 `AGENTS.md` 第 3 节写代码：分层（表现/桥接/领域/基础设施）、复用现有函数与组件、排版一行一件事。
4. **真跑**：`scripts\build-dev.cmd build`，再覆盖刚改的路径（UI、`--list-devices`、`--capture`、`--route`）。
5. 写 `docs/step-XX-*.md`（编号递增，**不重写旧文档**），同步 `docs/step-02-rust-architecture.md` 里被改掉的事实。
6. 不重做 `docs/step-*.md` 里已完成的步骤；不提交用户没要的 md、测试垫片、脚手架。

---

## 六、每轮验收

音频链路必须真的通：

```text
选进程 → 选输出设备 → 开始转发 → 目标设备有声 → 音量有效 → 停止即停
```

同时覆盖异常路径：

```text
进程退出 / 设备拔出 / Initialize 失败 / 进程重启 / 输出设备切换 / 无音频进程 / 长时间运行
```

要求：UI 只给短句（`Error::user_message()`），细节进 `tracing`，线程不崩。

UI 改动要看真实渲染结果（起 UI 或截图核对），`cargo check` 不算完成。

---

## 七、性能与资源

持续关注：Idle/Active CPU、RAM、启动时间、exe 体积。

Release 构建（`Cargo.toml` 已定）：

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

不为了追求 exe 小而明显牺牲运行性能。需要取舍时以实测数据说话，不要只说「Rust 很轻」。

---

## 八、优先级

```text
小、快、稳、原生、低内存、低磁盘
>
功能堆砌、UI 炫技、跨平台、复杂架构、多 crate
```
