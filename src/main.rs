//! ProcessMic：无参数开 UI；`--list-*` / `--capture` / `--route` 走 CLI。

mod audio;
mod config;
mod engine;
mod error;
mod hotkey;
mod process;
mod ui_bridge;
mod vbcable;

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use engine::AudioEngine;
use error::Error;

fn main() {
    // 声明 Per-Monitor V2 DPI 感知。不声明时系统会把窗口位图拉伸到 150%，界面发虚，
    // 而且 Win32 返回的工作区是虚拟化坐标，窗口就会算得比屏幕还大、底部跑到屏幕外。
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    // RUST_LOG 可覆盖；默认 info，音频热路径不打日志。
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// 解析 argv。无参数启动 Slint。
fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return ui_bridge::run_ui();
    }
    match args[0].as_str() {
        "--list-processes" => {
            for p in process::list_processes()? {
                println!("{}", p.label());
            }
        }
        "--list-devices" => {
            for d in audio::device::list_destinations(true)? {
                println!("{}\t{}", d.render_id, d.label);
            }
        }
        "--capture" => {
            let pid = parse_flag_u32(&args, "--capture")?;
            let wav = parse_flag_str(&args, "--wav").unwrap_or("capture.wav");
            let seconds = parse_flag_f32(&args, "--seconds").unwrap_or(5.0);
            println!("capturing pid {pid} for {seconds}s -> {wav}");
            audio::capture::capture_process_to_wav(pid, &PathBuf::from(wav), seconds)?;
            println!("wrote {wav}");
        }
        "--route" => {
            let pid = parse_flag_u32(&args, "--route")?;
            let device = parse_flag_str(&args, "--device")
                .ok_or_else(|| Error::InvalidArgs("missing --device <id>".into()))?;
            let seconds = parse_flag_f32(&args, "--seconds").unwrap_or(8.0);
            let engine = AudioEngine::new();
            engine.start(pid, device, None)?;
            println!("routing pid {pid} for {seconds}s");
            thread::sleep(Duration::from_secs_f32(seconds.max(0.5)));
            engine.stop();
            println!("stopped ({:?})", engine.status());
        }
        "--help" | "-h" => print_help(),
        "--snapshot-ui" => {
            let out = parse_flag_str(&args, "--out")
                .unwrap_or("snap.raw")
                .to_string();
            let menu = args.iter().any(|a| a == "--menu");
            ui_bridge::snapshot_to_file(&out, menu)?;
        }
        "--probe-playback" => {
            let file = parse_flag_str(&args, "--probe-playback")
                .ok_or_else(|| Error::InvalidArgs("missing audio file".into()))?;
            let seconds = parse_flag_f32(&args, "--seconds").unwrap_or(8.0);
            let start = parse_flag_f32(&args, "--start").unwrap_or(0.0);
            let out = parse_flag_str(&args, "--out")
                .unwrap_or("probe.wav")
                .to_string();
            let sfx = args.iter().any(|a| a == "--sfx");
            probe_playback(&PathBuf::from(file), seconds, start, &out, sfx)?;
        }
        other => {
            eprintln!("unknown argument: {other}");
            print_help();
            return Err(Error::InvalidArgs("unknown argument".into()));
        }
    }
    Ok(())
}

/// 参数里是否出现该 flag。
fn args_has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

/// 临时验证：按设备时钟消费播放源，落 WAV 并与原文件逐采样比对。
fn probe_playback(
    path: &std::path::Path,
    seconds: f32,
    start_secs: f32,
    out: &str,
    use_sfx: bool,
) -> Result<(), Error> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use audio::buffer::SpscRing;
    use audio::format::{AudioFormat, SampleKind};

    let reference = audio::decode::open(path)?.read_all()?;
    let rate = reference.sample_rate;
    let channels = reference.channels;
    let target = AudioFormat {
        sample_rate: rate,
        channels,
        kind: SampleKind::F32,
    };
    let start_hns = (start_secs.max(0.0) as f64 * 10_000_000.0) as u64;
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let capacity = ((rate as usize).saturating_mul(channels as usize) / 2).max(8192);
    let ring = Arc::new(SpscRing::with_capacity_samples(capacity));
    let (info, rx) = audio::decode::stream_to_channel(path.to_path_buf(), start_hns, false, cancel)?;
    if args_has(&std::env::args().collect::<Vec<_>>(), "--raw") {
        let mut got: Vec<f32> = Vec::new();
        while let Ok(chunk) = rx.recv_timeout(Duration::from_millis(500)) {
            got.extend_from_slice(&chunk.samples);
            if chunk.done {
                break;
            }
        }
        let compare = got.len().min(reference.samples.len());
        let mut first = None;
        let mut mismatch = 0usize;
        for i in 0..compare {
            if (got[i] - reference.samples[i]).abs() > 1e-3 {
                mismatch += 1;
                if first.is_none() {
                    first = Some(i);
                }
            }
        }
        println!(
            "source=raw 解码 {} 采样；首个不匹配 {:?}；不匹配 {}/{}",
            got.len(),
            first,
            mismatch,
            compare
        );
        return Ok(());
    }
    let mut keep_tx = None;
    let mut source = None;
    if use_sfx {
        let (tx, commands) = audio::sfx::channel();
        let ring_mixer = ring.clone();
        let stop_mixer = stop.clone();
        source = Some(thread::spawn(move || {
            audio::sfx::run_sfx_loop(ring_mixer, target, stop_mixer, commands)
        }));
        tx.send(audio::sfx::SfxCommand::PlayStream {
            sample_rate: rate,
            channels,
            mode: audio::sfx::SfxMode::Once,
            volume: 1.0,
            rx,
        })
        .map_err(|_| Error::RenderInit("probe: sfx channel closed"))?;
        keep_tx = Some(tx);
    } else {
        let ring_source = ring.clone();
        let stop_source = stop.clone();
        source = Some(thread::spawn(move || {
            audio::preview::run_stream_source(ring_source, target, info, rx, false, stop_source)
        }));
    }

    // 设备时钟：每 10 ms 取一块，余量不足的记录为空取次数。
    let frames = (rate as usize / 100).max(64);
    let block_samples = frames * channels as usize;
    let mut block = vec![0.0f32; block_samples];
    let mut got: Vec<f32> = Vec::with_capacity(rate as usize * channels as usize * 2);
    let mut short = 0usize;
    let start = Instant::now();
    let mut next = start;
    while start.elapsed().as_secs_f32() < seconds {
        next += Duration::from_millis(10);
        let n = ring.pop(&mut block);
        if n < block_samples {
            short += 1;
        }
        got.extend_from_slice(&block[..n]);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Release);

    let compare = got.len().min(reference.samples.len());
    // 跳转后要对齐到参考里的对应位置：解码器 seek 可能落在最近的可定位点上，
    // 所以先在期望位置附近找一个最佳偏移，再逐采样比较。
    let skip_frames = (start_hns as u128 * rate as u128 / 10_000_000) as usize;
    let skip = skip_frames * channels as usize;
    let probe_len = (channels as usize * 2000).min(got.len());
    let mut offset = skip;
    if probe_len > 0 {
        let mut best = f64::MAX;
        let low = skip.saturating_sub(channels as usize * 4096);
        let high = (skip + channels as usize * 4096).min(reference.samples.len());
        let mut candidate = low;
        while candidate + probe_len <= high {
            let err: f64 = (0..probe_len)
                .step_by(channels as usize * 7)
                .map(|i| (got[i] - reference.samples[candidate + i]).abs() as f64)
                .sum();
            if err < best {
                best = err;
                offset = candidate;
            }
            candidate += channels as usize;
        }
    }
    let compare = compare.min(reference.samples.len().saturating_sub(offset));
    let mut first = None;
    let mut mismatch = 0usize;
    for i in 0..compare {
        if (got[i] - reference.samples[offset + i]).abs() > 1e-3 {
            mismatch += 1;
            if first.is_none() {
                first = Some(i);
            }
        }
    }
    let drift = offset as i64 - skip as i64;
    println!(
        "source={} 起始 {:.1}s 对齐帧 {}/{}（偏移 {} 采样）；消费 {:.2}s / {} 采样；空取 {} 次；首个不匹配 {:?}；不匹配 {}/{}",
        if use_sfx { "sfx" } else { "preview" },
        start_secs,
        offset / channels as usize,
        skip / channels as usize,
        drift,
        got.len() as f32 / (rate as f32 * channels as f32),
        got.len(),
        short,
        first,
        mismatch,
        compare
    );
    audio::wav::write_pcm16_wav(std::path::Path::new(out), rate, channels, &got)?;
    let ref_out = format!("{out}.ref.wav");
    audio::wav::write_pcm16_wav(
        std::path::Path::new(&ref_out),
        rate,
        channels,
        &reference.samples,
    )?;
    println!("写出 {out} / {ref_out}");
    drop(keep_tx);
    if let Some(handle) = source {
        let _ = handle.join();
    }
    Ok(())
}

/// CLI 用法。
fn print_help() {
    eprintln!(
        "mintcat-proc-mic\n\
         \n\
         (no args)                         launch UI\n\
         --list-processes\n\
         --list-devices\n\
         --capture <pid> --wav <path> --seconds N\n\
         --route <pid> --device <id> --seconds N"
    );
}

/// 取 `--flag value` 的 value。
fn parse_flag_str<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].as_str())
}

/// 解析 PID。
fn parse_flag_u32(args: &[String], flag: &str) -> Result<u32, Error> {
    parse_flag_str(args, flag)
        .ok_or_else(|| Error::InvalidArgs("missing pid".into()))?
        .parse()
        .map_err(|_| Error::InvalidArgs("pid must be u32".into()))
}

/// 可选浮点 flag，解析失败当没传。
fn parse_flag_f32(args: &[String], flag: &str) -> Option<f32> {
    parse_flag_str(args, flag)?.parse().ok()
}
