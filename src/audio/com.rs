//! COM 公寓初始化。音频线程使用 MTA。

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

use crate::error::Error;

/// 当前线程的 COM 作用域。`uninit == true` 时 Drop 会 `CoUninitialize`。
pub struct ComScope {
    /// 是否由本次调用完成初始化（已经是别的公寓则不反初始化）。
    uninit: bool,
}

/// 在当前线程进入 COM MTA。
///
/// 若线程已经是 STA，`RPC_E_CHANGED_MODE` 视为可继续使用现有公寓，不失败。
pub fn init_mta() -> Result<ComScope, Error> {
    // windows 0.62：CoInitializeEx 直接返回 HRESULT，不是 Result。
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_ok() {
        Ok(ComScope { uninit: true })
    } else if hr == RPC_E_CHANGED_MODE {
        Ok(ComScope { uninit: false })
    } else {
        Err(Error::Com(windows::core::Error::from_hresult(hr)))
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        // 只撤销我们自己初始化的那一次，避免拆掉 UI 线程的 STA。
        if self.uninit {
            unsafe { CoUninitialize() };
        }
    }
}
