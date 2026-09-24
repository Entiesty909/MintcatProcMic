# Step 31 — 长音频流式播放与右键菜单
范围：长音频从“完整解码后才播放”改为有界分块解码，SFX 与本机试听各消费一条流；分类和音频列表均提供右键菜单。未改变进程捕获与输出设备架构。
改动文件：
- `src/audio/decode.rs`：新增有界 `sync_channel` 分块解码，MF 解码器在后台线程打开，避免把 COM reader 跨线程移动。
- `src/audio/sfx.rs`：新增流式播放器，收到首块后即可进入渲染 ring。
- `src/audio/preview.rs`：新增流式本机试听消费者。
- `src/config.rs`：音频条目保存独立热键字段。
- `src/ui_bridge.rs`：连接流式播放、试听、音频右键动作、文件移除、Explorer 定位和音频热键捕获。
- `ui/pages/player.slint`、`ui/main.slint`：分类右键菜单与音频右键菜单。
菜单动作：播放（扬声器和麦克风）、扬声器、麦克风、移除、设置/移除热键、Explorer 定位、编辑文件。
怎么跑：
- `scripts\\build-dev.cmd build`
- 无参数启动 UI，在音频库中右键分类或音频条目。
- 长音频双击后应先开始流式输出，不等待整文件读完。
实测：
- 2026-09-24，Windows x64：构建成功，仅有 dead-code 警告。
- 原生 UI 进程启动并持续运行，随后停止。
- `--list-devices` 与真实右键点击/扬声器听感仍需桌面手动验收。
