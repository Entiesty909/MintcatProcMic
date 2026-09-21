//! 解析 WAVEFORMATEX，捕获线程把 PCM 转成渲染端 f32。

use windows::Win32::Media::Audio::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE};
use windows::core::GUID;

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const KSDATAFORMAT_SUBTYPE_PCM: GUID =
    GUID::from_u128(0x0000_0001_0000_0010_8000_00aa_0038_9b71);
const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
    GUID::from_u128(0x0000_0003_0000_0010_8000_00aa_0038_9b71);

/// PCM 样本类型。ring 内部一律 f32。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    /// IEEE float 32-bit。
    F32,
    /// 有符号 16-bit。
    I16,
    /// 打包 24-bit（3 字节 LE）。
    I24,
    /// 有符号 32-bit PCM。
    I32,
    /// 无符号 8-bit。
    U8,
}

impl SampleKind {
    pub fn bytes(self) -> usize {
        match self {
            Self::F32 | Self::I32 => 4,
            Self::I16 => 2,
            Self::I24 => 3,
            Self::U8 => 1,
        }
    }
}

/// 交错 PCM 的逻辑格式。
#[derive(Clone, Copy, Debug)]
pub struct AudioFormat {
    /// 采样率 Hz。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
    /// 样本类型。
    pub kind: SampleKind,
}

impl AudioFormat {
    /// 一帧（所有声道）字节数。
    pub fn frame_bytes(self) -> usize {
        self.channels as usize * self.kind.bytes()
    }

    /// 从 WASAPI mix 解析。无法识别则 None。
    pub fn from_wave(fmt: &WAVEFORMATEX) -> Option<Self> {
        let kind = match fmt.wFormatTag {
            WAVE_FORMAT_IEEE_FLOAT if fmt.wBitsPerSample == 32 => SampleKind::F32,
            WAVE_FORMAT_PCM => match fmt.wBitsPerSample {
                8 => SampleKind::U8,
                16 => SampleKind::I16,
                24 => SampleKind::I24,
                32 => SampleKind::I32,
                _ => return None,
            },
            WAVE_FORMAT_EXTENSIBLE => {
                if (fmt.cbSize as usize) < size_of::<WAVEFORMATEXTENSIBLE>() - size_of::<WAVEFORMATEX>()
                {
                    return None;
                }
                let ext = unsafe { &*(fmt as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
                let sub = ext.SubFormat;
                if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT && fmt.wBitsPerSample == 32 {
                    SampleKind::F32
                } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
                    match fmt.wBitsPerSample {
                        8 => SampleKind::U8,
                        16 => SampleKind::I16,
                        24 => SampleKind::I24,
                        32 => SampleKind::I32,
                        _ => return None,
                    }
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        if fmt.nChannels == 0 || fmt.nSamplesPerSec == 0 {
            return None;
        }
        Some(Self {
            sample_rate: fmt.nSamplesPerSec,
            channels: fmt.nChannels,
            kind,
        })
    }
}

/// 捕获格式 → 渲染 f32。重采样分数和残余帧留在捕获线程。
pub struct Converter {
    /// 输入格式。
    src: AudioFormat,
    /// 输出格式（kind 应为 F32）。
    dst: AudioFormat,
    /// 尚未消耗的源 f32 帧。
    hold: Vec<f32>,
    /// 当前读指针（源帧分数位置）。
    pos: f64,
    /// decode 暂存。
    decoded: Vec<f32>,
    /// 声道混合暂存。
    mixed: Vec<f32>,
}

impl Converter {
    /// 预分配 decode/mix 缓冲，避免音频循环扩容。
    pub fn new(src: AudioFormat, dst: AudioFormat) -> Self {
        Self {
            src,
            dst,
            hold: Vec::with_capacity(64),
            pos: 0.0,
            decoded: Vec::with_capacity(8192),
            mixed: Vec::with_capacity(8192),
        }
    }

    /// 采样率、声道、类型都一致时，convert 只做 memcpy 语义。
    pub fn passthrough(&self) -> bool {
        self.src.sample_rate == self.dst.sample_rate
            && self.src.channels == self.dst.channels
            && self.src.kind == SampleKind::F32
            && self.dst.kind == SampleKind::F32
    }

    /// 把一包 PCM 转成 dst 格式的 f32。`out` 先 clear。
    pub fn convert(&mut self, bytes: &[u8], frames: usize, silent: bool, out: &mut Vec<f32>) {
        out.clear();
        let src_ch = self.src.channels as usize;
        let dst_ch = self.dst.channels as usize;
        self.decoded.clear();
        if silent {
            self.decoded.resize(frames.saturating_mul(src_ch), 0.0);
        } else {
            decode_to_f32(bytes, frames, self.src, &mut self.decoded);
        }

        self.mixed.clear();
        mix_channels(&self.decoded, src_ch, dst_ch, &mut self.mixed);

        if self.src.sample_rate == self.dst.sample_rate {
            out.extend_from_slice(&self.mixed);
            return;
        }

        self.hold.extend_from_slice(&self.mixed);
        let src_frames = self.hold.len() / dst_ch;
        if src_frames < 2 {
            return;
        }
        let step = f64::from(self.src.sample_rate) / f64::from(self.dst.sample_rate);
        while self.pos + 1.0 < src_frames as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = i * dst_ch;
            let b = (i + 1) * dst_ch;
            for ch in 0..dst_ch {
                let s = self.hold[a + ch] * (1.0 - frac) + self.hold[b + ch] * frac;
                out.push(s);
            }
            self.pos += step;
        }
        let drop_frames = self.pos.floor() as usize;
        let drop = drop_frames.min(src_frames.saturating_sub(1)) * dst_ch;
        if drop > 0 {
            self.hold.drain(..drop);
            self.pos -= drop_frames.min(src_frames.saturating_sub(1)) as f64;
        }
        if self.hold.len() > dst_ch * 48_000 {
            self.hold.clear();
            self.pos = 0.0;
        }
    }
}

fn decode_to_f32(bytes: &[u8], frames: usize, fmt: AudioFormat, out: &mut Vec<f32>) {
    let ch = fmt.channels as usize;
    let need = frames * fmt.frame_bytes();
    let bytes = if bytes.len() < need { bytes } else { &bytes[..need] };
    let frames = bytes.len() / fmt.frame_bytes();
    out.reserve(frames * ch);
    match fmt.kind {
        SampleKind::F32 => {
            for chunk in bytes.chunks_exact(4).take(frames * ch) {
                out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
        }
        SampleKind::I16 => {
            for chunk in bytes.chunks_exact(2).take(frames * ch) {
                let v = i16::from_le_bytes([chunk[0], chunk[1]]);
                out.push(f32::from(v) / 32768.0);
            }
        }
        SampleKind::I32 => {
            for chunk in bytes.chunks_exact(4).take(frames * ch) {
                let v = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                out.push(v as f32 / 2147483648.0);
            }
        }
        SampleKind::I24 => {
            for chunk in bytes.chunks_exact(3).take(frames * ch) {
                let mut b = [0u8; 4];
                b[0] = chunk[0];
                b[1] = chunk[1];
                b[2] = chunk[2];
                if chunk[2] & 0x80 != 0 {
                    b[3] = 0xFF;
                }
                let v = i32::from_le_bytes(b);
                out.push(v as f32 / 8388608.0);
            }
        }
        SampleKind::U8 => {
            for &b in bytes.iter().take(frames * ch) {
                out.push((f32::from(b) - 128.0) / 128.0);
            }
        }
    }
}

fn mix_channels(src: &[f32], src_ch: usize, dst_ch: usize, out: &mut Vec<f32>) {
    if src_ch == 0 || dst_ch == 0 {
        return;
    }
    if src_ch == dst_ch {
        out.extend_from_slice(src);
        return;
    }
    let frames = src.len() / src_ch;
    out.reserve(frames * dst_ch);
    for frame in 0..frames {
        let base = frame * src_ch;
        if src_ch == 1 {
            let s = src[base];
            for _ in 0..dst_ch {
                out.push(s);
            }
        } else if dst_ch == 1 {
            let mut sum = 0.0;
            for ch in 0..src_ch {
                sum += src[base + ch];
            }
            out.push(sum / src_ch as f32);
        } else {
            for ch in 0..dst_ch {
                if ch < src_ch {
                    out.push(src[base + ch]);
                } else {
                    out.push(0.0);
                }
            }
        }
    }
}

pub fn encode_from_f32(src: &[f32], kind: SampleKind, dst: &mut [u8]) {
    match kind {
        SampleKind::F32 => {
            for (i, sample) in src.iter().enumerate() {
                let bytes = sample.clamp(-1.0, 1.0).to_le_bytes();
                let o = i * 4;
                if o + 4 <= dst.len() {
                    dst[o..o + 4].copy_from_slice(&bytes);
                }
            }
        }
        SampleKind::I16 => {
            for (i, sample) in src.iter().enumerate() {
                let v = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                let o = i * 2;
                if o + 2 <= dst.len() {
                    dst[o..o + 2].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        SampleKind::I32 => {
            for (i, sample) in src.iter().enumerate() {
                let v = (sample.clamp(-1.0, 1.0) * 2147483647.0) as i32;
                let o = i * 4;
                if o + 4 <= dst.len() {
                    dst[o..o + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        SampleKind::U8 => {
            for (i, sample) in src.iter().enumerate() {
                if i < dst.len() {
                    dst[i] = (sample.clamp(-1.0, 1.0) * 127.0 + 128.0) as u8;
                }
            }
        }
        SampleKind::I24 => {
            for (i, sample) in src.iter().enumerate() {
                let v = (sample.clamp(-1.0, 1.0) * 8388607.0) as i32;
                let o = i * 3;
                if o + 3 <= dst.len() {
                    let b = v.to_le_bytes();
                    dst[o] = b[0];
                    dst[o + 1] = b[1];
                    dst[o + 2] = b[2];
                }
            }
        }
    }
}
