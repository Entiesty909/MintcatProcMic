//! WAV 读写：开发验证写出与声板 PCM 解码。

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::Error;

/// 已解码的交错 f32 WAV。
#[derive(Clone, Debug)]
pub struct WavData {
    /// 采样率。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
    /// 交错 f32 样本。
    pub samples: Vec<f32>,
}

/// 只读取 WAV 头部，导入前验证格式并计算时长，不加载长音频样本。
pub fn probe_wav(path: &Path) -> Result<(u32, u16, u64), Error> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(Error::Format("not a RIFF/WAVE file"));
    }
    let mut format = None;
    let mut data_bytes = None;
    loop {
        let mut chunk = [0u8; 8];
        if file.read_exact(&mut chunk).is_err() {
            break;
        }
        let size = u32::from_le_bytes(chunk[4..8].try_into().unwrap_or([0; 4])) as u64;
        if &chunk[0..4] == b"fmt " {
            let mut body = vec![0u8; size.min(40) as usize];
            file.read_exact(&mut body)?;
            if body.len() >= 16 {
                format = Some((
                    u16::from_le_bytes([body[0], body[1]]),
                    u16::from_le_bytes([body[2], body[3]]),
                    u32::from_le_bytes(body[4..8].try_into().unwrap_or([0; 4])),
                    u16::from_le_bytes(body[14..16].try_into().unwrap_or([0; 2])),
                ));
            }
            if size > body.len() as u64 {
                file.seek(SeekFrom::Current((size - body.len() as u64) as i64))?;
            }
        } else {
            if &chunk[0..4] == b"data" {
                data_bytes = Some(size);
            }
            file.seek(SeekFrom::Current(size as i64))?;
        }
        if size & 1 != 0 {
            file.seek(SeekFrom::Current(1))?;
        }
        if format.is_some() && data_bytes.is_some() {
            break;
        }
    }
    let Some((tag, channels, rate, bits)) = format else {
        return Err(Error::Format("WAV fmt missing"));
    };
    let Some(data_bytes) = data_bytes else {
        return Err(Error::Format("WAV data missing"));
    };
    if channels == 0
        || rate == 0
        || !((tag == 1 && matches!(bits, 8 | 16 | 24 | 32)) || (tag == 3 && bits == 32))
    {
        return Err(Error::Format("unsupported WAV format"));
    }
    let frame_bytes = u64::from(channels) * u64::from(bits / 8);
    if frame_bytes == 0 || data_bytes < frame_bytes {
        return Err(Error::Format("empty WAV"));
    }
    let duration_hns = data_bytes / frame_bytes * 10_000_000 / u64::from(rate);
    Ok((rate, channels, duration_hns))
}

/// 读取 PCM/IEEE float WAV。只接受 RIFF/WAVE，未知 chunk 会跳过。
pub fn read_wav(path: &Path) -> Result<WavData, Error> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Error::Format("not a RIFF/WAVE file"));
    }
    let mut pos = 12usize;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size =
            u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap_or([0; 4])) as usize;
        pos += 8;
        let end = pos.saturating_add(size).min(bytes.len());
        if id == b"fmt " && end >= pos + 16 {
            let tag = u16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap_or([0; 2]));
            let channels = u16::from_le_bytes(bytes[pos + 2..pos + 4].try_into().unwrap_or([0; 2]));
            let rate = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap_or([0; 4]));
            let bits = u16::from_le_bytes(bytes[pos + 14..pos + 16].try_into().unwrap_or([0; 2]));

            format = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            data = Some(&bytes[pos..end]);
        }
        pos = end + (size & 1);
    }
    let (tag, channels, sample_rate, bits) = format.ok_or(Error::Format("WAV fmt missing"))?;
    if channels == 0 || sample_rate == 0 || !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(Error::Format("unsupported WAV format"));
    }
    let data = data.ok_or(Error::Format("WAV data missing"))?;
    let bytes_per_sample = usize::from(bits / 8);
    let frame_bytes = bytes_per_sample * usize::from(channels);
    if frame_bytes == 0 || data.len() < frame_bytes {
        return Err(Error::Format("empty WAV"));
    }
    let mut samples = Vec::with_capacity(data.len() / bytes_per_sample);
    for chunk in data.chunks_exact(bytes_per_sample) {
        let value = if tag == 3 && bits == 32 {
            f32::from_le_bytes(chunk.try_into().unwrap_or([0; 4]))
        } else if tag == 1 {
            match bits {
                8 => (f32::from(chunk[0]) - 128.0) / 128.0,
                16 => f32::from(i16::from_le_bytes(chunk.try_into().unwrap_or([0; 2]))) / 32768.0,
                24 => {
                    let sign = if chunk[2] & 0x80 != 0 { 0xFF } else { 0 };
                    let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], sign]);
                    value as f32 / 8_388_608.0
                }
                32 => {
                    i32::from_le_bytes(chunk.try_into().unwrap_or([0; 4])) as f32 / 2_147_483_648.0
                }
                _ => return Err(Error::Format("unsupported WAV bits")),
            }
        } else {
            return Err(Error::Format("unsupported WAV encoding"));
        };
        samples.push(value.clamp(-1.0, 1.0));
    }
    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}

/// 把交错 f32 量化成 16-bit PCM WAV。
pub fn write_pcm16_wav(
    path: &Path,
    sample_rate: u32,
    channels: u16,
    samples_f32: &[f32],
) -> Result<(), Error> {
    let mut pcm = Vec::with_capacity(samples_f32.len() * 2);
    for &s in samples_f32 {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    write_pcm16_bytes(path, sample_rate, channels, &pcm)
}

/// 写标准 44 字节头 + PCM16 数据。
pub fn write_pcm16_bytes(
    path: &Path,
    sample_rate: u32,
    channels: u16,
    pcm: &[u8],
) -> Result<(), Error> {
    let mut file = BufWriter::new(File::create(path)?);
    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let block_align = channels * 2;
    let riff_size = 36 + data_len;
    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&channels.to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;
    file.write_all(pcm)?;
    file.flush()?;
    Ok(())
}
