//! 从官网拉最新 VB-CABLE 并提权运行安装包。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

use crate::error::Error;

/// 官网页面，用来解析当前 Driver_Pack 编号。
const CABLE_PAGE: &str = "https://vb-audio.com/Cable/";
/// 解析失败时用的已知最新包（页面标注 OCT 2024 Pack45）。
const FALLBACK_ZIP: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";

/// 下载最新包、解压、以管理员运行 Setup。可能需要重启后设备才出现。
pub fn install_latest() -> Result<(), Error> {
    let work = std::env::temp_dir().join("mintcat-vbcable");
    if work.exists() {
        let _ = fs::remove_dir_all(&work);
    }
    fs::create_dir_all(&work)?;
    let zip_path = work.join("VBCABLE.zip");
    let url = resolve_latest_zip_url();
    tracing::info!("downloading VB-CABLE from {url}");
    download(&url, &zip_path)?;
    extract_zip(&zip_path, &work)?;
    let setup = find_setup(&work).ok_or_else(|| {
        Error::Cable("安装包里没有找到 Setup.exe，请到 vb-audio.com 手动安装。".into())
    })?;
    tracing::info!("running {}", setup.display());
    run_as_admin(&setup)?;
    Ok(())
}

/// 抓 Cable 页面里编号最大的 `VBCABLE_Driver_PackNN.zip`。
fn resolve_latest_zip_url() -> String {
    let html = match download_string(CABLE_PAGE) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("fetch cable page failed: {e}");
            return FALLBACK_ZIP.into();
        }
    };
    let mut best = 0u32;
    let needle = "VBCABLE_Driver_Pack";
    let bytes = html.as_str();
    let mut start = 0;
    while let Some(rel) = bytes[start..].find(needle) {
        let at = start + rel + needle.len();
        let rest = &bytes[at..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if rest.starts_with(&format!("{digits}.zip"))
            && let Ok(n) = digits.parse::<u32>()
            && n > best
        {
            best = n;
        }
        start = at + 1;
        if start >= bytes.len() {
            break;
        }
    }
    if best == 0 {
        FALLBACK_ZIP.into()
    } else {
        format!("https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack{best}.zip")
    }
}

/// curl 下载到文件。Win11 自带 curl。
fn download(url: &str, dest: &Path) -> Result<(), Error> {
    let status = Command::new("curl")
        .args(["-fsSL", "--retry", "2", "-o"])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .status()
        .map_err(|_| Error::Cable("本机没有 curl，无法下载 VB-CABLE。".into()))?;
    if !status.success() {
        return Err(Error::Cable("下载 VB-CABLE 失败，请检查网络。".into()));
    }
    if dest.metadata().map(|m| m.len()).unwrap_or(0) < 1024 {
        return Err(Error::Cable("下载的安装包不完整。".into()));
    }
    Ok(())
}

/// curl 把页面拉到内存（有上限）。
fn download_string(url: &str) -> Result<String, Error> {
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "2", url])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| Error::Cable("本机没有 curl，无法下载 VB-CABLE。".into()))?;
    if !out.status.success() {
        return Err(Error::Cable("无法打开 VB-CABLE 官网。".into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Windows 自带 tar 解 zip。
fn extract_zip(zip: &Path, dest: &Path) -> Result<(), Error> {
    let status = Command::new("tar")
        .args(["-xf"])
        .arg(zip)
        .arg("-C")
        .arg(dest)
        .stdin(Stdio::null())
        .status()
        .map_err(|_| Error::Cable("无法解压安装包。".into()))?;
    if !status.success() {
        return Err(Error::Cable("解压 VB-CABLE 安装包失败。".into()));
    }
    Ok(())
}

/// 优先 x64 Setup。
fn find_setup(root: &Path) -> Option<PathBuf> {
    let mut found = Vec::new();
    collect_setups(root, &mut found);
    found.sort_by_key(|p| {
        let n = p
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if n.contains("x64") {
            0
        } else if n.contains("setup") {
            1
        } else {
            2
        }
    });
    found.into_iter().next()
}

/// 递归收集 *setup*.exe。
fn collect_setups(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            collect_setups(&path, out);
            continue;
        }
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name.ends_with(".exe") && name.contains("setup") {
            out.push(path);
        }
    }
}

/// `runas` 启动安装程序并等待退出。
fn run_as_admin(setup: &Path) -> Result<(), Error> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = setup
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(wide.as_ptr()),
        nShow: SW_SHOWNORMAL.0 as i32,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info)
            .map_err(|_| Error::Cable("已取消或无法提权安装 VB-CABLE。".into()))?;
        if !info.hProcess.is_invalid() {
            let wr = WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
            if wr != WAIT_OBJECT_0 {
                return Err(Error::Cable("安装程序异常结束。".into()));
            }
        }
    }
    Ok(())
}

/// 设备名或驱动标识是否像 VB-CABLE。
pub fn device_looks_like_cable(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.contains("VB-AUDIO") || n.contains("VB AUDIO") || n.contains("CABLE") || n.contains("VIRTUAL AUDIO")
}
