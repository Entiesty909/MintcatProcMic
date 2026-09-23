# Step 22 — 默认设备角色与多进程输入基础
范围：恢复设置页默认播放/默认通信设备选项；路由页物理麦开关已移到开始转发状态栏；多进程输入后端增加四个进程源槽位。多输出 fan-out 尚未完成。

完成：
- 设置页恢复：
  - 设为默认麦克风。
  - 设为系统默认播放设备。
  - 设为默认通信播放设备。
- 默认设备 role 写入配置，使用现有 PolicyConfig 设置 Windows endpoint role。
- 路由页「混入物理麦」只在设置页已选物理麦时启用。
- 多进程输入：最多四个独立 Process Loopback SPSC ring，路由页可加入/清空进程源。
- 每个进程源独立捕获线程，render 侧汇总多个 ring。

怎么跑：
```bat
scripts\\build-dev.cmd build
target\\debug\\mintcat-proc-mic.exe --route <pid> --device <wasapi-id> --seconds 1
```

实测：
- Debug 构建通过。
- 旧单进程 CLI 路由兼容入口保留。

未完成：
- 多输出设备 fan-out：需要每个输出独立 render ring 和独立格式转换，不能共享同一消费 ring。
- 音频预览总线与预览音量。
- 音频分类树右键新增节点。

下一步：
- 单独实现输出 fan-out 与预览 render，避免把不同设备格式硬塞进同一 render loop。
