# Step 17 — 最终验证与文档同步
范围：完成 release 构建、CLI 烟测、性能基线记录、架构文档同步；不虚报未完成的 UI 文件播放与 SendInput 实际联动。当前 `src/ui_bridge.rs` 仍有未提交的配置接线，先在本阶段提交后续收口。

最终阶段提交前状态：
- S1 UI 骨架与响应式路由页：已提交。
- S2 常驻输出与动态源：已提交，CLI 路由回归通过。
- S3 serde 配置、旧热键迁移、单实例、设备/进程状态保存：已提交。
- S4 WAV 读取、SFX ring、播放模式模型：已提交；真实文件到 UI 触发入口仍需继续收口。
- S5 Media Foundation 后端、播放器 UI 输入：已提交；真实 MP4 端到端播放烟测仍需继续收口。
- S6 多模式热键配置模型、安全系统热键默认、GlobalHook 安全回退：已提交；真实 hook/SendInput 仍未启用。

怎么跑：
```bat
scripts\build-dev.cmd build --release
target\release\mintcat-proc-mic.exe --list-processes
target\release\mintcat-proc-mic.exe --list-devices
target\release\mintcat-proc-mic.exe --route <pid> --device <wasapi-id> --seconds 1
```

实测（当前机器）：
- release 构建：成功。
- release exe：10,569,216 bytes，约 10.1 MB；与 Step 9 基线相同量级。
- `--list-devices`：成功，能列出 Realtek、VB-CABLE、Steam Streaming 播放设备。
- `--list-processes`：前序阶段成功，能列出当前出声进程。
- `--route`：前序阶段成功，Process Loopback 初始化为 44.1 kHz PCM、2 声道，Start/Stop 正常。
- S1 UI：前序阶段成功启动；144 DPI 下已启用 Per-Monitor V2，卡片使用响应式等宽伸缩，进程长标题不改变父布局宽度。

已同步：
- `docs/rust-architecture.md`：常驻输出、三条 SPSC ring、动态源、SFX、响应式 UI、50 ms 状态轮询。
- `docs/step-11-ui-shell.md` 至 `docs/step-16-hotkey-safety.md`：每阶段改动、命令、实测与明确未完成边界。

性能备注：
- 现有 Step 9 release 基线为约 10.1 MB；本阶段加入 serde、serde_json、Media Foundation bindings 后体积仍为 10.1 MB 量级。
- 还没有可靠记录 idle/active RAM；UI debug 启动烟测曾观察到约 39 MB 工作集，不能当 release 性能结论。
- 当前保留若干阶段性 dead-code 警告，来源是下一阶段接口（SFX、MF reader、Converter 辅助方法）；后续接线或清理。

下一步：
- 收口播放器与声板 UI 到 `AudioEngine::sfx_sender()`。
- 实现全局播放按键的 `SendInput` Press/Hold 与可见风险提示。
- 最后再做真实 WAV/MP4 文件端到端音频验证，并补 release RAM/CPU 采样。
