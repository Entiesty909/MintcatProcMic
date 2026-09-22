# Step 14 — WAV 声板与播放模式
范围：完成 WAV 解码、独立 sfx 混音线程、固定 `ring_sfx` 生产入口与播放模式数据模型；UI pad 网格与真正热键触发仍在 S6 接入。

改动文件：
- `src/audio/wav.rs`：新增 WAV 读取，支持 PCM 8/16/24/32-bit 与 IEEE float 32-bit，输出交错 f32。
- `src/audio/sfx.rs`：新增 10 ms 声板混音线程、最多 8 路播放器、Once / Loop / Hold / Toggle 状态模型、SPSC ring 生产。
- `src/engine.rs`：输出打开时启动 sfx 线程；暴露 `sfx_sender()`，render 使用真实音量原子值；停止时 join sfx 线程。
- `src/audio/render.rs`：消费三条 ring，按进程/麦/音效总线音量混合。
- `src/config.rs`：全局 `PlaybackKeyAction` 数据模型，模式为 None / Press / Hold；明确不挂在单个音频项上。
- `Cargo.lock`：同步 serde 依赖。

播放行为目标：
- 声音项的播放模式：单次、循环、按住、切换。
- 全局「播放时按键」动作：不按、按一次、播放期间保持按下。
- 当前阶段只保存动作配置，实际 `SendInput` 与风险提示由 S6 接入；默认 None，不会注入任何键盘输入。

怎么跑：
```bat
scripts\build-dev.cmd build
target\debug\mintcat-proc-mic.exe --route <pid> --device <wasapi-id> --seconds 1
```

实测：
- `scripts\build-dev.cmd build`：成功。
- `--route 25772 --device {0.0.0.00000000}.{8244a1cb-827e-4930-8cf6-b7858bfd501e} --seconds 1`：成功。
- Process Loopback 初始化成功，常驻输出与 sfx 线程不会破坏旧 CLI 路由。
- WAV 解码代码已编译；真实文件播放验证等待 UI/文件选择入口接入。

下一步：
- Step 15：Media Foundation 文件播放器，mp3/mp4/m4a 等文件通过 `ring_sfx` 转发。
