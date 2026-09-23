//! Media Foundation 文件音频读取：mp3 / mp4 / m4a / flac / wma，视频轨只忽略不解码。

use std::path::Path;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::core::{GUID, PCWSTR};

use super::com;
use super::format::{AudioFormat, SampleKind};
use super::wav::{self, WavData};
use crate::error::Error;

static MF_INIT: Once = Once::new();

/// 解码后的媒体元数据。
#[derive(Clone, Copy, Debug)]
pub struct MediaInfo {
    /// 100 ns 单位的时长。
    pub duration_hns: u64,
    /// 原始采样率。
    pub sample_rate: u32,
    /// 原始声道数。
    pub channels: u16,
}

/// 统一文件读取器。WAV 自解码，其余走 MF。
pub enum AudioReader {
    /// 已完整读入内存的 WAV。
    Wav { data: WavData, position: usize },
    /// Media Foundation 流式读取器。
    Mf(MfReader),
}

/// Media Foundation 状态。
pub struct MfReader {
    reader: IMFSourceReader,
    info: MediaInfo,
    format: AudioFormat,
}

/// 统一打开入口。
pub fn open(path: &Path) -> Result<AudioReader, Error> {
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("wav"))
    {
        return Ok(AudioReader::Wav {
            data: wav::read_wav(path)?,
            position: 0,
        });
    }
    Ok(AudioReader::Mf(MfReader::open(path)?))
}

/// 导入前验证扩展名与音频轨，避免无效条目进入音频库。
pub fn validate_file(path: &Path) -> Result<MediaInfo, Error> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "wav" | "mp3" | "mp4" | "m4a" | "flac" | "wma") {
        return Err(Error::Format("unsupported audio extension"));
    }
    if ext == "wav" {
        let (sample_rate, channels) = wav::probe_wav(path)?;
        return Ok(MediaInfo {
            duration_hns: 0,
            sample_rate,
            channels,
        });
    }
    let reader = open(path)?;
    let info = reader.info();
    if info.sample_rate == 0 || info.channels == 0 {
        return Err(Error::Format("unsupported audio format"));
    }
    Ok(info)
}

impl AudioReader {
    /// 媒体元数据。
    pub fn info(&self) -> MediaInfo {
        match self {
            Self::Wav { data, .. } => MediaInfo {
                duration_hns: if data.sample_rate == 0 {
                    0
                } else {
                    (data.samples.len() as u64 / u64::from(data.channels.max(1))) * 10_000_000
                        / u64::from(data.sample_rate)
                },
                sample_rate: data.sample_rate,
                channels: data.channels,
            },
            Self::Mf(reader) => reader.info,
        }
    }

    /// 从 100 ns 位置 seek。
    pub fn seek_hns(&mut self, hns: u64) -> Result<(), Error> {
        match self {
            Self::Wav { data, position } => {
                let frames = hns.saturating_mul(u64::from(data.sample_rate)) / 10_000_000;
                *position = (frames as usize)
                    .saturating_mul(usize::from(data.channels))
                    .min(data.samples.len());
                Ok(())
            }
            Self::Mf(reader) => reader.seek_hns(hns),
        }
    }
    /// 读取完整文件；每个块读取前检查取消标志，避免切换长音频继续占用 CPU/内存。
    pub fn read_all_cancellable(mut self, cancel: &AtomicBool) -> Result<Option<WavData>, Error> {
        let info = self.info();
        let mut samples = Vec::new();
        let mut block = Vec::with_capacity(16_384);
        loop {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            let n = self.read_block(&mut block, 16_384)?;
            if n == 0 {
                break;
            }
            samples.extend_from_slice(&block);
        }
        Ok(Some(WavData {
            sample_rate: info.sample_rate,
            channels: info.channels,
            samples,
        }))
    }

    /// 读取完整文件供当前 SFX 播放器使用；兼容旧调用。
    pub fn read_all(mut self) -> Result<WavData, Error> {
        let info = self.info();
        let mut samples = Vec::new();
        let mut block = Vec::with_capacity(16_384);
        loop {
            let n = self.read_block(&mut block, 16_384)?;
            if n == 0 {
                break;
            }
            samples.extend_from_slice(&block);
        }
        Ok(WavData {
            sample_rate: info.sample_rate,
            channels: info.channels,
            samples,
        })
    }

    /// 读取一块交错 f32，返回样本数；0 表示 EOF。
    pub fn read_block(&mut self, dst: &mut Vec<f32>, max_samples: usize) -> Result<usize, Error> {
        dst.clear();
        match self {
            Self::Wav { data, position } => {
                let end = position.saturating_add(max_samples).min(data.samples.len());
                dst.extend_from_slice(&data.samples[*position..end]);
                *position = end;
                Ok(dst.len())
            }
            Self::Mf(reader) => reader.read_block(dst, max_samples),
        }
    }
}

impl MfReader {
    fn open(path: &Path) -> Result<Self, Error> {
        let _com = com::init_mta()?;
        MF_INIT.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_LITE);
        });
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None) }
            .map_err(|_| Error::Format("Media Foundation cannot open file"))?;
        unsafe {
            let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            reader
                .SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32, true)
                .map_err(|_| Error::Format("audio track missing"))?;
            let media_type = MFCreateMediaType().map_err(|_| Error::Format("create media type"))?;
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .map_err(|_| Error::Format("audio type"))?;
            media_type
                .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
                .map_err(|_| Error::Format("float type"))?;
            media_type
                .SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)
                .map_err(|_| Error::Format("independent samples"))?;
            reader
                .SetCurrentMediaType(
                    MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                    None,
                    &media_type,
                )
                .map_err(|_| Error::Format("unsupported audio format"))?;
            let current = reader
                .GetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32)
                .map_err(|_| Error::Format("audio media type"))?;
            let rate = current
                .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
                .map_err(|_| Error::Format("sample rate"))?;
            let channels = current
                .GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
                .map_err(|_| Error::Format("channels"))? as u16;
            let duration = reader
                .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
                .ok()
                .and_then(propvar_u64)
                .unwrap_or(0);
            Ok(Self {
                reader,
                info: MediaInfo {
                    duration_hns: duration,
                    sample_rate: rate,
                    channels,
                },
                format: AudioFormat {
                    sample_rate: rate,
                    channels,
                    kind: SampleKind::F32,
                },
            })
        }
    }

    fn info(&self) -> MediaInfo {
        self.info
    }

    fn seek_hns(&mut self, hns: u64) -> Result<(), Error> {
        let mut value = unsafe { std::mem::zeroed::<PROPVARIANT>() };
        unsafe {
            (*value.Anonymous.Anonymous).vt = windows::Win32::System::Variant::VT_I8;
            (*value.Anonymous.Anonymous).Anonymous.lVal = hns as i32;
            self.reader
                .SetCurrentPosition(&GUID::zeroed(), &value)
                .map_err(|_| Error::Format("seek failed"))
        }
    }

    fn read_block(&mut self, dst: &mut Vec<f32>, max_samples: usize) -> Result<usize, Error> {
        let mut flags = 0u32;
        let mut sample = None;
        unsafe {
            self.reader
                .ReadSample(
                    MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32,
                    0,
                    None,
                    Some(&mut flags as *mut u32),
                    None,
                    Some(&mut sample),
                )
                .map_err(|_| Error::Format("decode failed"))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(0);
            }
            let Some(sample) = sample else {
                return Ok(0);
            };
            let buffer = sample
                .ConvertToContiguousBuffer()
                .map_err(|_| Error::Format("audio buffer"))?;
            let mut ptr = std::ptr::null_mut();
            let mut current = 0u32;
            buffer
                .Lock(&mut ptr, None, Some(&mut current))
                .map_err(|_| Error::Format("lock audio buffer"))?;
            let bytes = std::slice::from_raw_parts(ptr, current as usize);
            for chunk in bytes.chunks_exact(4).take(max_samples) {
                dst.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
            }
            let _ = buffer.Unlock();
            Ok(dst.len())
        }
    }
}

fn propvar_u64(value: PROPVARIANT) -> Option<u64> {
    unsafe { Some((*value.Anonymous.Anonymous).Anonymous.uhVal) }
}
