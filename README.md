# ProcessMic

轻量 Windows 进程音频路由器。把指定进程的声音经 WASAPI Process Loopback 送到所选输出设备。

原 C# DRG SoundPad 仅作功能参考，见 `docs/architecture-analysis.md`。Rust 实现不是逐行移植。

## 要求

- Windows 10 21H1+ / Windows 11（Process Loopback）
- 本机已用 `stable-x86_64-pc-windows-msvc` 构建

## 构建

```bat
call "D:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cargo build --release
```

或 `scripts\build-dev.cmd build --release`

## 运行

```text
target\release\mintcat-proc-mic.exe                 UI
--list-processes
--list-devices
--capture <pid> --wav capture.wav --seconds 5
--route <pid> --device <wasapi-id> --seconds 8
```

日志：`RUST_LOG=info`。音频热路径不打日志。

## 文档

| 文件 | 内容 |
|---|---|
| `AGENTS.md` | 给智能体的约束、代码规范、开始前/后流程 |
| `docs/architecture-analysis.md` | C# 原项目分析 |
| `docs/rust-architecture.md` | Rust 架构 |
| `docs/step-00-toolchain.md` | rustc / MSVC |
| `docs/step-03-*.md` … | 各阶段实测 |

## 许可

MIT
