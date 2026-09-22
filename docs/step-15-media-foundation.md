# Step 15 — Media Foundation 文件播放器
范围：完成 Media Foundation 音频读取后端与播放器页输入/循环/播放入口；实际把文件命令发送到 SFX 线程的完整 UI 联动仍需继续收口。

改动文件：
- `Cargo.toml`：加入 `Win32_Media_MediaFoundation` feature，并恢复 `Win32_System_Threading`。
- `src/audio/decode.rs`：
  - `MFStartup(MF_VERSION, MFSTARTUP_LITE)` 懒初始化。
  - `MFCreateSourceReaderFromURL` 打开文件。
  - 只选择音频流，视频轨不解码。
  - 请求交错 f32 输出，读取原始采样率/声道。
  - 支持 seek、读取块、读取完整文件；错误映射为 `Error::Format`。
- `src/audio/wav.rs`：统一 WAV 读取与 f32 输出。
- `ui/pages/player.slint`：文件路径、循环、播放/停止、当前输出、全局播放按键说明。
- `ui/main.slint`：接入播放器状态字段。

当前支持计划：
- WAV：自写解析。
- MP3 / MP4 / M4A / FLAC / WMA：Media Foundation。
- MP4 只读音轨，不播放画面。
- 播放时按键是全局 `AppConfig.playback_key`，不是固定音频属性；默认关闭。

怎么跑：
```bat
scripts\\build-dev.cmd build
target\\debug\\mintcat-proc-mic.exe --route <pid> --device <wasapi-id> --seconds 1
```

实测：
- `scripts\\build-dev.cmd build`：成功。
- 旧 CLI Process Loopback 路由回归仍通过。
- Media Foundation 代码已通过 windows-rs 0.62.2 编译。
- 未声称真实 MP4 播放已完成：文件路径到 SFX 命令的最终 UI 回调与实际文件样本回归仍需继续。

下一步：
- Step 16：接入播放器/声板真实播放命令、全局 `SendInput` 按键动作、三档热键方式与风险提示。
