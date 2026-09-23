//! WASAPI 共享模式渲染：从 ring 取 f32，写到用户选的设备。

use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient, IAudioRenderClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::error::Error;

use super::buffer::SpscRing;
use super::com;
use super::format::{encode_from_f32, AudioFormat, SampleKind};

/// 已 Initialize 的渲染客户端，由渲染线程独占。
pub struct RenderClient {
    /// WASAPI 客户端。
    pub client: IAudioClient,
    /// 渲染服务。
    pub render: IAudioRenderClient,
    /// 缓冲区可写事件。
    pub event: HANDLE,
    /// 设备 mix 格式。
    pub format: AudioFormat,
    /// 引擎分配的帧数。
    pub buffer_frames: u32,
    /// GetMixFormat 指针，Drop 时释放。
    mix: *mut WAVEFORMATEX,
}

// COM 接口在 MTA 下所有权转移到渲染线程。
unsafe impl Send for RenderClient {}

impl Drop for RenderClient {
    fn drop(&mut self) {
        // 事件和 mix 都在这里收，避免拆结构体时 Drop 被跳过。
        unsafe {
            if !self.event.is_invalid() {
                let _ = CloseHandle(self.event);
                self.event = HANDLE::default();
            }
            if !self.mix.is_null() {
                CoTaskMemFree(Some(self.mix as *const _));
                self.mix = ptr::null_mut();
            }
        }
    }
}

impl RenderClient {
    /// 设备 mix WAVEFORMATEX，捕获侧 AUTOCONVERT 会克隆它。
    pub fn mix_format(&self) -> &WAVEFORMATEX {
        unsafe { &*self.mix }
    }
}

/// 按 WASAPI 设备 ID 打开共享模式渲染流。
pub fn open_render_device(device_id: &str) -> Result<RenderClient, Error> {
    let _com = com::init_mta()?;
    let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDevice(windows::core::PCWSTR(wide.as_ptr()))?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let mix = client.GetMixFormat()?;
        if mix.is_null() {
            return Err(Error::Format("render mix format null"));
        }
        let format = AudioFormat::from_wave(&*mix).ok_or(Error::Format("render mix format"))?;
        let event = CreateEventW(None, false, false, None)?;
        // 共享模式必须用 mix format；hns 填 0 让引擎选周期。
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            0,
            0,
            &*mix,
            None,
        )?;
        client.SetEventHandle(event)?;
        let buffer_frames = client.GetBufferSize()?;
        let render: IAudioRenderClient = client.GetService()?;
        Ok(RenderClient {
            client,
            render,
            event,
            format,
            buffer_frames,
            mix,
        })
    }
}

/// 渲染线程：消费多个进程 ring、麦 ring、SFX ring，再写一个输出设备。
pub fn run_render_loop(
    session: RenderClient,
    process_rings: Vec<Arc<SpscRing>>,
    mic_ring: Option<Arc<SpscRing>>,
    sfx_ring: Option<Arc<SpscRing>>,
    volume: Arc<AtomicU32>,
    mic_volume: Arc<AtomicU32>,
    sfx_volume: Arc<AtomicU32>,
    master: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU32>,
    on_fail: impl FnOnce(Error),
) {
    if let Err(e) = unsafe { session.client.Start() } {
        on_fail(e.into());
        return;
    }

    let channels = session.format.channels as usize;
    let buffer_frames = session.buffer_frames;
    let mut tmp = vec![0.0f32; buffer_frames as usize * channels];
    let mut mic_tmp = vec![0.0f32; buffer_frames as usize * channels];
    let mut sfx_tmp = vec![0.0f32; buffer_frames as usize * channels];
    let mut failed = None;

    while !stop.load(Ordering::Acquire) {
        let wait = unsafe { WaitForSingleObject(session.event, 80) };
        if wait == WAIT_TIMEOUT {
            continue;
        }
        if wait != WAIT_OBJECT_0 {
            failed = Some(Error::RenderInit("render wait failed"));
            break;
        }
        let padding = match unsafe { session.client.GetCurrentPadding() } {
            Ok(p) => p,
            Err(e) => {
                failed = Some(classify_render_error(e));
                break;
            }
        };
        let avail = buffer_frames.saturating_sub(padding);
        if avail == 0 {
            continue;
        }
        let ptr_data = match unsafe { session.render.GetBuffer(avail) } {
            Ok(p) => p,
            Err(e) => {
                failed = Some(classify_render_error(e));
                break;
            }
        };
        let samples = avail as usize * channels;
        let vol = f32::from_bits(volume.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let mic_vol = f32::from_bits(mic_volume.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let sfx_vol = f32::from_bits(sfx_volume.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        if tmp.len() < samples { tmp.resize(samples, 0.0); }
        tmp[..samples].fill(0.0);
        for process_ring in &process_rings {
            let n = process_ring.pop(&mut mic_tmp[..samples]);
            mic_tmp[n..samples].fill(0.0);
            for i in 0..samples { tmp[i] += mic_tmp[i] * vol; }
        }
        if let Some(mic) = &mic_ring {
            if mic_tmp.len() < samples {
                mic_tmp.resize(samples, 0.0);
            }
            let mn = mic.pop(&mut mic_tmp[..samples]);
            mic_tmp[mn..samples].fill(0.0);
            for i in 0..samples {
                tmp[i] = tmp[i] + mic_tmp[i] * mic_vol;
            }
        }
        if let Some(sfx) = &sfx_ring {
            if sfx_tmp.len() < samples {
                sfx_tmp.resize(samples, 0.0);
            }
            let sn = sfx.pop(&mut sfx_tmp[..samples]);
            sfx_tmp[sn..samples].fill(0.0);
            for i in 0..samples {
                tmp[i] = tmp[i] + sfx_tmp[i] * sfx_vol;
            }
        }
        for sample in &mut tmp[..samples] {
            *sample = sample.clamp(-1.0, 1.0);
        }
        let master_vol = f32::from_bits(master.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        if (master_vol - 1.0).abs() > f32::EPSILON {
            for s in &mut tmp[..samples] {
                *s = (*s * master_vol).clamp(-1.0, 1.0);
            }
        }
        super::peak::hold_peak(&peak, &tmp[..samples]);
        if !ptr_data.is_null() {
            let nbytes = samples * session.format.kind.bytes();
            let dst = unsafe { std::slice::from_raw_parts_mut(ptr_data, nbytes) };
            if session.format.kind == SampleKind::F32 {
                for (i, sample) in tmp[..samples].iter().enumerate() {
                    let bytes = sample.to_le_bytes();
                    let o = i * 4;
                    dst[o..o + 4].copy_from_slice(&bytes);
                }
            } else {
                encode_from_f32(&tmp[..samples], session.format.kind, dst);
            }
        }
        if let Err(e) = unsafe { session.render.ReleaseBuffer(avail, 0) } {
            failed = Some(classify_render_error(e));
            break;
        }
    }

    let _ = unsafe { session.client.Stop() };
    if let Some(e) = failed
        && !stop.load(Ordering::Acquire)
    {
        on_fail(e);
    }
}

/// 设备拔出映射为 DeviceNotFound，其余保留 COM 细节给 tracing。
fn classify_render_error(e: windows::core::Error) -> Error {
    let code = e.code().0 as u32;
    if code == 0x8889_0004 || code == 0x8889_0013 {
        Error::DeviceNotFound
    } else {
        e.into()
    }
}
