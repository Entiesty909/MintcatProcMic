//! 进程枚举：有窗口或正在出声的进程，去掉系统噪声。

use std::collections::{HashMap, HashSet};
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH};
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IAudioSessionControl, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GW_OWNER, GetWindow, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::Interface;
use windows_core::BOOL;

use crate::audio::com;
use crate::error::Error;

/// 一条可被 UI 选中的进程。
#[derive(Clone, Debug)]
pub struct ProcessInfo {
    /// 进程 ID。
    pub pid: u32,
    /// 可执行文件名，如 `notepad.exe`。
    pub name: String,
    /// 可见顶层窗口标题（没有窗口则为 None）。
    pub title: Option<String>,
    /// 当前是否有 WASAPI 音频会话。
    pub sounding: bool,
}

impl ProcessInfo {
    /// ComboBox 显示文本。正在出声的加标记。
    pub fn label(&self) -> String {
        let mark = if self.sounding { "出声 · " } else { "" };
        match &self.title {
            Some(title) if !title.is_empty() => {
                format!("{mark}{}  [{}]  {}", self.name, self.pid, title)
            }
            _ => format!("{mark}{}  [{}]", self.name, self.pid),
        }
    }
}

/// 只保留：有可见窗口，或正在出声。排除自身和系统进程。
pub fn list_processes() -> Result<Vec<ProcessInfo>, Error> {
    let self_pid = std::process::id();
    let titles = window_titles();
    let sounding = audio_session_pids();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? };
    if snapshot.is_invalid() {
        return Err(Error::CaptureInit("process snapshot failed"));
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut list = Vec::new();
    unsafe {
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                if pid > 4 && pid != self_pid {
                    let name = wchar_to_string(&entry.szExeFile);
                    if !name.is_empty() && !is_noise_process(&name) {
                        let title = titles.get(&pid).cloned();
                        let has_audio = sounding.contains(&pid);
                        if title.is_some() || has_audio {
                            list.push(ProcessInfo {
                                pid,
                                name,
                                title,
                                sounding: has_audio,
                            });
                        }
                    }
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }

    list.sort_by(|a, b| {
        b.sounding
            .cmp(&a.sounding)
            .then(b.title.is_some().cmp(&a.title.is_some()))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(list)
}

/// 服务/壳进程，几乎不会作为音频来源。
fn is_noise_process(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "svchost.exe"
            | "csrss.exe"
            | "smss.exe"
            | "wininit.exe"
            | "winlogon.exe"
            | "lsass.exe"
            | "services.exe"
            | "fontdrvhost.exe"
            | "dwm.exe"
            | "conhost.exe"
            | "runtimebroker.exe"
            | "searchhost.exe"
            | "searchindexer.exe"
            | "searchfilterhost.exe"
            | "searchprotocolhost.exe"
            | "dllhost.exe"
            | "wmiprvse.exe"
            | "sihost.exe"
            | "taskhostw.exe"
            | "ctfmon.exe"
            | "textinputhost.exe"
            | "applicationframehost.exe"
            | "shellexperiencehost.exe"
            | "startmenuexperiencehost.exe"
            | "securityhealthsystray.exe"
            | "securityhealthservice.exe"
            | "nvcontainer.exe"
            | "nvdisplay.container.exe"
            | "aggregatorhost.exe"
            | "registry"
            | "secure system"
            | "system"
            | "idle"
            | "memory compression"
            | " audiodg.exe"
            | "audiodg.exe"
            | "wudfhost.exe"
            | "spoolsv.exe"
            | "dashost.exe"
            | "lsaiso.exe"
            | "ngciso.exe"
            | "smartscreen.exe"
            | "widgetservice.exe"
            | "widgets.exe"
            | "msedgewebview2.exe"
            | "crashpad_handler.exe"
            | "elevation_service.exe"
    )
}

/// 所有活动渲染设备上的音频会话 PID。失败则空集，不挡窗口进程。
fn audio_session_pids() -> HashSet<u32> {
    let mut pids = HashSet::new();
    let _com = match com::init_mta() {
        Ok(c) => c,
        Err(_) => return pids,
    };
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(_) => return pids,
            };
        let collection = match enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            Ok(c) => c,
            Err(_) => return pids,
        };
        let count = collection.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(device) = collection.Item(i) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            let Ok(n) = sessions.GetCount() else {
                continue;
            };
            for j in 0..n {
                let Ok(ctl) = sessions.GetSession(j) else {
                    continue;
                };
                let ctl: IAudioSessionControl = ctl;
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                if let Ok(pid) = ctl2.GetProcessId()
                    && pid > 4
                {
                    pids.insert(pid);
                }
            }
        }
    }
    pids
}

/// 可见、无 owner 的顶层窗口 → PID 标题。每个 PID 只留第一个。
fn window_titles() -> HashMap<u32, String> {
    let mut titles = HashMap::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut titles as *mut HashMap<u32, String> as isize),
        );
    }
    titles
}

/// EnumWindows 回调。过滤工具窗口和不可见窗口。
unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let titles = unsafe { &mut *(lparam.0 as *mut HashMap<u32, String>) };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
        if owner != HWND::default() {
            return true.into();
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || titles.contains_key(&pid) {
            return true.into();
        }
        let mut buf = [0u16; MAX_PATH as usize];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n > 0 {
            let title = String::from_utf16_lossy(&buf[..n as usize]);
            if !title.is_empty() {
                titles.insert(pid, title);
            }
        }
    }
    true.into()
}

/// 截到第一个 NUL 的 UTF-16 → String。
fn wchar_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
