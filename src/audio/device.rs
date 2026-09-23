//! 枚举 WASAPI 端点。输出列表是全部播放设备，不靠你改过的友好名称过滤。

use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_DeviceInterface_FriendlyName, PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    eCapture, eCommunications, eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, STGM_READ};
use windows::Win32::System::Variant::{VT_BSTR, VT_LPWSTR};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use crate::error::Error;

use super::com;

/// 一只物理播放设备：友好名称 + 设备 ID。
#[derive(Clone, Debug)]
pub struct RenderDevice {
    /// WASAPI 设备 ID（不是 MME 下标）。
    pub id: String,
    /// 友好名称（用户改名后就是新名字）。
    pub name: String,
}

/// 物理麦克风：排除虚拟线缆录音端，避免把 CABLE Output 再混进去。
pub fn list_physical_mics() -> Result<Vec<RenderDevice>, Error> {
    Ok(list_endpoints(eCapture, eCommunications)?
        .into_iter()
        .filter(|d| !is_virtual_driver(d))
        .map(Endpoint::into_render)
        .collect())
}

/// UI 输出列表里的一项。
#[derive(Clone, Debug)]
pub struct DestDevice {
    /// ComboBox 文案（用当前友好名称）。
    pub label: String,
    /// 实际 WASAPI 渲染目标。
    pub render_id: String,
    /// 若能配到录音端，设默认麦用。
    pub capture_id: Option<String>,
    /// 游戏里应选择的麦克风名（也可能是你改过的名字）。
    pub capture_name: Option<String>,
}

/// 输出列表：虚拟扬声器与虚拟麦克风各一条，不做「扬声器 → 麦」拼接。
/// `include_all` 为真时把物理播放设备也一并列出（设置页的开关）。
pub fn list_destinations(include_all: bool) -> Result<Vec<DestDevice>, Error> {
    let renders = list_endpoints(eRender, eConsole)?;
    let captures = list_endpoints(eCapture, eCommunications)?;
    let mut out = Vec::new();

    for r in &renders {
        if !include_all && !is_virtual_driver(r) {
            continue;
        }
        out.push(DestDevice {
            label: r.name.clone(),
            render_id: r.id.clone(),
            capture_id: None,
            capture_name: None,
        });
    }

    for c in &captures {
        if !is_virtual_driver(c) {
            continue;
        }
        let Some(r) = paired_render(c, &renders) else {
            continue;
        };
        out.push(DestDevice {
            label: c.name.clone(),
            render_id: r.id.clone(),
            capture_id: Some(c.id.clone()),
            capture_name: Some(c.name.clone()),
        });
    }

    Ok(out)
}

/// 当前默认录音设备 ID（三种 role）。
pub fn default_capture_ids() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for (role_n, role) in [(0u32, eConsole), (1, eMultimedia), (2, eCommunications)] {
        if let Ok(id) = default_endpoint_id(eCapture, role) { out.push((role_n, id)); }
    }
    out
}

/// 当前默认播放端的 console role 设备 ID。
pub fn default_render_id() -> Result<String, Error> {
    default_endpoint_id(eRender, eConsole)
}

/// 当前默认播放设备 ID（三种 role）。
pub fn default_render_ids() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for (role_n, role) in [(0u32, eConsole), (1, eMultimedia), (2, eCommunications)] {
        if let Ok(id) = default_endpoint_id(eRender, role) { out.push((role_n, id)); }
    }
    out
}
#[derive(Clone, Debug)]
struct Endpoint {
    id: String,
    name: String,
    desc: String,
    adapter: String,
    is_default: bool,
}

impl Endpoint {
    fn into_render(self) -> RenderDevice {
        RenderDevice {
            id: self.id,
            name: self.name,
        }
    }
}

fn list_endpoints(flow: EDataFlow, default_role: ERole) -> Result<Vec<Endpoint>, Error> {
    let _com = com::init_mta()?;
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let collection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        let default_id = enumerator
            .GetDefaultAudioEndpoint(flow, default_role)
            .ok()
            .and_then(|d| device_id(&d).ok());

        let mut devices = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = collection.Item(i)?;
            let id = device_id(&device)?;
            let name = prop_key(&device, &PKEY_Device_FriendlyName).unwrap_or_else(|| id.clone());
            let desc = prop_key(&device, &PKEY_Device_DeviceDesc).unwrap_or_default();
            let adapter = prop_key(&device, &PKEY_DeviceInterface_FriendlyName).unwrap_or_default();
            let is_default = default_id.as_ref() == Some(&id);
            devices.push(Endpoint {
                id,
                name,
                desc,
                adapter,
                is_default,
            });
        }
        devices.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.name.cmp(&b.name)));
        Ok(devices)
    }
}

fn default_endpoint_id(flow: EDataFlow, role: ERole) -> Result<String, Error> {
    let _com = com::init_mta()?;
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(flow, role)?;
        device_id(&device)
    }
}

fn paired_render<'a>(capture: &Endpoint, renders: &'a [Endpoint]) -> Option<&'a Endpoint> {
    if !is_virtual_driver(capture) {
        return None;
    }
    let mut same: Vec<&Endpoint> = renders
        .iter()
        .filter(|r| is_virtual_driver(r) && same_driver(capture, r))
        .collect();
    if same.is_empty() {
        return None;
    }
    if same.len() == 1 {
        return Some(same[0]);
    }
    if let Some(r) = same
        .iter()
        .copied()
        .find(|r| complementary(&r.desc, &capture.desc))
    {
        return Some(r);
    }
    same.sort_by(|a, b| a.name.cmp(&b.name));
    Some(same[0])
}
fn same_driver(a: &Endpoint, b: &Endpoint) -> bool {
    if !a.adapter.is_empty()
        && !b.adapter.is_empty()
        && a.adapter.eq_ignore_ascii_case(&b.adapter)
    {
        return driver_family(a) == driver_family(b);
    }
    driver_family(a) == driver_family(b)
}

fn is_virtual_driver(ep: &Endpoint) -> bool {
    looks_virtual(&ep.adapter) || looks_virtual(&ep.desc)
}

fn looks_virtual(s: &str) -> bool {
    let n = s.to_ascii_uppercase();
    n.contains("VB-AUDIO")
        || n.contains("VB AUDIO")
        || n.contains("VOICEMEETER")
        || n.contains("STEAM STREAMING")
        || n.contains("VIRTUAL AUDIO")
        || n.contains("CABLE")
}

fn driver_family(ep: &Endpoint) -> &'static str {
    let n = format!("{} {}", ep.adapter, ep.desc).to_ascii_uppercase();
    if n.contains("16") && (n.contains("CABLE") || n.contains("VB-AUDIO")) {
        "cable16"
    } else if n.contains("CABLE") || n.contains("VB-AUDIO") {
        "cable"
    } else if n.contains("VOICEMEETER") {
        "voicemeeter"
    } else if n.contains("STEAM STREAMING") {
        "steam"
    } else {
        "other"
    }
}

fn complementary(render_desc: &str, capture_desc: &str) -> bool {
    let r = render_desc.to_ascii_uppercase();
    let c = capture_desc.to_ascii_uppercase();
    let render_in = r.contains("INPUT") || r.contains("IN ") || r.contains("输入") || r.contains("SPEAKER");
    let capture_out = c.contains("OUTPUT")
        || c.contains("OUT")
        || c.contains("输出")
        || c.contains("MIC")
        || c.contains("麦克");
    render_in && capture_out
}

/// IMMDevice::GetId，调用方负责理解这是系统设备字符串。
pub fn device_id(device: &IMMDevice) -> Result<String, Error> {
    unsafe {
        let pwstr = device.GetId()?;
        let id = pwstr.to_string().unwrap_or_default();
        CoTaskMemFree(Some(pwstr.0 as *const _));
        if id.is_empty() {
            Err(Error::DeviceNotFound)
        } else {
            Ok(id)
        }
    }
}

fn prop_key(device: &IMMDevice, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut pv = store.GetValue(key).ok()?;
        let name = propvariant_string(&pv);
        let _ = windows::Win32::System::Com::StructuredStorage::PropVariantClear(&mut pv);
        if name.is_empty() { None } else { Some(name) }
    }
}

/// 从 PROPVARIANT 取宽字符串。
fn propvariant_string(pv: &windows::Win32::System::Com::StructuredStorage::PROPVARIANT) -> String {
    unsafe {
        let vt = pv.Anonymous.Anonymous.vt;
        if vt == VT_LPWSTR || vt == VT_BSTR {
            let p = pv.Anonymous.Anonymous.Anonymous.pwszVal;
            p.to_string().unwrap_or_default()
        } else {
            String::new()
        }
    }
}
