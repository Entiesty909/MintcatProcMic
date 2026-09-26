# FRONTEND.md —— UI 边界、契约面与音频库状态

本文件描述**当前代码**的 UI 边界。改 UI 前先读它，改完必须同步它（`AGENTS.md` 第 5 节第 6 条）。
重构后的目标结构见 `docs/refactor-plan.md` 第三节。

---

## 1. 边界

| 文件 | 职责 | 明确不管 |
|---|---|---|
| `ui/main.slint`（508 行） | 窗口、顶栏页签、页面容器、常驻状态栏，**以及整个音频库右键菜单覆盖层**（378–507） | 页面内部布局 |
| `ui/pages/route.slint`（178） | 进程/麦/输出的选择与音量、声纹、事件上报 | 音频库 |
| `ui/pages/player.slint`（253） | 分类树、音频列表、时长列、播放条；只保存菜单的**坐标与开关** | 菜单本体（在 `main.slint` 顶层） |
| `ui/pages/settings.slint`（234） | 设备多选、混音、热键、试听、虚拟线缆 | 日常音频选择 |
| `ui/components/*.slint` | 可复用控件；`empty-state.slint` 目前无引用 | — |
| `ui/theme.slint`（30） | 颜色与长度 token；**尚无字号/字重/间距 token** | — |

Rust 侧现状：

- `src/ui_bridge.rs`（1976 行）承担桥接层全部职责：38 个 Slint 回调（`wire_callbacks` 364–1350）、
  `UiCache`、50ms 状态轮询（`start_status_timer` 1371–1505）、进程/设备刷新、音频库 CRUD、
  配置持久化、VB-Cable 安装、调试截图入口。
- 已登记的反例（重构收掉）：桥接层直接 `use crate::audio::{decode,sfx,preview,device,policy}`
  与 `vbcable`，并自己持有一个 `PreviewEngine`（等于第二个音频引擎）。
  目标：UI 不持有音频引擎、不直接解码（`AGENTS.md` 3.8）。

---

## 2. PCM 与电平

- UI **不持有 PCM**：只持有三路各 32 点的峰值历史
  （`capture-wave` / `mic-wave` / `render-wave`），每 50ms 用 `sample_*_peak()` 衰减采样后写入。
- 解码、混音、重采样、音量全部在领域层；UI 只消费标量（时长、播放位置、状态、错误短句）。

---

## 3. 契约面

- `MainWindow` 上的 **61 个 `in-out` 属性 + 40 个 callback** 是 Rust 与 UI 的唯一边界。
- 改契约必须同一次改完 `ui/main.slint` 与 `src/ui_bridge.rs`；删属性/回调时，
  连同 `wire_callbacks` 里的注册一起删，不留悬空转发。
- 已知死面（重构 S3 收口，先别当例子抄）：

```text
属性：mic-volume、mic-wave、mic-name、has-mic、category-edit-name、
      duration-seconds、entry-edit-name、device-index
回调：open-settings、entry-edit-tags、entry-save-tags、mic-volume-edited
```

---

## 4. 音频库状态语义

- 分类配置用 `/` 路径表示父子关系；`全部` 与 `未分类` 是固定根项。
- `audio_categories()` 把路径展开成带缩进的**显示模型**，回调按同一顺序映射回真实路径；
  **不要把带缩进的显示文本写回配置**。
- 当前分类包含其所有后代分类；搜索在筛选后的条目上执行；列表索引从当前可见结果从 1 开始。
- 时长来自 `AudioEntry.duration_hns`（100ns 单位；0 显示 `—`），由 WAV 头或 MF 元数据补齐。
- 单双击语义：最近一次点击在阈值内视为双击（播放/停止），否则视为选中；搜索词变化会重置可见列表。

---

## 5. 已知缺陷（重构修，不要当特性保留）

- 音频库条目列表**没有滚动容器**（`ui/pages/player.slint:143-195`），条目一多就看不见；
  S3 加 `ScrollView`，并考虑 `ListView` 虚拟化。
- 全仓无虚拟化：分类树、条目列表、设备列表都是裸 `for`，一次实例化全部行。
- 三处「能设置、不生效」：设置页的播放时按键、音频条目热键、`Hotkeys.mode`；
  全仓真正注册的热键只有 `toggle_route`（`src/hotkey.rs`）。
- 菜单尺寸硬编码（132px / 292px）与坐标钳制写在 `main.slint`，菜单项增删要同步改常量；S3 改为组件自算。

---

## 6. 验证

```text
scripts\build-dev.cmd build
target\debug\mintcat-proc-mic.exe --snapshot-ui --out snap-ui.raw [--menu]
target\debug\mintcat-proc-mic.exe --list-devices
```

`--snapshot-ui` 用软件渲染器把真实窗口渲染成 RGBA（文件头 8 字节 = 宽/高 小端 u32），
可核对布局、索引列、菜单——这是本仓唯一能自动化的 UI 证据。

交互仍需 Windows 桌面手工验收：右键建多层子树、重命名/删除父树、导入音频看索引与时长、
双击播放与再双击停止、拖动进度跳转、切换音频后进度归零、切页签后状态栏一致。
