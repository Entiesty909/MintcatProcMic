//! 开发验证用 PCM16 WAV 写出。不走热路径。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::error::Error;

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
    file.write_all(&1u16.to_le_bytes())?; // PCM
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
