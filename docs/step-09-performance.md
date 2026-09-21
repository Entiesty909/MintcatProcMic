# Step 9 — 体积与启动

Release profile：`opt-level=3`、`lto=true`、`codegen-units=1`、`panic=abort`、`strip=true`。

## 实测（本机 2026-09-20）

| 项 | 值 |
|---|---|
| `target/release/mintcat-proc-mic.exe` | 10 569 216 字节（≈ 10.1 MB） |
| `target/debug/mintcat-proc-mic.exe` | 30 111 744 字节（≈ 28.7 MB） |
| `--list-processes` 墙钟 | 92 ms |
| `--list-devices` 墙钟 | 73 ms |
| `--capture` 2s @ 44.1 kHz stereo PCM16 | WAV 352 844 字节，与时长吻合 |
| `--route` 3s | Start/Stop 正常，无崩溃 |

体积主要来自 Slint（software renderer + winit）。未引入 FFmpeg / cpal / tokio。  
未对比 `opt-level = "z"`：当前 `3` 下启动已 <100 ms，优先运行时而不是再挤几百 KB。

Idle/Active RAM 需 UI 长时间挂着看任务管理器；本会话未做交互窗口采样。音频线程为 event-driven WASAPI，热路径无分配（ring 预分配）。
