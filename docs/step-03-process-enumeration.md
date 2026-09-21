# Step 3 — 进程枚举

实现：`src/process.rs`，CLI `--list-processes`。

## 行为

- `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`
- 可见顶层窗口标题：`EnumWindows` + `GetWindowTextW`
- 排除 PID 0/4 和自身（避免回授）
- 有窗口的排前面，其余按名称

## 实测

`cargo run -- --list-processes` 成功列出本机进程，含窗口标题，例如：

- `explorer.exe  [9352]  MintcatProcMic - 文件资源管理器`
- `msedge.exe  [19820]  ... Microsoft Edge`
- `piliplus.exe  [21712]  PiliPlus`
- `QQ.exe` / `Weixin.exe` / `steam.exe`

无崩溃，无需管理员。
