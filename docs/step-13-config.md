# Step 13 — 配置模型与单实例
范围：完成 serde/serde_json 配置模型、旧热键配置迁移、单实例互斥、设备/进程/混音状态保存，以及全局播放时按键动作的数据模型。

改动文件：
- `Cargo.toml`：加入 `serde`（derive）与 `serde_json`。
- `src/config.rs`：
  - `AppConfig` v2 根配置。
  - 输出、路由、三路混音、声板 pad、播放器、热键集合模型。
  - 旧 `{mods,vk}` 配置迁移为 `hotkeys.toggle_route`。
  - `Local\\ProcessMic.SingleInstance` 命名互斥体。
  - 新增全局 `PlaybackKeyAction`：`None` / `Press` / `Hold`。这是所有音频播放共用的游戏 PTT 动作，不属于固定 pad。
- `src/ui_bridge.rs`：启动时读取配置，退出时保存；保存当前进程名、输出 ID/名称、麦克风、设备显示开关、音量与默认麦设置。

全局播放按键约定：
- `None`：播放不模拟键盘输入。
- `Press`：播放开始时按一次并释放。
- `Hold`：播放开始时按下，播放结束、停止或切换时释放。
- 实际 `SendInput` 注入在 S6/S4 的播放状态机接入；当前阶段只完成持久化模型，默认关闭。
- 设置页最终必须显示风险提示：这是系统级输入注入，部分游戏反作弊可能拒绝；游戏与工具权限不一致时可能失效。

怎么跑：
```bat
scripts\\build-dev.cmd build
target\\debug\\mintcat-proc-mic.exe --list-processes
target\\debug\\mintcat-proc-mic.exe --list-devices
target\\debug\\mintcat-proc-mic.exe --route <pid> --device <wasapi-id> --seconds 1
```

实测：
- `scripts\\build-dev.cmd build`：成功。
- `--list-processes`：成功。
- `--list-devices`：成功。
- 动态路由 S2 回归仍通过：Process Loopback 初始化成功，Start/Stop 正常。
- 配置解析使用 serde_json；没有配置文件时回退默认，不阻塞 UI/音频。

下一步：
- Step 14：接入 WAV 解码、sfx 混音线程、pad 网格与单次/循环/按住/切换播放模式。
