//! WASAPI Process Loopback：按 PID 捕获该进程树的 PCM。

use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, IAudioCaptureClient, IAudioClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVE_FORMAT_PCM, WAVEFORMATEX,
};
use windows::Win32::Media::Audio::{
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{IUnknown, Interface, PCWSTR, Result as WinResult, implement};

use crate::error::Error;

use super::buffer::SpscRing;
use super::com;
use super::format::{AudioFormat, Converter, SampleKind};
use super::wav;

/// `ActivateAudioInterfaceAsync` 完成时写入的结果。
struct ActivateState {
    /// 激活 HRESULT。
    hr: windows::core::HRESULT,
    /// 成功时的 IUnknown，随后 QI 成 IAudioClient。
    unknown: Option<IUnknown>,
}

/// COM 完成回调。字段由 implement 宏生成的 `*_Impl` 通过 Deref 访问。
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    /// 激活完成事件，调用方 Wait。
    event: HANDLE,
    /// 跨回调线程回传结果。
    state: Arc<Mutex<ActivateState>>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    /// 激活完成。在 MTA 线程池上调用，只写 state 并 SetEvent。
    fn ActivateCompleted(
        &self,
        activate_operation: windows::core::Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> WinResult<()> {
        unsafe {
            if let Some(op) = activate_operation.as_ref() {
                let mut hr = windows::core::HRESULT::default();
                let mut unk = None;
                // GetActivateResult 给出 hr + IUnknown；失败也要唤醒等待方。
                if op.GetActivateResult(&mut hr, &mut unk).is_ok()
                    && let Ok(mut guard) = self.state.lock()
                {
                    guard.hr = hr;
                    guard.unknown = unk;
                }
            }
            let _ = SetEvent(self.event);
        }
        Ok(())
    }
}

/// 已 Initialize 的 loopback 捕获客户端，供捕获线程独占使用。
pub struct CaptureClient {
    /// WASAPI 音频客户端。
    pub client: IAudioClient,
    /// 捕获服务。
    pub capture: IAudioCaptureClient,
    /// 缓冲区就绪事件。
    pub event: HANDLE,
    /// Initialize 之后实际使用的格式。
    pub format: AudioFormat,
    /// GetMixFormat 指针，Drop 时 CoTaskMemFree。
    pub wave: MixFormat,
}

/// 堆上的 WAVEFORMATEX（含 cbSize 扩展）。
pub struct MixFormat {
    /// CoTaskMemAlloc 出来的格式块。
    ptr: *mut WAVEFORMATEX,
}

// 所有权转移到捕获线程一次，之后不再共享。
unsafe impl Send for MixFormat {}
unsafe impl Send for CaptureClient {}

impl MixFormat {
    /// 借用 mix 格式。指针在 Drop 前有效。
    fn as_ref(&self) -> &WAVEFORMATEX {
        unsafe { &*self.ptr }
    }
}

impl Drop for MixFormat {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { CoTaskMemFree(Some(self.ptr as *const _)) };
            self.ptr = ptr::null_mut();
        }
    }
}

/// 按 PID 激活 Process Loopback 的 IAudioClient（尚未 Initialize）。
pub fn activate_process_loopback(pid: u32) -> Result<IAudioClient, Error> {
    // 手动复位事件：回调 SetEvent 后 Wait 返回。
    let event = unsafe { CreateEventW(None, true, false, None)? };
    let state = Arc::new(Mutex::new(ActivateState {
        hr: S_OK,
        unknown: None,
    }));
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler {
        event,
        state: state.clone(),
    }
    .into();

    // INCLUDE_TARGET_PROCESS_TREE：游戏启动器/子进程出声也能抓到。
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: windows::Win32::Media::Audio::AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };

    // blob 指向栈上 params。PROPVARIANT Drop 会 CoTaskMemFree，必须 ManuallyDrop。
    let mut activate_params =
        std::mem::ManuallyDrop::new(unsafe { std::mem::zeroed::<PROPVARIANT>() });
    unsafe {
        let inner = &mut *(*activate_params).Anonymous.Anonymous;
        ptr::write(&mut inner.vt, VT_BLOB);
        ptr::write(
            &mut inner.Anonymous.blob.cbSize,
            size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        );
        ptr::write(
            &mut inner.Anonymous.blob.pBlobData,
            &params as *const AUDIOCLIENT_ACTIVATION_PARAMS as *mut u8,
        );

        // 必须持有 IActivateAudioInterfaceAsyncOperation 直到 Wait 返回，否则激活会被取消。
        let async_op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*activate_params as *const PROPVARIANT),
            &handler,
        )?;

        let wait = WaitForSingleObject(event, 8000);
        let _ = CloseHandle(event);
        drop(async_op);
        drop(handler);
        if wait != WAIT_OBJECT_0 {
            return Err(Error::TimedOut("process loopback activate"));
        }
    }

    let (hr, unknown) = {
        let guard = state
            .lock()
            .map_err(|_| Error::CaptureInit("activate lock poisoned"))?;
        (guard.hr, guard.unknown.clone())
    };
    hr.ok()?;
    let unknown = unknown.ok_or(Error::CaptureInit("no audio client"))?;
    let client: IAudioClient = unknown.cast()?;
    Ok(client)
}

/// 单次 Initialize。失败后 IAudioClient 不能复用，调用方必须重新 activate。
fn init_capture_client(
    client: IAudioClient,
    fmt: &WAVEFORMATEX,
    flags: u32,
) -> Result<CaptureClient, Error> {
    unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, fmt, None)?;
        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        let format = AudioFormat::from_wave(fmt).ok_or(Error::Format("capture format"))?;
        Ok(CaptureClient {
            client,
            capture,
            event,
            format,
            wave: MixFormat {
                ptr: ptr::null_mut(),
            },
        })
    }
}

/// 去掉 WAVEFORMATEXTENSIBLE 尾巴。只拷 WAVEFORMATEX 且 cbSize≠0 时 Initialize 会 0x80070057。
fn sanitize_wave(fmt: &WAVEFORMATEX) -> WAVEFORMATEX {
    let channels = fmt.nChannels.max(1);
    let rate = if fmt.nSamplesPerSec == 0 {
        48_000
    } else {
        fmt.nSamplesPerSec
    };
    let bits = if fmt.wBitsPerSample == 0 {
        32
    } else {
        fmt.wBitsPerSample
    };
    let is_float = fmt.wFormatTag == 3
        || bits == 32
            && fmt.nBlockAlign == channels * 4
            && fmt.wFormatTag != WAVE_FORMAT_PCM as u16;
    if is_float {
        float_wave(rate, channels)
    } else {
        pcm_wave(rate, channels, bits)
    }
}

/// PCM WAVEFORMATEX，cbSize 必须为 0。
fn pcm_wave(sample_rate: u32, channels: u16, bits: u16) -> WAVEFORMATEX {
    let channels = channels.max(1);
    let bits = if bits == 8 || bits == 16 || bits == 24 || bits == 32 {
        bits
    } else {
        16
    };
    let bytes = (u32::from(bits) / 8) * u32::from(channels);
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: channels,
        nSamplesPerSec: sample_rate,
        nAvgBytesPerSec: sample_rate * bytes,
        nBlockAlign: bytes as u16,
        wBitsPerSample: bits,
        cbSize: 0,
    }
}

/// 立体声 PCM16。
fn pcm16_wave(sample_rate: u32) -> WAVEFORMATEX {
    pcm_wave(sample_rate, 2, 16)
}

/// IEEE float WAVEFORMATEX。
fn float_wave(sample_rate: u32, channels: u16) -> WAVEFORMATEX {
    let channels = channels.max(1);
    let bytes = 4u32 * u32::from(channels);
    WAVEFORMATEX {
        wFormatTag: 3,
        nChannels: channels,
        nSamplesPerSec: sample_rate,
        nAvgBytesPerSec: sample_rate * bytes,
        nBlockAlign: bytes as u16,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// 进程环回 Initialize。本机实测 44.1k PCM16 + LOOPBACK|AUTOCONVERT 能成功。
pub fn open_process_capture(
    pid: u32,
    preferred: Option<&WAVEFORMATEX>,
) -> Result<CaptureClient, Error> {
    let conv = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    let ms = conv | AUDCLNT_STREAMFLAGS_LOOPBACK;
    let mut attempts: Vec<(WAVEFORMATEX, u32, &'static str)> = Vec::new();
    attempts.push((pcm16_wave(44_100), ms, "44100-pcm-loopback"));
    attempts.push((pcm16_wave(48_000), ms, "48000-pcm-loopback"));
    if let Some(fmt) = preferred {
        let fmt = sanitize_wave(fmt);
        attempts.push((fmt, conv, "preferred"));
        attempts.push((fmt, ms, "preferred-loopback"));
    }
    attempts.push((float_wave(48_000, 2), ms, "48000-f32-loopback"));
    attempts.push((pcm16_wave(44_100), conv, "44100-pcm"));

    let mut last: Option<Error> = None;
    for (fmt, flags, name) in attempts {
        let client = match activate_process_loopback(pid) {
            Ok(c) => c,
            Err(e) => {
                last = Some(e);
                continue;
            }
        };
        match init_capture_client(client, &fmt, flags) {
            Ok(c) => {
                tracing::info!(
                    "capture initialize ok ({name}, {} Hz {} ch)",
                    c.format.sample_rate,
                    c.format.channels
                );
                return Ok(c);
            }
            Err(e) => {
                tracing::debug!("capture initialize {name} failed: {e}");
                last = Some(e);
            }
        }
    }

    match open_process_capture_native_mix(pid) {
        Ok(c) => {
            tracing::info!(
                "capture initialize ok (loopback-mix, {} Hz {} ch)",
                c.format.sample_rate,
                c.format.channels
            );
            Ok(c)
        }
        Err(e) => Err(last.unwrap_or(e)),
    }
}

/// 用进程环回客户端自己的 mix 格式 Initialize。
fn open_process_capture_native_mix(pid: u32) -> Result<CaptureClient, Error> {
    let client = activate_process_loopback(pid)?;
    unsafe {
        let mix = client.GetMixFormat()?;
        if mix.is_null() {
            return Err(Error::Format("loopback mix format null"));
        }
        let event = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        let with_loop = event | AUDCLNT_STREAMFLAGS_LOOPBACK;
        let result = init_capture_client(client, &*mix, with_loop).or_else(|_| {
            let client = activate_process_loopback(pid)?;
            init_capture_client(client, &*mix, event)
        });
        CoTaskMemFree(Some(mix as *const _));
        result
    }
}

/// 打开物理麦克风（普通 WASAPI 捕获，不是 loopback）。
pub fn open_mic_capture(
    device_id: &str,
    preferred: Option<&WAVEFORMATEX>,
) -> Result<CaptureClient, Error> {
    let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if let Some(fmt) = preferred {
        let fmt = sanitize_wave(fmt);
        let client = activate_mic_device(device_id)?;
        match init_capture_client(client, &fmt, flags) {
            Ok(c) => {
                tracing::info!("mic initialize ok (preferred)");
                return Ok(c);
            }
            Err(e) => tracing::debug!("mic preferred format failed: {e}"),
        }
    }
    let client = activate_mic_device(device_id)?;
    unsafe {
        let mix = client.GetMixFormat()?;
        if mix.is_null() {
            return Err(Error::Format("mic mix format null"));
        }
        let result = init_capture_client(client, &*mix, AUDCLNT_STREAMFLAGS_EVENTCALLBACK);
        CoTaskMemFree(Some(mix as *const _));
        result
    }
}

/// 按设备 ID 取出录音端 IAudioClient。
fn activate_mic_device(device_id: &str) -> Result<IAudioClient, Error> {
    let _com = com::init_mta()?;
    let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDevice(PCWSTR(wide.as_ptr()))?;
        Ok(device.Activate(CLSCTX_ALL, None)?)
    }
}

/// 捕获线程主循环：等事件 → 取包 → 转 f32 → 推进 ring。
pub fn run_capture_loop(
    capture: CaptureClient,
    ring: Arc<SpscRing>,
    mut converter: Converter,
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU32>,
    on_fail: impl FnOnce(Error),
) {
    let CaptureClient {
        client,
        capture,
        event,
        format,
        wave: _wave,
    } = capture;

    if let Err(e) = unsafe { client.Start() } {
        on_fail(e.into());
        unsafe {
            let _ = CloseHandle(event);
        }
        return;
    }

    // 预分配，循环内只 clear/extend，不扩 cap（包大小通常稳定）。
    let mut converted = Vec::with_capacity(8192);
    let mut failed = None;
    while !stop.load(Ordering::Acquire) {
        let wait = unsafe { WaitForSingleObject(event, 80) };
        if wait == WAIT_TIMEOUT {
            continue;
        }
        if wait != WAIT_OBJECT_0 {
            failed = Some(Error::CaptureInit("capture wait failed"));
            break;
        }
        loop {
            let packet = match unsafe { capture.GetNextPacketSize() } {
                Ok(n) => n,
                Err(e) => {
                    failed = Some(classify_capture_error(e));
                    break;
                }
            };
            if packet == 0 {
                break;
            }
            let mut data: *mut u8 = ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            if let Err(e) =
                unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
            {
                failed = Some(classify_capture_error(e));
                break;
            }
            // SILENT 标志表示这一包应写成零，不要读野指针。
            let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
            let nbytes = frames as usize * format.frame_bytes();
            let bytes = if data.is_null() || silent {
                &[][..]
            } else {
                unsafe { std::slice::from_raw_parts(data, nbytes) }
            };
            converter.convert(
                bytes,
                frames as usize,
                silent || data.is_null(),
                &mut converted,
            );
            super::peak::hold_peak(&peak, &converted);
            ring.push_latest(&converted);
            if let Err(e) = unsafe { capture.ReleaseBuffer(frames) } {
                failed = Some(classify_capture_error(e));
                break;
            }
        }
        if failed.is_some() {
            break;
        }
    }

    let _ = unsafe { client.Stop() };
    unsafe {
        let _ = CloseHandle(event);
    }
    if let Some(e) = failed
        && !stop.load(Ordering::Acquire)
    {
        on_fail(e);
    }
}

/// 把 WASAPI 失效码映射成进程退出，其余原样上抛。
fn classify_capture_error(e: windows::core::Error) -> Error {
    let code = e.code().0 as u32;
    // AUDCLNT_E_DEVICE_INVALIDATED / AUDCLNT_E_RESOURCES_INVALIDATED
    if code == 0x8889_0004 || code == 0x8889_0013 {
        Error::ProcessNotFound(0)
    } else {
        e.into()
    }
}

/// CLI 验证：Process Loopback → PCM16 WAV。不经过 ring / render。
pub fn capture_process_to_wav(pid: u32, path: &Path, seconds: f32) -> Result<(), Error> {
    let _com = com::init_mta()?;
    let pcm16 = pcm16_wave(44_100);
    let capture = open_process_capture(pid, Some(&pcm16))?;

    let src_format = capture.format;
    let mut converter = Converter::new(
        src_format,
        AudioFormat {
            sample_rate: src_format.sample_rate,
            channels: src_format.channels,
            kind: SampleKind::F32,
        },
    );

    unsafe { capture.client.Start()? };

    let mut collected = Vec::new();
    let mut converted = Vec::with_capacity(8192);
    let deadline = Instant::now() + Duration::from_secs_f32(seconds.max(0.2));
    while Instant::now() < deadline {
        let wait = unsafe { WaitForSingleObject(capture.event, 80) };
        if wait == WAIT_TIMEOUT {
            continue;
        }
        if wait != WAIT_OBJECT_0 {
            break;
        }
        loop {
            let packet = unsafe { capture.capture.GetNextPacketSize()? };
            if packet == 0 {
                break;
            }
            let mut data: *mut u8 = ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            unsafe {
                capture
                    .capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            }
            let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
            let nbytes = frames as usize * src_format.frame_bytes();
            let bytes = if data.is_null() || silent {
                &[][..]
            } else {
                unsafe { std::slice::from_raw_parts(data, nbytes) }
            };
            converter.convert(
                bytes,
                frames as usize,
                silent || data.is_null(),
                &mut converted,
            );
            collected.extend_from_slice(&converted);
            unsafe { capture.capture.ReleaseBuffer(frames)? };
        }
    }

    unsafe {
        let _ = capture.client.Stop();
        let _ = CloseHandle(capture.event);
    }

    wav::write_pcm16_wav(
        path,
        src_format.sample_rate,
        src_format.channels,
        &collected,
    )
}
