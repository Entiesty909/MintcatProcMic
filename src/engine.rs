//! 音频引擎：常驻输出 + 可动态启停的进程环回、物理麦与音效源。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::audio::buffer::SpscRing;
use crate::audio::capture::{open_mic_capture, open_process_capture, run_capture_loop};
use crate::audio::com;
use crate::audio::format::{AudioFormat, Converter, SampleKind};
use crate::audio::render::{open_render_device, run_render_loop};
use crate::error::Error;

/// 引擎对外状态，UI 定时轮询。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineStatus {
    /// 没有输出线程。
    Idle,
    /// 输出线程正在写设备。
    Running,
    /// 进程源会话结束，输出仍可继续服务其它源。
    ProcessExited,
    /// 渲染设备失效。
    DeviceGone,
    /// 其它可读错误。
    Error(String),
}

/// 输出线程拥有的固定三条源 ring。
struct OutputState {
    /// 请求停止输出线程。
    stop: Arc<AtomicBool>,
    /// 进程环回源。
    process_ring: Arc<SpscRing>,
    /// 物理麦源。
    mic_ring: Arc<SpscRing>,
    /// 音效/文件源；S2 先由静音 ring 占位，S4/S5 接入生产者。
    sfx_ring: Arc<SpscRing>,
    /// 输出设备的 mix 格式，给后续动态源建 Converter。
    render_format: AudioFormat,
    /// 原始 mix 格式，保留完整 cbSize 语义给 WASAPI 捕获初始化。
    mix_format: windows::Win32::Media::Audio::WAVEFORMATEX,
    /// 输出线程句柄。
    render: JoinHandle<()>,
}

/// 动态源线程。每个 ring 始终只有一个生产者。
#[derive(Default)]
struct SourceThreads {
    /// 进程源停止令牌。
    process_stop: Option<Arc<AtomicBool>>,
    /// 进程源线程。
    process: Option<JoinHandle<()>>,
    /// 麦源停止令牌。
    mic_stop: Option<Arc<AtomicBool>>,
    /// 麦源线程。
    mic: Option<JoinHandle<()>>,
}

/// 可跨 UI 回调共享的引擎。
pub struct AudioEngine {
    /// 进程音量。
    volume: Arc<AtomicU32>,
    /// 物理麦音量。
    mic_volume: Arc<AtomicU32>,
    /// 音效/文件总线音量。
    sfx_volume: Arc<AtomicU32>,
    /// 总音量：写进设备前的最终增益。
    master: Arc<AtomicU32>,
    /// 进程捕获峰值。
    capture_peak: Arc<AtomicU32>,
    /// 麦克风峰值。
    mic_peak: Arc<AtomicU32>,
    /// 渲染端峰值。
    render_peak: Arc<AtomicU32>,
    /// 是否有输出线程。
    running: Arc<AtomicBool>,
    /// 给 UI 的状态。
    status: Arc<Mutex<EngineStatus>>,
    /// 输出线程与三条 ring。
    output: Mutex<Option<OutputState>>,
    /// 动态源线程。
    sources: Mutex<SourceThreads>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    /// 空闲引擎，所有总线音量为 1.0。
    pub fn new() -> Self {
        Self {
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            mic_volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            sfx_volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            master: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            capture_peak: Arc::new(AtomicU32::new(0)),
            mic_peak: Arc::new(AtomicU32::new(0)),
            render_peak: Arc::new(AtomicU32::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(EngineStatus::Idle)),
            output: Mutex::new(None),
            sources: Mutex::new(SourceThreads::default()),
        }
    }

    /// 是否已打开输出线程。
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// 当前状态副本。
    pub fn status(&self) -> EngineStatus {
        self.status
            .lock()
            .map(|g| g.clone())
            .unwrap_or(EngineStatus::Error("status lock".into()))
    }

    /// 设置进程音量 0..=1。
    pub fn set_volume(&self, volume: f32) {
        self.volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// 设置物理麦音量 0..=1。
    pub fn set_mic_volume(&self, volume: f32) {
        self.mic_volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// 设置音效/文件总线音量 0..=1。
    pub fn set_sfx_volume(&self, volume: f32) {
        self.sfx_volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// 设置总音量 0..=1。
    pub fn set_master_volume(&self, volume: f32) {
        self.master
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// 进程捕获峰值并衰减。
    pub fn sample_capture_peak(&self) -> f32 {
        crate::audio::peak::sample_and_decay(&self.capture_peak, 0.62)
    }

    /// 麦克风峰值并衰减。
    pub fn sample_mic_peak(&self) -> f32 {
        crate::audio::peak::sample_and_decay(&self.mic_peak, 0.62)
    }

    /// 渲染端峰值并衰减。
    pub fn sample_render_peak(&self) -> f32 {
        crate::audio::peak::sample_and_decay(&self.render_peak, 0.62)
    }

    /// 打开一个只负责写设备的输出线程。源可以随后动态挂载。
    pub fn open_output(&self, device_id: &str) -> Result<(), Error> {
        self.stop();
        let _com = com::init_mta()?;
        let render = open_render_device(device_id).map_err(|e| {
            tracing::warn!("render open failed: {e}");
            Error::RenderInit("open device")
        })?;
        let render_format = render.format;
        let mix_format = copy_wave(render.mix_format());
        let ring_samples = (render_format.sample_rate as usize)
            .saturating_mul(render_format.channels as usize)
            / 4
            * 2;
        let capacity = ring_samples.max(8192);
        let process_ring = Arc::new(SpscRing::with_capacity_samples(capacity));
        let mic_ring = Arc::new(SpscRing::with_capacity_samples(capacity));
        let sfx_ring = Arc::new(SpscRing::with_capacity_samples(capacity));
        let stop = Arc::new(AtomicBool::new(false));

        self.capture_peak.store(0, Ordering::Relaxed);
        self.capture_peak.store(0, Ordering::Relaxed);
        let stop_r = stop.clone();
        let process_ring_render = process_ring.clone();
        let mic_ring_render = mic_ring.clone();
        let sfx_ring_render = sfx_ring.clone();
        let volume = self.volume.clone();
        let mic_volume = self.mic_volume.clone();
        let sfx_volume = self.sfx_volume.clone();
        let master = self.master.clone();
        let peak = self.render_peak.clone();
        let status = self.status.clone();
        let running = self.running.clone();
        let render_thread = thread::Builder::new()
            .name("wasapi-render".into())
            .spawn(move || {
                let _com = com::init_mta();
                run_render_loop(
                    render,
                    process_ring_render,
                    Some(mic_ring_render),
                    Some(sfx_ring_render),
                    volume,
                    mic_volume,
                    sfx_volume,
                    master,
                    stop_r,
                    peak,
                    |err| {
                        tracing::warn!("render stopped: {err}");
                        let mapped = match err {
                            Error::DeviceNotFound => EngineStatus::DeviceGone,
                            other => EngineStatus::Error(other.user_message()),
                        };
                        if let Ok(mut g) = status.lock() {
                            *g = mapped;
                        }
                        running.store(false, Ordering::Release);
                    },
                );
            })
            .map_err(|_| Error::RenderInit("spawn render thread"))?;

        if let Ok(mut output) = self.output.lock() {
            *output = Some(OutputState {
                stop,
                process_ring,
                mic_ring,
                sfx_ring,
                render_format,
                mix_format,
                render: render_thread,
            });
        }
        self.set_status(EngineStatus::Running);
        self.running.store(true, Ordering::Release);
        Ok(())
    }

    /// 挂载或替换进程环回源。要求输出已经打开。
    pub fn set_process_source(&self, pid: u32) -> Result<(), Error> {
        let _com = com::init_mta()?;
        self.stop_process_source();
        let (ring, _mic_ring, render_format, mix_format) = self.output_routing()?;
        let capture = open_process_capture(pid, Some(&mix_format))?;
        let converter = Converter::new(
            capture.format,
            AudioFormat {
                sample_rate: render_format.sample_rate,
                channels: render_format.channels,
                kind: SampleKind::F32,
            },
        );
        let stop = Arc::new(AtomicBool::new(false));
        let status = self.status.clone();
        let peak = self.capture_peak.clone();
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("wasapi-capture".into())
            .spawn(move || {
                let _com = com::init_mta();
                run_capture_loop(capture, ring, converter, thread_stop, peak, |err| {
                    tracing::warn!("capture stopped: {err}");
                    let mapped = match err {
                        Error::ProcessNotFound(_) => EngineStatus::ProcessExited,
                        Error::DeviceNotFound => EngineStatus::DeviceGone,
                        other => EngineStatus::Error(other.user_message()),
                    };
                    if let Ok(mut g) = status.lock() {
                        *g = mapped;
                    }
                });
            })
            .map_err(|_| Error::CaptureInit("spawn capture thread"))?;
        if let Ok(mut sources) = self.sources.lock() {
            sources.process_stop = Some(stop);
            sources.process = Some(thread);
        }
        self.set_status(EngineStatus::Running);
        Ok(())
    }

    pub fn set_mic_source(&self, mic_id: Option<&str>) -> Result<(), Error> {
        let _com = com::init_mta()?;
        self.stop_mic_source();
        let Some(mic_id) = mic_id else {
            return Ok(());
        };
        let (_process_ring, ring, render_format, mix_format) = self.output_routing()?;
        let mic = open_mic_capture(mic_id, Some(&mix_format))?;
        let converter = Converter::new(
            mic.format,
            AudioFormat {
                sample_rate: render_format.sample_rate,
                channels: render_format.channels,
                kind: SampleKind::F32,
            },
        );
        let stop = Arc::new(AtomicBool::new(false));
        let peak = self.mic_peak.clone();
        let thread_stop = stop.clone();
        let thread = thread::Builder::new()
            .name("wasapi-mic".into())
            .spawn(move || {
                let _com = com::init_mta();
                run_capture_loop(mic, ring, converter, thread_stop, peak, |err| {
                    tracing::warn!("mic capture stopped: {err}");
                });
            })
            .map_err(|_| Error::CaptureInit("spawn mic thread"))?;
        if let Ok(mut sources) = self.sources.lock() {
            sources.mic_stop = Some(stop);
            sources.mic = Some(thread);
        }
        Ok(())
    }

    /// 兼容现有 UI/CLI 的一次性启动入口。
    pub fn start(&self, pid: u32, device_id: &str, mic_id: Option<&str>) -> Result<(), Error> {
        self.open_output(device_id)?;
        if let Err(e) = self
            .set_process_source(pid)
            .and_then(|_| self.set_mic_source(mic_id))
        {
            self.stop();
            return Err(e);
        }
        Ok(())
    }

    /// 关闭全部源与输出线程。
    pub fn stop(&self) {
        self.stop_process_source();
        self.stop_mic_source();
        let output = self.output.lock().ok().and_then(|mut g| g.take());
        if let Some(output) = output {
            output.stop.store(true, Ordering::Release);
            let _ = output.render.join();
        }
        self.running.store(false, Ordering::Release);
        self.capture_peak.store(0, Ordering::Relaxed);
        self.mic_peak.store(0, Ordering::Relaxed);
        self.render_peak.store(0, Ordering::Relaxed);
        if matches!(self.status(), EngineStatus::Running) {
            self.set_status(EngineStatus::Idle);
        }
    }

    fn output_routing(
        &self,
    ) -> Result<(
        Arc<SpscRing>,
        Arc<SpscRing>,
        AudioFormat,
        windows::Win32::Media::Audio::WAVEFORMATEX,
    ), Error> {
        let output = self
            .output
            .lock()
            .map_err(|_| Error::RenderInit("output lock poisoned"))?;
        let output = output
            .as_ref()
            .ok_or(Error::RenderInit("output is closed"))?;
        Ok((
            output.process_ring.clone(),
            output.mic_ring.clone(),
            output.render_format,
            copy_wave(&output.mix_format),
        ))
    }

    fn stop_process_source(&self) {
        let (stop, thread) = self.sources.lock().ok().map(|mut s| (s.process_stop.take(), s.process.take())).unwrap_or((None, None));
        if let Some(stop) = stop { stop.store(true, Ordering::Release); }
        if let Some(thread) = thread { let _ = thread.join(); }
    }
    fn stop_mic_source(&self) {
        let (stop, thread) = self.sources.lock().ok().map(|mut s| (s.mic_stop.take(), s.mic.take())).unwrap_or((None, None));
        if let Some(stop) = stop { stop.store(true, Ordering::Release); }
        if let Some(thread) = thread { let _ = thread.join(); }
    }

    fn set_status(&self, status: EngineStatus) {
        if let Ok(mut g) = self.status.lock() {
            *g = status;
        }
    }

}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 浅拷贝 WAVEFORMATEX（不含 cbSize 扩展块）。
fn copy_wave(
    src: &windows::Win32::Media::Audio::WAVEFORMATEX,
) -> windows::Win32::Media::Audio::WAVEFORMATEX {
    *src
}
