# DEBUGGING.md —— 本仓故障定位手册

先分类再动手，别凭感觉改代码：

```text
设备/初始化问题 → --list-devices / --route 看端点与初始化
播放/进度问题   → --probe-playback 做逐采样比对（解码→SFX/试听时钟）
UI 渲染问题     → --snapshot-ui --out <file> [--menu] 出真 RGBA 图
```

---

## 1. 通用顺序

1. `scripts\build-dev.cmd build` 拿编译反馈（Slint 语法错误只在编译期暴露）。
2. 用 CLI 把问题从 UI 里剥出来：`--list-processes` / `--list-devices` /
   `--capture <pid> [--wav out.wav] [--seconds N]` / `--route <pid> --device <id> [--seconds N]`。
3. 在能复现的最小路径上改；改完按 `AGENTS.md` 第 5 节真跑一遍（截图 / CLI 输出 / WAV 比对）。

---

## 2. 音频链路

| 现象 | 先查 | 再查 |
|---|---|---|
| 目标设备没声 | `--list-devices` 里端点是否存在、是否被当成虚拟设备；`--route` 能否出声 | 输出 ring 与 render 线程是否起来；设备格式是否被 Initialize 接受 |
| `0x80070057` 参数错误 | `WAVEFORMATEXTENSIBLE` 是否被 `*src` 成 `WAVEFORMATEX`（`cbSize=22` 尾巴丢失） | `sanitize_wave` 是否把格式做成 `cbSize=0` 的 PCM / IEEE float |
| `0x88890021` 无法初始化 | 进程环回 Initialize 是否带了 `AUDCLNT_STREAMFLAGS_LOOPBACK` | `AUTOCONVERTPCM` / `SRC_DEFAULT_QUALITY` 是否在 flags 里 |
| 播放重复某几帧 | `src/audio/decode.rs` 是否把超过当前块大小的 MF sample 尾部丢掉 | `src/audio/sfx.rs`、`src/audio/preview.rs` 是否按已消费的完整帧清理缓冲 |
| 切歌/停止后进度不归零 | 停旧源三步（取消标志、`StopAll`、停试听源）是否在**成功与失败两条路径**都执行 | UI 是否同时写了 `audio-progress`、`audio-duration-seconds`、`audio-current-position` |
| 试听无声 | 引擎的试听输出是否被打开、设备是否可用 | 打开失败时是否只提示不崩（UI 短句 + tracing 细节） |
| 进程退出/设备拔出后无声 | 状态是否变为 `ProcessExited` / `DeviceGone` | 音频线程是否仍存活、是否被正确 join |

---

## 3. 音频库与 UI

- 分类异常：先核对「显示索引 → `category_paths()` 真实路径」的映射，再核对配置里的路径；
  **不要**把带缩进的显示文本写回配置。
- 索引/时长对不上：核对列表筛选顺序（当前分类含全部后代 + 搜索）与 `duration_hns` 是否已补齐。
- 菜单不消失或位置不对：坐标由页面按下时上报，`main.slint` 顶层只做钳制；
  菜单项增删必须同步高度常量（现为 132px / 292px，S3 改为组件自算）。
- 渲染核对：`--snapshot-ui --out <file>`（可加 `--menu` 复现右键菜单态），
  与改动前的基线图逐像素对比，比肉眼看窗口可靠。

---

## 4. 已知验证限制

- 原生 Slint 窗口不能用浏览器工具驱动；渲染可自动化（`--snapshot-ui`），**交互仍需 Windows 桌面手工操作**。
- 需要真实声卡/虚拟线缆的判断（听感、Discord 侧是否听到）无法自动化，必须人工确认。
- 工具与游戏权限不一致时热键收不到：与游戏同权限运行（都提权或都不提权）。
- 重构会把 `src/ui_bridge.rs` 拆成 `src/ui_bridge/` 目录：定位时按职责找文件
  （`callbacks/`、`lists.rs`、`persist.rs`、`playback`），不要找一个 2000 行的文件。
