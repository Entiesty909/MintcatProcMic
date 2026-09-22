//! 全局开始/停止热键。独立消息窗口，游戏前台也能用。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, HWND_MESSAGE, MSG, PostMessageW, RegisterClassExW, SetTimer, TranslateMessage,
    UnregisterClassW, WINDOW_EX_STYLE, WM_HOTKEY, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS};
use windows::core::w;

use crate::config::Hotkey;

const HOTKEY_ID: i32 = 1;
const WM_STOP: u32 = 0x0400 + 33;

/// 后台热键服务。
pub struct HotkeyServer {
    /// packed: mods << 32 | vk
    packed: Arc<AtomicU64>,
    hwnd_bits: Arc<AtomicU64>,
}

impl HotkeyServer {
    /// 启动消息线程并注册默认热键。
    pub fn start(initial: Hotkey, on_hotkey: Arc<dyn Fn() + Send + Sync>) -> Self {
        let packed = Arc::new(AtomicU64::new(pack(initial)));
        let hwnd_bits = Arc::new(AtomicU64::new(0));
        let packed_t = packed.clone();
        let hwnd_t = hwnd_bits.clone();
        thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || hotkey_thread(packed_t, hwnd_t, on_hotkey))
            .ok();
        Self { packed, hwnd_bits }
    }

    /// 换成新组合。
    pub fn set(&self, hk: Hotkey) {
        self.packed.store(pack(hk), Ordering::SeqCst);
    }
}

impl Drop for HotkeyServer {
    fn drop(&mut self) {
        let bits = self.hwnd_bits.load(Ordering::SeqCst);
        if bits != 0 {
            let hwnd = HWND(bits as *mut _);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_STOP, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn pack(hk: Hotkey) -> u64 {
    (u64::from(hk.mods) << 32) | u64::from(hk.vk)
}

fn unpack(v: u64) -> Hotkey {
    Hotkey {
        mods: (v >> 32) as u32,
        vk: v as u32,
    }
}

fn hotkey_thread(
    packed: Arc<AtomicU64>,
    hwnd_bits: Arc<AtomicU64>,
    on_hotkey: Arc<dyn Fn() + Send + Sync>,
) {
    unsafe {
        let class = w!("ProcessMicHotkey");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class,
            ..Default::default()
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            tracing::warn!("RegisterClassEx hotkey failed");
            return;
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("ProcessMicHotkey"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        );
        let Ok(hwnd) = hwnd else {
            tracing::warn!("CreateWindowEx hotkey failed");
            let _ = UnregisterClassW(class, None);
            return;
        };
        hwnd_bits.store(hwnd.0 as u64, Ordering::SeqCst);
        let _ = SetTimer(Some(hwnd), 1, 200, None);
        let mut current = unpack(packed.load(Ordering::SeqCst));
        register(hwnd, current);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_STOP {
                break;
            }
            if msg.message == WM_HOTKEY {
                on_hotkey();
            }
            let now = unpack(packed.load(Ordering::Relaxed));
            if now.mods != current.mods || now.vk != current.vk {
                unregister(hwnd);
                register(hwnd, now);
                current = now;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        unregister(hwnd);
        let _ = DestroyWindow(hwnd);
        let _ = UnregisterClassW(class, None);
        hwnd_bits.store(0, Ordering::SeqCst);
    }
}

fn register(hwnd: HWND, hk: Hotkey) {
    unsafe {
        let mods = HOT_KEY_MODIFIERS(hk.mods);
        if RegisterHotKey(Some(hwnd), HOTKEY_ID, mods, hk.vk).is_err() {
            tracing::warn!("RegisterHotKey failed mods={} vk={}", hk.mods, hk.vk);
        }
    }
}

fn unregister(hwnd: HWND) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
