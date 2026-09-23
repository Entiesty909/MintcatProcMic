//! 用未文档化的 IPolicyConfig 切换系统默认录音设备。失败则忽略。

use std::ffi::c_void;

use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::core::{GUID, HRESULT, IUnknown, Interface, PCWSTR};

use super::com;
use crate::error::Error;

/// `{870AF99C-171D-4F9E-AF0D-E63DF40C2BC9}`
const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870a_f99c_171d_4f9e_af0d_e63d_f40c_2bc9);

/// IUnknown(3) + 10 个无关方法 + SetDefaultEndpoint。
#[repr(C)]
struct PolicyConfigVtbl {
    _iunknown: [*const (); 3],
    _unused: [*const (); 10],
    set_default_endpoint: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
}

/// 把设备设为三种录音 role 的默认麦。
pub fn set_default_capture(device_id: &str) -> std::result::Result<(), Error> {
    set_default(device_id, &[0, 1, 2])
}

/// 把播放端设为系统默认播放设备（console + multimedia）。
pub fn set_default_render(device_id: &str) -> std::result::Result<(), Error> {
    set_default(device_id, &[0, 1])
}

/// 把播放端设为默认通信设备。
pub fn set_default_communications(device_id: &str) -> std::result::Result<(), Error> {
    set_default(device_id, &[2])
}

/// 恢复之前保存的默认端点（role, id）。
pub fn restore_defaults(saved: &[(u32, String)]) -> std::result::Result<(), Error> {
    for (role, id) in saved { set_default(id, &[*role])?; }
    Ok(())
}

fn set_default(device_id: &str, roles: &[u32]) -> std::result::Result<(), Error> {
    let _com = com::init_mta()?;
    let wide: Vec<u16> = device_id.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let unk: IUnknown = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL)
            .map_err(|_| Error::Cable("无法切换系统默认麦克风。".into()))?;
        let this = Interface::as_raw(&unk);
        let vtbl = &**(this as *mut *const PolicyConfigVtbl);
        for role in roles {
            let hr = (vtbl.set_default_endpoint)(this, PCWSTR(wide.as_ptr()), *role);
            if hr.is_err() {
                tracing::warn!("SetDefaultEndpoint role {role} failed: {hr:?}");
                return Err(Error::Cable("无法把虚拟麦设成系统默认麦克风。".into()));
            }
        }
    }
    Ok(())
}
