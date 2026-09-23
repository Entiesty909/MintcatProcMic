//! Capture → Render 的 SPSC 环形缓冲。音频线程不分配。

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 单生产者单消费者 f32 交错采样环。
///
/// 容量为 2 的幂。写满时丢掉最旧数据，把延迟钉在环长度内。
pub struct SpscRing {
    /// 采样存储。SPSC + 原子头尾指针，故用 UnsafeCell 做内部可变。
    buf: UnsafeCell<Box<[f32]>>,
    /// `capacity - 1`，用于取模。
    mask: usize,
    /// 生产者写入位置（单调递增）。
    write: AtomicUsize,
    /// 消费者读取位置（单调递增）。
    read: AtomicUsize,
}

// 生产者与消费者不会同时写同一槽位；跨线程共享是设计目标。
unsafe impl Send for SpscRing {}
unsafe impl Sync for SpscRing {}

impl SpscRing {
    /// 按最少采样数分配，向上取 2 的幂，至少 4096。
    pub fn with_capacity_samples(min_samples: usize) -> Self {
        // 2 的幂才能用 mask 代替取模。
        let cap = min_samples.next_power_of_two().max(4096);
        Self {
            buf: UnsafeCell::new(vec![0.0; cap].into_boxed_slice()),
            mask: cap - 1,
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
        }
    }

    /// 环容量（采样数，含一个空槽约定下实际可用 cap-1）。
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// 当前已占用采样数。
    pub fn len(&self) -> usize {
        let w = self.write.load(Ordering::Acquire);
        let r = self.read.load(Ordering::Acquire);
        w.wrapping_sub(r)
    }
    /// 丢弃当前积压样本，切换独立播放源时使用。
    pub fn clear(&self) {
        let write = self.write.load(Ordering::Acquire);
        self.read.store(write, Ordering::Release);
    }

    /// 写入最新采样。空间不足时先丢最旧，保证实时路径不阻塞。
    pub fn push_latest(&self, src: &[f32]) {
        if src.is_empty() {
            return;
        }
        let cap = self.capacity();
        // 单包比环还大：只留最后 cap-1 个采样。
        if src.len() >= cap {
            let src = &src[src.len() - (cap - 1)..];
            self.overwrite(src);
            return;
        }
        let mut w = self.write.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let used = w.wrapping_sub(r);
        let free = cap.wrapping_sub(used).saturating_sub(1);
        if src.len() > free {
            let drop = src.len() - free;
            self.read.store(r.wrapping_add(drop), Ordering::Release);
        }
        let buf = unsafe { &mut *self.buf.get() };
        for &sample in src {
            buf[w & self.mask] = sample;
            w = w.wrapping_add(1);
        }
        self.write.store(w, Ordering::Release);
    }

    /// 覆盖式写入，读写指针一起跳到最新窗口。
    fn overwrite(&self, src: &[f32]) {
        let mut w = self.write.load(Ordering::Relaxed);
        let buf = unsafe { &mut *self.buf.get() };
        for &sample in src {
            buf[w & self.mask] = sample;
            w = w.wrapping_add(1);
        }
        self.write.store(w, Ordering::Release);
        self.read
            .store(w.wrapping_sub(src.len()), Ordering::Release);
    }

    /// 从环弹出到 `dst`。返回实际弹出数量；不足部分由调用方填静音。
    pub fn pop(&self, dst: &mut [f32]) -> usize {
        let mut r = self.read.load(Ordering::Relaxed);
        let w = self.write.load(Ordering::Acquire);
        let available = w.wrapping_sub(r).min(dst.len());
        let buf = unsafe { &*self.buf.get() };
        for sample in dst.iter_mut().take(available) {
            *sample = buf[r & self.mask];
            r = r.wrapping_add(1);
        }
        self.read.store(r, Ordering::Release);
        available
    }
}
