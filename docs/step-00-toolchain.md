# 工具链安装记录

本机原先没有 `rustc` / `cargo`。已安装并核实。

| 项 | 值 |
|---|---|
| 安装方式 | `winget install --id Rustlang.Rustup` |
| rustup | 1.29.1 |
| rustc | 1.98.1 (48a229cea 2026-09-01) |
| 默认 toolchain | `stable-x86_64-pc-windows-msvc` |
| cargo bin | `C:\Users\12565\.cargo\bin` |
| MSVC | Visual Studio Community 2022 17.14，`cl` 19.44.35217 |
| VS 路径 | `D:\Program Files\Microsoft Visual Studio\2022\Community` |
| Windows SDK | `D:\Windows Kits\10` `10.0.26100.0` |
| vcvars | `...\VC\Auxiliary\Build\vcvars64.bat` |

未新装 VS：本机已有带 `VC.Tools.x86.x64` 的 Community 2022。  
Windows SDK 不在 `C:\Program Files (x86)\Windows Kits`，在 D 盘。

构建时优先走 `vcvars64.bat`，避免 Git 自带的 `link.exe` 抢 PATH。
