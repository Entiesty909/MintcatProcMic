//! 音频引擎：进程环回 + 可选物理麦，混音后送到同一渲染设备。

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
    /// 未转发。
    Idle,
    /// 线程在跑。
    Running,
    /// 捕获会话随进程结束。
    ProcessExited,
    /// 渲染设备失效。
    DeviceGone,
    /// 其它可读错误。
    Error(String),
}

/// 正在跑的音频线程句柄。
struct Threads {
    /// 进程环回捕获。
    capture: JoinHandle<()>,
    /// 物理麦捕获（可选）。
    mic: Option<JoinHandle<()>>,
    /// 渲染。
    render: JoinHandle<()>,
}

/// 可跨 UI 回调共享的引擎。
pub struct AudioEngine {
    /// 请求停止。
    stop: Arc<AtomicBool>,
    /// 进程音量。
    volume: Arc<AtomicU32>,
    /// 物理麦音量。
    mic_volume: Arc<AtomicU32>,
    /// 总音量：写进设备前的最终增益，对所有源统一生效。
    master: Arc<AtomicU32>,
    /// 进程捕获峰值。
    capture_peak: Arc<AtomicU32>,
    /// 麦克风峰值。
    mic_peak: Arc<AtomicU32>,
    /// 渲染端峰值。
    render_peak: Arc<AtomicU32>,
    /// 是否认为在跑。
    running: Arc<AtomicBool>,
    /// 给 UI 的状态。
    status: Arc<Mutex<EngineStatus>>,
    /// join 用。
    threads: Mutex<Option<Threads>>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioEngine {
    /// 空闲引擎，音量 1.0。
    pub fn new() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            mic_volume: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            master: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            capture_peak: Arc::new(AtomicU32::new(0)),
            mic_peak: Arc::new(AtomicU32::new(0)),
            render_peak: Arc::new(AtomicU32::new(0)),
            running: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(EngineStatus::Idle)),
            threads: Mutex::new(None),
        }
    }

    /// 是否已启动且线程未报停。
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

    /// 渲染峰值并衰减。
    pub fn sample_render_peak(&self) -> f32 {
        crate::audio::peak::sample_and_decay(&self.render_peak, 0.62)
    }

    /// 写状态；锁中毒则忽略。
    fn set_status(&self, status: EngineStatus) {
        if let Ok(mut g) = self.status.lock() {
            *g = status;
        }
    }

    /// 进程环回 + 可选物理麦，混音后送到 render 设备。
    pub fn start(
        &self,
        pid: u32,
        device_id: &str,
        mic_id: Option<&str>,
    ) -> Result<(), Error> {
        if self.is_running() {
            self.stop();
        }

        let _com = com::init_mta()?;
        let render = open_render_device(device_id).map_err(|e| {
            tracing::warn!("render open failed: {e}");
            Error::RenderInit("open device")
        })?;
        let render_fmt = render.format;
        let mix_fmt_copy = copy_wave(render.mix_format());

        let capture = open_process_capture(pid, Some(&mix_fmt_copy)).map_err(|e| {
            tracing::warn!("loopback capture failed: {e}");
            e
        })?;

        let mic_client = if let Some(id) = mic_id {
            Some(open_mic_capture(id, Some(&mix_fmt_copy)).map_err(|e| {
                tracing::warn!("mic capture failed: {e}");
                Error::CaptureInit("open microphone")
            })?)
        } else {
            None
        };

        let src = capture.format;
        let dst = AudioFormat {
            sample_rate: render_fmt.sample_rate,
            channels: render_fmt.channels,
            kind: SampleKind::F32,
        };
        let converter = Converter::new(src, dst);
        let ring_samples = (render_fmt.sample_rate as usize)
            .saturating_mul(render_fmt.channels as usize)
            / 4
            * 2;
        let ring = Arc::new(SpscRing::with_capacity_samples(ring_samples.max(8192)));
        let mic_ring = mic_client.as_ref().map(|_| {
            Arc::new(SpscRing::with_capacity_samples(ring_samples.max(8192)))
        });

        self.stop.store(false, Ordering::Release);
        self.capture_peak.store(0, Ordering::Relaxed);
        self.mic_peak.store(0, Ordering::Relaxed);
        self.render_peak.store(0, Ordering::Relaxed);
        self.set_status(EngineStatus::Running);
        self.running.store(true, Ordering::Release);

        let stop_c = self.stop.clone();
        let ring_c = ring.clone();
        let status_c = self.status.clone();
        let running_c = self.running.clone();
        let peak_c = self.capture_peak.clone();
        let capture_thread = thread::Builder::new()
            .name("wasapi-capture".into())
            .spawn(move || {
                let _com = com::init_mta();
                run_capture_loop(capture, ring_c, converter, stop_c, peak_c, |err| {
                    tracing::warn!("capture stopped: {err}");
                    let mapped = match err {
                        Error::ProcessNotFound(_) => EngineStatus::ProcessExited,
                        Error::DeviceNotFound => EngineStatus::DeviceGone,
                        other => EngineStatus::Error(other.user_message()),
                    };
                    if let Ok(mut g) = status_c.lock() {
                        *g = mapped;
                    }
                    running_c.store(false, Ordering::Release);
                });
            })
            .map_err(|_| Error::CaptureInit("spawn capture thread"))?;

        let mic_thread = if let (Some(mic), Some(mring)) = (mic_client, mic_ring.clone()) {
            let mic_fmt = mic.format;
            let mic_conv = Converter::new(
                mic_fmt,
                AudioFormat {
                    sample_rate: render_fmt.sample_rate,
                    channels: render_fmt.channels,
                    kind: SampleKind::F32,
                },
            );
            let stop_m = self.stop.clone();
            let peak_m = self.mic_peak.clone();
            Some(
                thread::Builder::new()
                    .name("wasapi-mic".into())
                    .spawn(move || {
                        let _com = com::init_mta();
                        run_capture_loop(mic, mring, mic_conv, stop_m, peak_m, |err| {
                            tracing::warn!("mic capture stopped: {err}");
                        });
                    })
                    .map_err(|_| Error::CaptureInit("spawn mic thread"))?,
            )
        } else {
            None
        };

        let stop_r = self.stop.clone();
        let vol_r = self.volume.clone();
        let mic_vol_r = self.mic_volume.clone();
        let master_r = self.master.clone();
        let status_r = self.status.clone();
        let running_r = self.running.clone();
        let peak_r = self.render_peak.clone();
        let render_thread = thread::Builder::new()
            .name("wasapi-render".into())
            .spawn(move || {
                let _com = com::init_mta();
                run_render_loop(
                    render,
                    ring,
                    mic_ring,
                    vol_r,
                    mic_vol_r,
                    master_r,
                    stop_r,
                    peak_r,
                    |err| {
                        tracing::warn!("render stopped: {err}");
                        let mapped = match err {
                            Error::DeviceNotFound => EngineStatus::DeviceGone,
                            other => EngineStatus::Error(other.user_message()),
                        };
                        if let Ok(mut g) = status_r.lock() {
                            *g = mapped;
                        }
                        running_r.store(false, Ordering::Release);
                    },
                );
            })
            .map_err(|_| Error::RenderInit("spawn render thread"))?;

        if let Ok(mut g) = self.threads.lock() {
            *g = Some(Threads {
                capture: capture_thread,
                mic: mic_thread,
                render: render_thread,
            });
        }
        Ok(())
    }

    /// 置 stop、join 全部音频线程。
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(mut g) = self.threads.lock()
            && let Some(threads) = g.take()
        {
            let _ = threads.capture.join();
            if let Some(mic) = threads.mic {
                let _ = mic.join();
            }
            let _ = threads.render.join();
        }
        self.running.store(false, Ordering::Release);
        self.capture_peak.store(0, Ordering::Relaxed);
        self.mic_peak.store(0, Ordering::Relaxed);
        self.render_peak.store(0, Ordering::Relaxed);
        if matches!(self.status(), EngineStatus::Running) {
            self.set_status(EngineStatus::Idle);
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
