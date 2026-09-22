# Step 16 — 多模式热键与全局播放按键
范围：保留系统热键、全局 hook、关闭三档模式；完成系统热键安全默认与配置模型；全局播放按键动作已进入配置但尚未执行 `SendInput`。

改动文件：
- `src/hotkey.rs`：`HotkeyMode` 三档；系统热键模式保留 `RegisterHotKey`；GlobalHook 当前安全回退为不注册，避免半成品 hook 拦截游戏输入。
- `src/config.rs`：`HotkeyModeConfig`；全局 `PlaybackKeyAction`（None / Press / Hold），不挂在固定音频项上。
- `ui/pages/player.slint`：播放器页明确提示「播放时按键」是全局设置。

目标行为：
- 播放模式：单次 / 循环 / 按住 / 切换。
- 播放时全局按键：
  - None：不注入输入。
  - Press：播放开始时按一次并释放。
  - Hold：播放期间保持按下，停止/切换/EOF 时释放。
- GlobalHook 与 `SendInput` 属于有风险输入路径：设置页必须提示反作弊、权限与系统输入影响；默认仍使用 `RegisterHotKey`。

怎么跑：
```bat
scripts\\build-dev.cmd build
target\\debug\\mintcat-proc-mic.exe --list-processes
```

实测：
- `scripts\\build-dev.cmd build`：成功。
- 系统热键模式编译通过，旧 `Ctrl+Shift+F8` 路径保持。
- GlobalHook/SendInput 仍未声称完成，避免把未实现的全局拦截交给用户。

下一步：
- Step 17：性能实测、真实 WAV/MP4 文件播放烟测、清理阶段性警告、同步全部架构文档。
