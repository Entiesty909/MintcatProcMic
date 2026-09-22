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
        other => {
            eprintln!("unknown argument: {other}");
            print_help();
            return Err(Error::InvalidArgs("unknown argument".into()));
        }
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
