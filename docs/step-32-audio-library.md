# Step 32 — 音频库嵌套分类、时长与播放进度
范围：实现音频库 Win11 风格右键菜单、任意层级子分类、正确索引、时长列，并修复流式播放边界重复与进度不重置。未改变进程环回、物理麦克风和输出设备路由。

改动文件：
- `ui/pages/player.slint`：重写音频库列表列布局；索引使用 Slint 插值；增加时长列；右键菜单改为白色圆角悬浮菜单，分类菜单支持新建子分类。
- `ui/main.slint`：增加音频时长列表和当前播放时长属性绑定。
- `src/config.rs`：`AudioEntry` 保存 `duration_hns`；旧配置缺失字段按 0 兼容。
- `src/audio/wav.rs`、`src/audio/decode.rs`：WAV 头部计算时长；MF 样本块保留超出块大小的剩余样本，seek 时清空余量。
- `src/audio/sfx.rs`、`src/audio/preview.rs`：按实际消费帧数清理流式缓冲，不再保留已播放帧；试听流正确处理 EOF。
- `src/ui_bridge.rs`：分类路径使用 `/` 表示父子关系；父分类筛选包含后代；导入和旧配置补齐时长；播放切换清零进度和时长；每 50 ms 更新进度文本。
- `docs/step-02-rust-architecture.md`：同步音频库架构事实。

行为：
- 分类树内部保存路径，显示时按层级缩进；重命名父分类会递归更新后代和音频条目；删除父分类会把后代音频移到「未分类」。
- 当前筛选结果索引从 1 开始，不再显示字面量 `\\{index`。
- 时长显示为 `分:秒` 或 `时:分:秒`；无效或无法读取的旧文件显示 `—`。
- 双击切换音频前停止旧 SFX/试听源、取消旧解码并把进度归零；流式解码不丢弃媒体样本块尾部。

怎么跑：
- `scripts\\build-dev.cmd fmt -- --check`
- `scripts\\build-dev.cmd build`
- `target\\debug\\mintcat-proc-mic.exe --list-devices`
- 启动无参数 UI，在音频库右键分类创建子分类，继续右键子分类创建下一层；导入音频后检查索引与时长；双击不同音频检查进度从 0 开始。

实测：
- 2026-09-26，Windows x64：格式检查通过，Debug 构建成功，仅保留既有 dead-code/unused 警告。
- `--list-devices` 成功枚举 Realtek、VB-CABLE 和 Steam Streaming 设备。
- Debug UI 进程可启动；当前环境只做进程级冒烟，未进行桌面手工听感与鼠标交互验收。

下一步：
- 在真实音频文件上手工验收长音频、循环播放和嵌套分类拖拽/操作体验。
