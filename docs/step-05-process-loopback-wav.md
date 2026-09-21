# Step 5 — Process Loopback → WAV

实现：`src/audio/capture.rs`，CLI `--capture <pid> --wav <path> --seconds N`。

## 关键结论

1. `ActivateAudioInterfaceAsync(VAD\Process_Loopback)` **可以**拿到 `IAudioClient`。必须持有返回的 `IActivateAudioInterfaceAsyncOperation` 直到完成回调。
2. `PROPVARIANT` 的 blob 若指向栈，**禁止**让 `PROPVARIANT` Drop（会 `CoTaskMemFree` 栈地址 → 堆损坏）。使用 `ManuallyDrop`。
3. 进程环回上 `GetMixFormat` 在 Initialize 前返回 `E_NOTIMPL`。
4. 成功的 Initialize 标志（与微软 ApplicationLoopback 一致）：

   `LOOPBACK | EVENTCALLBACK | AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`

   格式：PCM 16-bit stereo 44100。不要先 `GetMixFormat`。Initialize 失败后必须重新 activate。

## 实测

```
--capture 21712 --wav capture.wav --seconds 2
INFO capture initialize ok (preferred+ms-flags)
wrote capture.wav
```

文件 `352844` 字节 = 44 字节头 + `44100 * 2ch * 2bytes * 2s`。格式正确。
