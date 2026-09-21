# Step 4 — 音频设备枚举

实现：`src/audio/device.rs`，CLI `--list-devices`。

## 行为

- `IMMDeviceEnumerator::EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)`
- 设备 ID + `PKEY_Device_FriendlyName`
- 默认 eConsole 设备标注 `(默认)`

## 实测

```
{0.0.0.00000000}.{d6777f57-...}  扬声器 (Realtek(R) Audio)  (默认)
{0.0.0.00000000}.{86dda696-...}  扬声器 (Steam Streaming Microphone)
{0.0.0.00000000}.{3520d1eb-...}  扬声器 (Steam Streaming Speakers)
```

本机未装 VB-CABLE。MVP 不内置安装器；列表里出现 CABLE 即可选。
