# Step 6 — Capture → Ring → WASAPI Render

实现：`src/engine.rs` + `src/audio/{buffer,capture,render,format}.rs`。  
CLI：`--route <pid> --device <id> --seconds N`。

## 链路

```
PID → Process Loopback → Converter(f32) → SPSC ring → volume → WASAPI Shared Render
```

- 捕获 / 渲染分线程，UI 不碰 PCM
- ring 满丢最旧；空则渲染静音
- 捕获 44100 PCM16 时，渲染端 mix 往往是 48k float，由 `Converter` 线性重采样

## 实测

```
--route 21712 --device {0.0.0.00000000}.{d6777f57-...} --seconds 3
WARN preferred+autoconvert 0x88890021 (UNSUPPORTED_FORMAT)
WARN preferred+ms-flags 0x80070057 (render mix 不能直接当 loopback 格式)
INFO capture initialize ok (44100-pcm-ms-sample)
routing pid 21712 for 3s
stopped (Idle)
```

引擎 Start/Stop 正常，无堆损坏、无 panic。本机默认设备为 Realtek 扬声器。
