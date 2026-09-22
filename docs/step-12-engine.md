# Step 12 — 常驻输出与动态音频源
范围：把一次性三线程启动改成常驻输出 + 动态进程/物理麦源；为 `ring_sfx` 预留声板与文件播放器入口。

改动文件：
- `src/engine.rs`
  - 新增 `open_output` / `set_process_source` / `set_mic_source`。
  - 输出线程独立于源线程，拥有固定 `process_ring`、`mic_ring`、`sfx_ring`。
  - 保留 `start(pid, device, mic)` 兼容 CLI/UI，一次调用内部组合新 API。
  - 增加音效总线音量字段，为 S4/S5 使用。
  - 每个动态源有独立停止令牌与线程句柄，替换源前先停旧线程并 join。
- `src/audio/render.rs`
  - render 线程同时消费进程、物理麦、音效三条 SPSC ring。
  - 三路先乘各自音量，再统一 clamp，最后乘 master 音量。
- `docs/rust-architecture.md`
  - 同步常驻输出、动态源与三条 ring 的事实。

怎么跑：
```bat
scripts\build-dev.cmd build
target\debug\mintcat-proc-mic.exe --route <pid> --device <wasapi-id> --seconds 1
```

实测：
- `scripts\build-dev.cmd build`：成功。
- `--route 25772 --device {0.0.0.00000000}.{8244a1cb-827e-4930-8cf6-b7858bfd501e} --seconds 1`：成功。
- 日志确认 Process Loopback 初始化：`44100-pcm-loopback, 44100 Hz 2 ch`。
- Start/Stop 正常返回 `Idle`，无崩溃。
- 曾发现动态源入口缺少 COM apartment；已在 `set_process_source` 与 `set_mic_source` 内显式初始化 MTA，并回归通过。

已知待后续阶段处理：
- `ring_sfx` 当前只由 render 消费，S4 接入声板生产线程。
- `set_sfx_volume` 在 S4 接入 UI。
- 仍有 5 个警告：3 个原有未使用方法，另有 `sfx_ring` / `set_sfx_volume` 将在后续阶段使用。

下一步：
- Step 13：serde 配置模型、配置版本迁移、单实例互斥、记住设备/进程与自动重连状态。
