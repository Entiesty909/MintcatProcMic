//! 音频线程写峰值，UI 线程读并衰减。热路径不分配。

use std::sync::atomic::{AtomicU32, Ordering};

/// 用本包绝对值峰值抬升 `slot`（只升不降）。
pub fn hold_peak(slot: &AtomicU32, samples: &[f32]) {
    let mut peak = 0.0f32;
    for &s in samples {
        let a = s.abs();
        if a > peak {
            peak = a;
        }
    }
    if peak <= f32::EPSILON {
        return;
    }
    let peak = peak.min(1.0);
    let old = f32::from_bits(slot.load(Ordering::Relaxed));
    if peak > old {
        slot.store(peak.to_bits(), Ordering::Relaxed);
    }
}

/// 读当前峰值并衰减，供 UI 声纹采样。
pub fn sample_and_decay(slot: &AtomicU32, decay: f32) -> f32 {
    let v = f32::from_bits(slot.load(Ordering::Relaxed)).clamp(0.0, 1.0);
    slot.store((v * decay).to_bits(), Ordering::Relaxed);
    v
}
