//! 统一错误。UI 只展示 `user_message()`，细节走 Display / tracing。

use std::fmt;
use std::io;

/// 应用层错误。音频热路径禁止 unwrap。
#[derive(Debug)]
pub enum Error {
    /// windows-rs / WASAPI HRESULT。
    Com(windows::core::Error),
    /// 目标进程不存在或音频会话已结束。
    ProcessNotFound(u32),
    /// 输出设备 ID 无效或设备已拔出。
    DeviceNotFound,
    /// Process Loopback 初始化失败。
    CaptureInit(&'static str),
    /// 渲染端 Initialize / 打开设备失败。
    RenderInit(&'static str),
    /// mix 格式无法解析或无法转换。
    Format(&'static str),
    /// WAV 等文件 IO。
    Io(io::Error),
    /// 激活或等待超时。
    TimedOut(&'static str),
    /// CLI / UI 参数。
    InvalidArgs(String),
    /// VB-CABLE 下载或安装失败。
    Cable(String),
}

impl Error {
    /// 给用户看的短句，不含 HRESULT。
    pub fn user_message(&self) -> String {
        match self {
            Self::DeviceNotFound | Self::RenderInit(_) => {
                "无法连接到音频设备\n设备可能已经被拔出。".into()
            }
            Self::ProcessNotFound(_) => "进程已退出".into(),
            Self::CaptureInit(_) => "无法捕获该进程的音频".into(),
            Self::Format(_) => "采样率或音频格式不兼容".into(),
            Self::TimedOut(_) => "音频设备初始化超时".into(),
            Self::Com(_) => "无法初始化音频系统".into(),
            Self::Io(_) => "写入文件失败".into(),
            Self::InvalidArgs(msg) => msg.clone(),
            Self::Cable(msg) => msg.clone(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Com(e) => write!(f, "COM/WASAPI: {e}"),
            Self::ProcessNotFound(pid) => write!(f, "process {pid} not found"),
            Self::DeviceNotFound => write!(f, "audio device not found"),
            Self::CaptureInit(msg) => write!(f, "capture init: {msg}"),
            Self::RenderInit(msg) => write!(f, "render init: {msg}"),
            Self::Format(msg) => write!(f, "format: {msg}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::TimedOut(msg) => write!(f, "timeout: {msg}"),
            Self::InvalidArgs(msg) => write!(f, "{msg}"),
            Self::Cable(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Com(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<windows::core::Error> for Error {
    fn from(value: windows::core::Error) -> Self {
        Self::Com(value)
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
