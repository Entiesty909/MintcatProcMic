# Step 11 — UI shell
范围：完成 UI 第一阶段骨架与路由页重排；未实现声板、播放器、自动重连与多模式热键。

改动文件：
- `ui/main.slint`：顶栏四页签、常驻状态栏、主音量、DPI 适配后的窗口尺寸。
- `ui/theme.slint`：颜色、间距、圆角、字号 token。
- `ui/components/*.slint`：卡片、按钮、页签、声纹、滑条、热键字段、空态。
- `ui/pages/route.slint`：进程路由页；三张父卡片按比例响应式伸缩，进程下拉框跟随卡片宽度，长标题在控件内截断；输出与麦克风选择入口移到设置页。
- `ui/pages/soundpad.slint`：声板空态页，后续阶段接入真实 pad。
- `ui/pages/player.slint`：播放器空态页，后续阶段接入文件解码。
- `ui/pages/settings.slint`：设备、麦克风、VB-CABLE、热键入口、显示全部播放设备开关。
- `src/ui_bridge.rs`：分页状态、进程筛选、设备列表开关、状态栏、主音量、DPI 工作区尺寸收窄。
- `src/audio/device.rs`：删除未使用的虚拟路由拼接 API；`list_destinations(bool)` 支持显示全部播放设备。
- `src/engine.rs` / `src/audio/render.rs`：增加总音量原子值并在渲染末端应用。
- `src/main.rs` / `Cargo.toml`：启用 Per-Monitor V2 DPI 感知。
- `src/hotkey.rs`：删除两个未使用导入。
- `docs/rust-architecture.md`：同步 UI 文件结构、轮询周期、三路音量事实。

怎么跑：
```bat
scripts\build-dev.cmd build
target\debug\mintcat-proc-mic.exe --list-processes
target\debug\mintcat-proc-mic.exe --list-devices
target\debug\mintcat-proc-mic.exe
```

预期：
- CLI 能列出真实进程与设备。
- UI 能启动，显示「路由 / 声板 / 播放器 / 设置」四页。
- 路由页包含进程筛选、响应式等宽路由卡片、麦克风混入卡片、输出卡片。
- 状态栏包含状态、输出名称、总音量、开始/停止。

实测：
- 日期：2026-09-23
- `scripts\build-dev.cmd build`：成功。
- `--list-processes`：成功，列出当前 `javaw.exe`、`msedge.exe`、`steam.exe` 等进程。
- `--list-devices`：成功，列出 Realtek、VB-CABLE、Steam Streaming 等播放/虚拟设备；未生成 `扬声器 → 麦` 拼接文案。
- UI 启动：成功，进程保持运行，任务管理器工作集约 39 MB（debug 构建）。
- DPI：当前窗口检测到 144 DPI，已启用 Per-Monitor V2；窗口尺寸按工作区收窄，避免底部状态栏落出屏幕。
- 构建保留 3 个既有未使用方法警告：`SpscRing::len`、`MixFormat::as_ref`、`Converter::passthrough`；后续音频阶段继续使用或删除。

下一步：
- Step 12：把输出流改为常驻，引入动态进程/麦源与 `ring_sfx`，为声板和播放器提供稳定混音入口。
