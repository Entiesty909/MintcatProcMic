# Step 7–8 — Slint UI / 音量 / 启停 / 状态

实现：`ui/main.slint`、`src/ui_bridge.rs`。无参数启动 UI。

## 界面

- 音频来源 ComboBox（进程）
- 输出设备 ComboBox
- 音量滑条 0–100，立即写入引擎 `AtomicU32`
- 状态：未运行 / 运行中 / 进程已退出 / 设备已断开
- 开始转发 / 停止
- 刷新列表（运行中禁用）

## 错误短句

- `无法连接到音频设备` / `设备可能已经被拔出。`
- `无法捕获该进程的音频`
- `进程已退出`

HRESULT 只进 tracing。

## 验证

引擎与 CLI `--route` 共用。无参数启动走同一 `AudioEngine`。  
本环境未做交互点击（会挡住会话）；CLI 已证明 Start/Stop/捕获/渲染。
