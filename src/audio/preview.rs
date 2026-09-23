//! 独立本地预览输出：不进入虚拟麦克风，只写系统播放设备。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::audio::buffer::SpscRing;
use crate::audio::com;
use crate::audio::device;
use crate::audio::format::AudioFormat;
use crate::audio::render::{open_render_device, run_render_loop};
use crate::audio::wav::WavData;
use crate::error::Error;

/// 独立本机试听输出。
pub struct PreviewEngine {
    ring: Arc<SpscRing>,
    target: AudioFormat,
    stop_render: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    source: Mutex<Option<SourceThread>>,
    render: Mutex<Option<JoinHandle<()>>>,
}

struct SourceThread {
    stop: Arc<AtomicBool>,
    join: JoinHandle<()>,
}

impl PreviewEngine {
    /// 打开指定播放设备。
    pub fn open(device_id: &str) -> Result<Self, Error> {
        let render = open_render_device(device_id)?;
        let target = render.format;
        let capacity =
            ((target.sample_rate as usize).saturating_mul(target.channels as usize) / 2).max(8192);
        let ring = Arc::new(SpscRing::with_capacity_samples(capacity));
        let stop_render = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU32::new(0.5f32.to_bits()));
        let render_thread = {
            let ring = ring.clone();
            let stop = stop_render.clone();
            let volume = volume.clone();
            let peak = Arc::new(AtomicU32::new(0));
            thread::Builder::new()
                .name("preview-render".into())
                .spawn(move || {
                    let _com = com::init_mta();
                    run_render_loop(
                        render,
                        vec![ring],
                        None,
                        None,
                        volume,
                        Arc::new(AtomicU32::new(1.0f32.to_bits())),
                        Arc::new(AtomicU32::new(1.0f32.to_bits())),
                        Arc::new(AtomicU32::new(1.0f32.to_bits())),
                        stop,
                        peak,
                        |_| {},
                    );
                })
                .map_err(|_| Error::RenderInit("spawn preview render"))?
        };
        Ok(Self {
            ring,
            target,
            stop_render,
            volume,
            source: Mutex::new(None),
            render: Mutex::new(Some(render_thread)),
        })
    }

    /// 打开系统默认播放设备试听。
    pub fn open_default() -> Result<Self, Error> {
        Self::open(&device::default_render_id()?)
    }
    /// 设置本地试听音量 0..=1。
    pub fn set_volume(&self, value: f32) {
        self.volume
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// 异步播放已解码音频，避免阻塞 UI。
    pub fn play(&self, data: Arc<WavData>, looped: bool) {
        self.stop_source();
        self.ring.clear();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let ring = self.ring.clone();
        let target = self.target;
        let join = thread::Builder::new()
            .name("preview-source".into())
            .spawn(move || {
                let src_channels = usize::from(data.channels.max(1));
                let dst_channels = usize::from(target.channels.max(1));
                let src_frames = data.samples.len() / src_channels;
                let step = f64::from(data.sample_rate) / f64::from(target.sample_rate);
                let chunk_frames = (target.sample_rate as usize / 20).max(64);
                let mut pos = 0.0f64;
                let mut out = Vec::with_capacity(chunk_frames * dst_channels);
                let chunk_time = Duration::from_millis(50);
                loop {
                    if stop_thread.load(Ordering::Acquire) {
                        break;
                    }
                    out.clear();
                    for _ in 0..chunk_frames {
                        let frame = pos.floor() as usize;
                        if frame >= src_frames {
                            if looped {
                                pos = 0.0;
                                continue;
                            }
                            break;
                        }
                        for ch in 0..dst_channels {
                            out.push(data.samples[frame * src_channels + ch.min(src_channels - 1)]);
                        }
                        pos += step;
                    }
                    if out.is_empty() {
                        break;
                    }
                    ring.push_latest(&out);
                    thread::sleep(chunk_time);
                }
            })
            .ok();
        if let Some(join) = join {
            if let Ok(mut slot) = self.source.lock() {
                *slot = Some(SourceThread { stop, join });
            }
        }
    }

    /// 停止当前试听源。
    pub fn stop_source(&self) {
        let source = self.source.lock().ok().and_then(|mut slot| slot.take());
        if let Some(source) = source {
            source.stop.store(true, Ordering::Release);
            let _ = source.join.join();
        }
    }
}

impl Drop for PreviewEngine {
    fn drop(&mut self) {
        self.stop_source();
        self.stop_render.store(true, Ordering::Release);
        if let Ok(mut render) = self.render.lock() {
            if let Some(join) = render.take() {
                let _ = join.join();
            }
        }
    }
}
