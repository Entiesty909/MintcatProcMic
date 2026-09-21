//! 快捷键配置，保存在 %APPDATA%\ProcessMic\config.json。

use std::fs;
use std::path::PathBuf;

/// 全局开始/停止热键。
#[derive(Clone, Copy, Debug)]
pub struct Hotkey {
    /// RegisterHotKey 修饰符：1 alt, 2 ctrl, 4 shift。
    pub mods: u32,
    /// 虚拟键码。
    pub vk: u32,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            mods: 0x0002 | 0x0004, // Ctrl+Shift
            vk: 0x77,              // F8
        }
    }
}

impl Hotkey {
    /// 给 UI 看的标签。
    pub fn label(self) -> String {
        let mut parts = Vec::new();
        if self.mods & 2 != 0 {
            parts.push("Ctrl");
        }
        if self.mods & 4 != 0 {
            parts.push("Shift");
        }
        if self.mods & 1 != 0 {
            parts.push("Alt");
        }
        parts.push(vk_name(self.vk));
        parts.join("+")
    }
}

/// 读盘，坏文件则用默认。
pub fn load() -> Hotkey {
    let Ok(text) = fs::read_to_string(config_path()) else {
        return Hotkey::default();
    };
    parse(&text).unwrap_or_default()
}

/// 写盘，失败只打日志。
pub fn save(hk: Hotkey) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let body = format!("{{\"mods\":{},\"vk\":{}}}\n", hk.mods, hk.vk);
    if let Err(e) = fs::write(&path, body) {
        tracing::warn!("save config: {e}");
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("ProcessMic").join("config.json")
}

fn parse(text: &str) -> Option<Hotkey> {
    let mods = find_num(text, "mods")?;
    let vk = find_num(text, "vk")?;
    Some(Hotkey { mods, vk })
}

fn find_num(text: &str, key: &str) -> Option<u32> {
    let pat = format!("\"{key}\"");
    let i = text.find(&pat)?;
    let rest = text[i + pat.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn vk_name(vk: u32) -> &'static str {
    match vk {
        0x20 => "Space",
        0x70 => "F1",
        0x71 => "F2",
        0x72 => "F3",
        0x73 => "F4",
        0x74 => "F5",
        0x75 => "F6",
        0x76 => "F7",
        0x77 => "F8",
        0x78 => "F9",
        0x79 => "F10",
        0x7A => "F11",
        0x7B => "F12",
        _ => "Key",
    }
}

/// Slint 按键文本 → VK。
pub fn slint_key_to_vk(text: &str) -> Option<u32> {
    if text == " " || text.eq_ignore_ascii_case("space") {
        return Some(0x20);
    }
    if let Some(n) = text.strip_prefix('F').or_else(|| text.strip_prefix('f'))
        && let Ok(i) = n.parse::<u32>()
        && (1..=12).contains(&i)
    {
        return Some(0x70 + i - 1);
    }
    let mut ch = text.chars();
    let c = ch.next()?;
    if ch.next().is_some() {
        let v = c as u32;
        if (0xF001..=0xF00C).contains(&v) {
            return Some(0x70 + (v - 0xF001));
        }
        return None;
    }
    let u = c.to_ascii_uppercase() as u32;
    if (0x30..=0x39).contains(&u) || (0x41..=0x5A).contains(&u) {
        Some(u)
    } else {
        None
    }
}
