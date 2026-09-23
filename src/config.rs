//! 应用配置：JSON 持久化、旧热键配置迁移、单实例互斥。

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{CloseHandle, HANDLE, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

/// 全局开始/停止热键。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Hotkey {
    /// RegisterHotKey 修饰符：1 alt, 2 ctrl, 4 shift。
    pub mods: u32,
    /// 虚拟键码。
    pub vk: u32,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self { mods: 0x0002 | 0x0004, vk: 0x77 }
    }
}

impl Hotkey {
    /// 给 UI 看的标签。
    pub fn label(self) -> String {
        let mut parts = Vec::new();
        if self.mods & 2 != 0 { parts.push("Ctrl"); }
        if self.mods & 4 != 0 { parts.push("Shift"); }
        if self.mods & 1 != 0 { parts.push("Alt"); }
        parts.push(vk_name(self.vk));
        parts.join("+")
    }
}

/// 全局热键集合。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hotkeys {
    /// 开始/停止转发。
    pub toggle_route: Hotkey,
    /// 停止全部声音。
    pub stop_all: Option<Hotkey>,
    /// 捕获方式。
    #[serde(default)]
    pub mode: HotkeyModeConfig,
}

impl Default for Hotkeys {
    fn default() -> Self { Self { toggle_route: Hotkey::default(), stop_all: None, mode: HotkeyModeConfig::System } }
}

/// 配置文件中的热键模式。
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyModeConfig {
    /// 安全默认：RegisterHotKey。
    #[default]
    System,
    /// 全局键盘/鼠标 hook；启用时必须显示风险提示。
    GlobalHook,
    /// 关闭。
    Disabled,
}
/// 输出设备与目标麦配置。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OutputConfig {
    /// WASAPI render ID，设备变更后用 name 兜底。
    pub device_id: Option<String>,
    /// 上次显示名。
    pub device_name: Option<String>,
    /// 配对的虚拟麦克风 ID。
    pub mic_id: Option<String>,
    /// 配对的虚拟麦克风显示名。
    pub mic_name: Option<String>,
    /// 是否把虚拟麦设成系统默认录音设备。
    pub set_default_mic: bool,
    /// 是否显示物理播放设备。
    pub all_devices: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RouteConfig {
    /// 上次选中的可执行文件名，用名字找新 PID。
    pub process_name: Option<String>,
    /// 进程退出后自动重连开关。
    pub auto_reconnect: bool,
    /// 开始转发时是否混入设置页选择的物理麦。
    #[serde(default)]
    pub mix_mic: bool,
}

/// 三路混音增益。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixConfig {
    /// 进程源增益。
    pub process: f32,
    /// 物理麦增益。
    pub mic: f32,
    /// 音效/文件增益。
    pub sfx: f32,
    /// 最终总增益。
    pub master: f32,
}

impl Default for MixConfig {
    fn default() -> Self { Self { process: 1.0, mic: 1.0, sfx: 1.0, master: 1.0 } }
}

/// 声板播放模式。
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayMode {
    /// 播放一次。
    #[default]
    Once,
    /// 播放到停止前循环。
    Loop,
    /// 按住播放，松开停止。
    Hold,
    /// 按一次开，再按一次关。
    Toggle,
}

/// 播放期间对游戏执行的全局按键动作。
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackKeyMode {
    /// 不模拟按键。
    #[default]
    None,
    /// 播放开始时按一次并立即松开。
    Press,
    /// 播放期间保持按下，停止时松开。
    Hold,
}

/// 全局 PTT / 游戏按键动作。不是某个固定音频的属性。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PlaybackKeyAction {
    /// 动作模式。
    #[serde(default)]
    pub mode: PlaybackKeyMode,
    /// 要模拟的按键。
    pub key: Hotkey,
}

/// 音频库中的一项。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioEntry {
    /// 显示名。
    pub name: String,
    /// 文件路径。
    pub path: String,
    /// 所属分类。
    #[serde(default = "default_category")]
    pub category: String,
    /// 是否循环。
    #[serde(default)]
    pub loop_playback: bool,
}

fn default_category() -> String { "未分类".into() }

/// 兼容旧版本的声板条目；音频库新条目使用 `AudioEntry`。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PadConfig {
    /// 显示名。
    pub name: String,
    /// 本地文件路径。
    pub path: String,
    /// 触发模式。
    pub mode: PlayMode,
    /// 独立音量。
    pub volume: f32,
    /// 触发热键。
    pub hotkey: Option<Hotkey>,
    /// 是否阻止游戏收到触发键。
    pub exclusive: bool,
    /// 是否打断其它 pad。
    pub interrupt: bool,
}

/// 播放器持久化状态。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerConfig {
    /// 最近一次文件。
    pub last_file: Option<String>,
    /// 是否循环。
    pub loop_playback: bool,
    /// 播放器触发热键。
    pub hotkey: Option<Hotkey>,
}

/// 配置文件根对象。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    /// 配置格式版本。
    pub version: u32,
    /// 输出配置。
    pub output: OutputConfig,
    /// 路由配置。
    pub route: RouteConfig,
    /// 混音配置。
    pub mix: MixConfig,
    /// 热键集合。
    pub hotkeys: Hotkeys,
    /// 声板 pad。
    pub pads: Vec<PadConfig>,
    /// 播放器。
    pub player: PlayerConfig,
    /// 音频库条目。
    #[serde(default)]
    pub audio_entries: Vec<AudioEntry>,
    /// 全局播放时按键动作；S4/S6 执行，默认不模拟。
    #[serde(default)]
    pub playback_key: Option<PlaybackKeyAction>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 2,
            output: OutputConfig::default(),
            route: RouteConfig::default(),
            mix: MixConfig::default(),
            hotkeys: Hotkeys::default(),
            pads: Vec::new(),
            player: PlayerConfig::default(),
            audio_entries: Vec::new(),
            playback_key: None,
        }
    }
}

/// 单实例互斥体。句柄存活期间保持应用独占。
pub struct SingleInstance { handle: HANDLE }

impl Drop for SingleInstance {
    fn drop(&mut self) { unsafe { let _ = CloseHandle(self.handle); } }
}

/// 获取命名互斥体；返回 None 表示已有实例。
pub fn acquire_single_instance() -> Result<Option<SingleInstance>, windows::core::Error> {
    let handle = unsafe { CreateMutexW(None, true, w!("Local\\ProcessMic.SingleInstance"))? };
    if unsafe { windows::Win32::Foundation::GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { let _ = CloseHandle(handle); }
        return Ok(None);
    }
    Ok(Some(SingleInstance { handle }))
}

/// 读配置。不存在、损坏或旧版本都回退默认并保留旧热键迁移。
pub fn load() -> AppConfig {
    let Ok(text) = fs::read_to_string(config_path()) else { return AppConfig::default(); };
    if let Ok(mut config) = serde_json::from_str::<AppConfig>(&text) {
        config.version = 2;
        return config;
    }
    if let Ok(legacy) = serde_json::from_str::<LegacyHotkey>(&text) {
        let mut config = AppConfig::default();
        config.hotkeys.toggle_route = legacy.into();
        return config;
    }
    tracing::warn!("config parse failed; using defaults");
    AppConfig::default()
}

/// 写配置；失败只打日志，不影响音频运行。
pub fn save(config: &AppConfig) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        if let Err(e) = fs::create_dir_all(dir) {
            tracing::warn!("create config directory: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(config) {
        Ok(body) => if let Err(e) = fs::write(&path, format!("{body}\n")) {
            tracing::warn!("save config: {e}");
        },
        Err(e) => tracing::warn!("serialize config: {e}"),
    }
}

#[derive(Deserialize)]
struct LegacyHotkey { mods: u32, vk: u32 }

impl From<LegacyHotkey> for Hotkey {
    fn from(value: LegacyHotkey) -> Self { Self { mods: value.mods, vk: value.vk } }
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("ProcessMic").join("config.json")
}

fn vk_name(vk: u32) -> &'static str {
    match vk {
        0x20 => "Space", 0x70 => "F1", 0x71 => "F2", 0x72 => "F3", 0x73 => "F4",
        0x74 => "F5", 0x75 => "F6", 0x76 => "F7", 0x77 => "F8", 0x78 => "F9",
        0x79 => "F10", 0x7A => "F11", 0x7B => "F12", _ => "Key",
    }
}

/// Slint 按键文本 → VK。
pub fn slint_key_to_vk(text: &str) -> Option<u32> {
    if text == " " || text.eq_ignore_ascii_case("space") { return Some(0x20); }
    if let Some(n) = text.strip_prefix('F').or_else(|| text.strip_prefix('f'))
        && let Ok(i) = n.parse::<u32>() && (1..=12).contains(&i) { return Some(0x70 + i - 1); }
    let mut ch = text.chars();
    let c = ch.next()?;
    if ch.next().is_some() {
        let v = c as u32;
        if (0xF001..=0xF00C).contains(&v) { return Some(0x70 + (v - 0xF001)); }
        return None;
    }
    let u = c.to_ascii_uppercase() as u32;
    if (0x30..=0x39).contains(&u) || (0x41..=0x5A).contains(&u) { Some(u) } else { None }
}
