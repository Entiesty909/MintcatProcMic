# Step 10 — 推迟（goal.md 第二/三阶段）

未实现，架构已预留状态枚举：

- 系统托盘
- 进程退出自动重连
- 设备热插拔恢复
- JSON 配置落盘
- 快捷键
- 多进程混音 / 多输出
- DRG Mod / TTS / 命名管道

捕获线程失败会把 `EngineStatus` 设为 `ProcessExited` / `DeviceGone`，UI 能显示，但不会自动再 `Start`。
