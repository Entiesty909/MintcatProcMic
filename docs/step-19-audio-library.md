# Step 19 — 音频库真实接入
范围：把本地音频选择、分类树、右侧音频列表、双击播放和 SFX sender 接上；不再保留独立声板页面。

改动文件：
- `ui/pages/player.slint`：真实音频库布局：顶部添加/搜索，左分类树，右音频列表，底部迷你播放条。
- `ui/main.slint`：接入音频库状态与回调。
- `src/config.rs`：新增 `AudioEntry`（name/path/category/loop_playback）并持久化。
- `src/ui_bridge.rs`：
  - 原生 Windows 文件选择器 `GetOpenFileNameW`。
  - 添加本地音频到「未分类」。
  - 分类筛选与搜索。
  - 双击后通过 `AudioEngine::sfx_sender()` 解码并发送 SFX 播放命令。
  - 停止按钮发送 `StopAll`。
- `ui/pages/soundpad.slint`：删除，避免与音频库重复。

怎么跑：
```bat
scripts\\build-dev.cmd build
target\\debug\\mintcat-proc-mic.exe
```

实测：
- Debug 构建成功。
- 原生文件选择器、分类树、音频列表与双击回调已经接入编译路径。
- 双击播放使用 WAV/Media Foundation 统一解码接口，并发送到 `ring_sfx`。
- 配置写入 `audio_entries`，下次启动恢复列表。

边界：
- 分类目前是「全部 / 未分类 + 配置中已有分类」，文件夹批量导入尚未实现；当前按钮只提供单文件原生选择。
- 音频项的真实时长仍待读取 `MediaInfo.duration_hns` 后显示。
- `SendInput` Press/Hold 仍默认不启用，风险提示与配置模型已存在。
