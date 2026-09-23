# Step 18 — 三页音频库 UI 重排
范围：按用户给的 Soundpad 参考图，删除重复的独立「声板」页；播放器改为统一「音频库」页，播放时按键移到设置页作为全局行为。

改动文件：
- `ui/components/tab-bar.slint`：顶栏从四页改为三页：路由 / 音频库 / 设置。
- `ui/main.slint`：删除 Soundpad 页面分支；PlayerPage 作为音频库页；设置页接入全局播放时按键模式与显示。
- `ui/pages/player.slint`：改为音频库方向的文件入口与当前播放信息，不再放「播放时按键在设置页配置」的独立多余卡片。
- `ui/pages/settings.slint`：增加全局「播放时按键」设置行：不按键 / 播放开始按一次 / 播放期间一直按，并显示风险说明。
- `src/ui_bridge.rs`：持久化全局播放时按键模式。
- `ui/pages/soundpad.slint`：删除，不再保留重复页面。
- `docs/ui-redesign-proposal.md`：更新为三页架构与左分类树/右音频列表设计。
- `docs/rust-architecture.md`：同步三页 UI 事实。

最终信息架构：
- 路由：进程声、物理麦、输出、混音、开始/停止。
- 音频库：左侧可折叠分类树，右侧当前分类音频列表，双击音频播放到当前目标，底部迷你播放条。
- 设置：设备、混音、热键、全局播放时按键、风险提示。

怎么跑：
```bat
scripts\\build-dev.cmd build
target\\debug\\mintcat-proc-mic.exe
```

实测：
- Debug 构建成功。
- 三页 Slint 页面编译成功。
- 音频库页面不再显示独立「播放时按键在设置页配置」卡片。
- 全局播放时按键说明与模式选择位于设置页。

明确边界：
- 音频库的真实分类树、音频列表数据绑定、双击发送 SFX 命令仍是后续接线工作；当前先完成 UI 信息架构，避免继续沿用重复的声板/播放器页面模型。
- `SendInput` Press/Hold 仍未启用，默认不注入输入。

下一步：
- 接入音频库数据模型、分类树、双击播放与 `AudioEngine::sfx_sender()`。
