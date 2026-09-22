//! 声板混音线程：读取已经解码的 WAV，在固定 ring 中混合并输出。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::buffer::SpscRing;
use super::format::{AudioFormat, SampleKind};
use super::wav::WavData;

/// 声板播放模式。
#[derive(Clone, Copy, Debug)]
pub enum SfxMode {
    /// 播放一次。
    Once,
    /// 播放到停止前循环。
    Loop,
    /// 与一次播放相同；按住释放由上层发送 Stop。
    Hold,
    /// 再次触发前持续循环。
    Toggle,
}

/// 声板控制命令。
pub enum SfxCommand {
    /// 播放一个已解码 WAV。
    Play { data: Arc<WavData>, mode: SfxMode, volume: f32 },
    /// 停止当前声音。
    Stop,
    /// 清空所有声音。
    StopAll,
}

/// 创建声板命令通道。
pub fn channel() -> (Sender<SfxCommand>, Receiver<SfxCommand>) {
    mpsc::channel()
}

/// 声板线程：每 10 ms 生成一块，ring 空闲时写静音。
pub fn run_sfx_loop(
    ring: Arc<SpscRing>,
    render: AudioFormat,
    stop: Arc<std::sync::atomic::AtomicBool>,
    rx: Receiver<SfxCommand>,
) {
    let channels = render.channels as usize;
    let frames = (render.sample_rate as usize / 100).max(64);
    let mut players: Vec<SfxPlayer> = Vec::with_capacity(8);
    let mut block = vec![0.0f32; frames * channels];
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        while let Ok(command) = rx.try_recv() {
            match command {
                SfxCommand::Play { data, mode, volume } => {
                    if matches!(mode, SfxMode::Toggle)
                        && players.iter().any(|p| Arc::ptr_eq(&p.data, &data))
                    {
                        players.retain(|p| !Arc::ptr_eq(&p.data, &data));
                    } else {
                        players.retain(|p| !p.finished);
                        if players.len() < 8 {
                            players.push(SfxPlayer::new(data, mode, volume));
                        }
                    }
                }
                SfxCommand::Stop | SfxCommand::StopAll => players.clear(),
            }
        }
        block.fill(0.0);
        players.retain_mut(|player| {
            player.mix_into(&mut block, render);
            !player.finished
        });
        ring.push_latest(&block);
        thread::sleep(Duration::from_millis(10));
    }
}

struct SfxPlayer {
    data: Arc<WavData>,
    mode: SfxMode,
    volume: f32,
    position: f64,
    finished: bool,
}

impl SfxPlayer {
    fn new(data: Arc<WavData>, mode: SfxMode, volume: f32) -> Self {
        Self { data, mode, volume: volume.clamp(0.0, 1.0), position: 0.0, finished: false }
    }

    fn mix_into(&mut self, out: &mut [f32], render: AudioFormat) {
        if self.data.channels == 0 || self.data.sample_rate == 0 {
            self.finished = true;
            return;
        }
        let out_channels = render.channels as usize;
        let src_channels = self.data.channels as usize;
        let out_frames = out.len() / out_channels;
        let step = f64::from(self.data.sample_rate) / f64::from(render.sample_rate);
        for frame in 0..out_frames {
            let src_frame = self.position.floor() as usize;
            if src_frame >= self.data.samples.len() / src_channels {
                if matches!(self.mode, SfxMode::Loop | SfxMode::Toggle) {
                    self.position = 0.0;
                } else {
                    self.finished = true;
                    break;
                }
            }
            let src_frame = self.position.floor() as usize;
            let src = &self.data.samples[src_frame * src_channels..(src_frame + 1) * src_channels];
            for channel in 0..out_channels {
                let value = if src_channels == 1 {
                    src[0]
                } else if channel < src_channels {
                    src[channel]
                } else {
                    src[src_channels - 1]
                };
                out[frame * out_channels + channel] += value * self.volume;
            }
            self.position += step;
        }
    }
}

#[allow(dead_code)]
fn _f32_format(sample_rate: u32, channels: u16) -> AudioFormat {
    AudioFormat { sample_rate, channels, kind: SampleKind::F32 }
}
