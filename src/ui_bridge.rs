//! Slint 桥：分页界面、列表填充、状态轮询、热键。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use slint::{ModelRc, Timer, TimerMode, VecModel};

use crate::audio::device::{self, DestDevice};
use crate::audio::policy;
use crate::config;
use crate::engine::{AudioEngine, EngineStatus};
use crate::error::Error;
use crate::hotkey::HotkeyServer;
use crate::process::{self, ProcessInfo};
use crate::vbcable;

slint::include_modules!();

/// 声纹柱数量。
const WAVE_BARS: usize = 32;

/// UI 列表缓存。当前项按 pid / 设备 ID 记住，不拿 ComboBox 下标当身份。
#[derive(Default)]
struct UiCache {
    /// 全部进程（未筛选）。
    processes: Vec<ProcessInfo>,
    /// 当前列出的进程 PID，下标与 ComboBox 对齐。
    pids: Vec<u32>,
    /// 输出设备列表。
    dests: Vec<DestDevice>,
    /// 麦克风设备 ID，下标 0 是「不混入麦克风」。
    mic_ids: Vec<String>,
    /// 麦克风显示名，与 `mic_ids` 对齐。
    mic_names: Vec<String>,
    /// 进程筛选关键字。
    filter: String,
    /// 输出列表是否包含物理播放设备。
    all_devices: bool,
    /// 是否把虚拟麦设成系统默认麦克风。
    set_default_mic: bool,
}

/// 首选窗口尺寸（逻辑像素）。实际会按显示器工作区收窄。
const WINDOW_W: f32 = 1080.0;
const WINDOW_H: f32 = 560.0;

/// 创建窗口、填列表、跑事件循环。退出时还原默认麦并停引擎。
pub fn run_ui() -> Result<(), Error> {
    let Some(_instance) = config::acquire_single_instance()? else {
        return Ok(());
    };
    let ui = MainWindow::new().map_err(|e| Error::InvalidArgs(e.to_string()))?;
    let engine = Rc::new(AudioEngine::new());
    let app_config = Rc::new(RefCell::new(config::load()));
    let persisted = app_config.borrow().clone();
    let cache = Rc::new(RefCell::new(UiCache {
        all_devices: persisted.output.all_devices,
        set_default_mic: persisted.output.set_default_mic,
        ..UiCache::default()
    }));
    let saved_mics = Rc::new(RefCell::new(Vec::<(u32, String)>::new()));
    let capture_hist = Rc::new(RefCell::new(vec![0.0f32; WAVE_BARS]));
    let mic_hist = Rc::new(RefCell::new(vec![0.0f32; WAVE_BARS]));
    let render_hist = Rc::new(RefCell::new(vec![0.0f32; WAVE_BARS]));

    let hotkey = Rc::new(RefCell::new(persisted.hotkeys.toggle_route));
    ui.set_hotkey_label(hotkey.borrow().label().into());
    ui.set_volume((persisted.mix.process * 100.0).clamp(0.0, 100.0));
    ui.set_mic_volume((persisted.mix.mic * 100.0).clamp(0.0, 100.0));
    ui.set_master_volume((persisted.mix.master * 100.0).clamp(0.0, 100.0));
    ui.set_all_devices(persisted.output.all_devices);
    ui.set_set_default_mic(persisted.output.set_default_mic);
    ui.window()
        .show()
        .map_err(|e| Error::InvalidArgs(e.to_string()))?;
    fit_window_to_screen(&ui);
    refill(&ui, &cache, true);
    restore_saved_selection(&ui, &cache, &persisted);
    set_wave(
        &ui,
        &capture_hist.borrow(),
        &mic_hist.borrow(),
        &render_hist.borrow(),
    );

    let hotkey_server = Rc::new(start_hotkey(&ui, &hotkey));
    wire_callbacks(
        &ui,
        &engine,
        &cache,
        &saved_mics,
        &hotkey,
        &hotkey_server,
        &app_config,
    );
    let _timer = start_status_timer(
        &ui,
        &engine,
        &cache,
        &saved_mics,
        &capture_hist,
        &mic_hist,
        &render_hist,
    );
    let _hotkey_server = hotkey_server;

    ui.run()
        .map_err(|e| Error::InvalidArgs(e.to_string()))?;
    persist_config(&ui, &cache, &app_config);
    restore_default_mics(&saved_mics);
    engine.stop();
    Ok(())
}

/// 把窗口尺寸按逻辑像素收窄到显示器工作区内。
/// 高 DPI 小屏如果直接使用默认尺寸，底部状态栏会落到屏幕外。
fn fit_window_to_screen(ui: &MainWindow) {
    let scale = ui.window().scale_factor().max(1.0);
    let (work_w, work_h) = primary_work_area().unwrap_or((
        (WINDOW_W * scale) as u32,
        (WINDOW_H * scale) as u32,
    ));
    let available_w = work_w as f32 / scale;
    let available_h = work_h as f32 / scale;
    let width = WINDOW_W.min(available_w - 12.0).max(900.0);
    let height = WINDOW_H.min(available_h - 24.0).max(520.0);
    ui.window()
        .set_size(slint::LogicalSize::new(width, height));
}

/// 主显示器工作区（物理像素）。取不到就返回 None。
fn primary_work_area() -> Option<(u32, u32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut rc = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rc as *mut RECT as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() && rc.right > rc.left && rc.bottom > rc.top {
        Some(((rc.right - rc.left) as u32, (rc.bottom - rc.top) as u32))
    } else {
        None
    }
}

/// 把 Slint 回调接到引擎与列表缓存。
fn wire_callbacks(
    ui: &MainWindow,
    engine: &Rc<AudioEngine>,
    cache: &Rc<RefCell<UiCache>>,
    saved_mics: &Rc<RefCell<Vec<(u32, String)>>>,
    hotkey: &Rc<RefCell<config::Hotkey>>,
    hotkey_server: &Rc<HotkeyServer>,
    app_config: &Rc<RefCell<config::AppConfig>>,
) {
    {
        let engine = engine.clone();
        let app_config = app_config.clone();
        ui.on_volume_edited(move |v| {
            let value = (v / 100.0).clamp(0.0, 1.0);
            engine.set_volume(value);
            let mut config = app_config.borrow_mut();
            config.mix.process = value;
            config::save(&config);
        });
    }
    {
        let engine = engine.clone();
        let app_config = app_config.clone();
        ui.on_mic_volume_edited(move |v| {
            let value = (v / 100.0).clamp(0.0, 1.0);
            engine.set_mic_volume(value);
            let mut config = app_config.borrow_mut();
            config.mix.mic = value;
            config::save(&config);
        });
    }
    {
        let engine = engine.clone();
        let app_config = app_config.clone();
        ui.on_master_volume_edited(move |v| {
            let value = (v / 100.0).clamp(0.0, 1.0);
            engine.set_master_volume(value);
            let mut config = app_config.borrow_mut();
            config.mix.master = value;
            config::save(&config);
        });
    }
    {
        let ui_weak = ui.as_weak();
        let cache = cache.clone();
        ui.on_process_filter_changed(move |text| {
            let Some(ui) = ui_weak.upgrade() else { return };
            cache.borrow_mut().filter = text.to_string();
            refill_processes(&ui, &cache);
        });
    }
    {
        let ui_weak = ui.as_weak();
        let cache = cache.clone();
        let app_config = app_config.clone();
        ui.on_all_devices_toggled(move |on| {
            let Some(ui) = ui_weak.upgrade() else { return };
            cache.borrow_mut().all_devices = on;
            app_config.borrow_mut().output.all_devices = on;
            refill_devices(&ui, &cache, false);
        });
    }
    {
        let cache = cache.clone();
        let app_config = app_config.clone();
        ui.on_set_default_mic_toggled(move |on| {
            cache.borrow_mut().set_default_mic = on;
            app_config.borrow_mut().output.set_default_mic = on;
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_install_cable(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            if ui.get_cable_busy() {
                return;
            }
            ui.set_cable_busy(true);
            ui.set_cable_text("正在下载最新版…".into());
            ui.set_error_text("".into());
            let ui_weak = ui.as_weak();
            thread::spawn(move || {
                let result = vbcable::install_latest();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    ui.set_cable_busy(false);
                    match result {
                        Ok(()) => {
                            ui.set_cable_text("安装结束，请重启后点刷新".into());
                            ui.set_error_text("".into());
                        }
                        Err(e) => {
                            tracing::warn!("vb-cable install: {e}");
                            ui.set_cable_text("安装失败".into());
                            ui.set_error_text(e.user_message().into());
                        }
                    }
                });
            });
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_begin_hotkey_capture(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_listening_hotkey(true);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        let hotkey = hotkey.clone();
        let server = hotkey_server.clone();
        let app_config = app_config.clone();
        ui.on_hotkey_captured(move |text, ctrl, shift, alt| {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_listening_hotkey(false);
            let Some(vk) = config::slint_key_to_vk(text.as_str()) else {
                return;
            };
            let mut mods = 0u32;
            if alt { mods |= 1; }
            if ctrl { mods |= 2; }
            if shift { mods |= 4; }
            let hk = config::Hotkey { mods, vk };
            *hotkey.borrow_mut() = hk;
            server.set(hk);
            let mut config = app_config.borrow_mut();
            config.hotkeys.toggle_route = hk;
            config::save(&config);
            ui.set_hotkey_label(hk.label().into());
        });
    }
    {
        let ui_weak = ui.as_weak();
        let hotkey = hotkey.clone();
        let server = hotkey_server.clone();
        let app_config = app_config.clone();
        ui.on_reset_hotkey(move || {
            let hk = config::Hotkey::default();
            *hotkey.borrow_mut() = hk;
            server.set(hk);
            let mut config = app_config.borrow_mut();
            config.hotkeys.toggle_route = hk;
            config::save(&config);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_listening_hotkey(false);
                ui.set_hotkey_label(hk.label().into());
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        let engine = engine.clone();
        let cache = cache.clone();
        let saved_mics = saved_mics.clone();
        let app_config = app_config.clone();
        ui.on_start_stop(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            if engine.is_running() {
                engine.stop();
                restore_default_mics(&saved_mics);
                ui.set_running(false);
                ui.set_status_text("未运行".into());
                ui.set_status_kind(0);
                ui.set_error_text("".into());
                return;
            }
            let (pid, dest, mic_id, set_default, process_name) = {
                let c = cache.borrow();
                let pid = c.pids.get(ui.get_process_index() as usize).copied().filter(|p| *p != 0);
                let dest = c.dests.get(ui.get_device_index() as usize).cloned();
                let mic_index = ui.get_mic_index() as usize;
                let mic_id = c.mic_ids.get(mic_index).filter(|s| !s.is_empty()).cloned();
                let process_name = pid.and_then(|pid| c.processes.iter().find(|p| p.pid == pid).map(|p| p.name.clone()));
                (pid, dest, mic_id, c.set_default_mic, process_name)
            };
            let (Some(pid), Some(dest)) = (pid, dest) else {
                ui.set_error_text("请选择进程和输出设备".into());
                return;
            };
            engine.set_volume(ui.get_volume() / 100.0);
            engine.set_mic_volume(ui.get_mic_volume() / 100.0);
            engine.set_master_volume(ui.get_master_volume() / 100.0);
            match engine.start(pid, &dest.render_id, mic_id.as_deref()) {
                Ok(()) => {
                    let mut config = app_config.borrow_mut();
                    config.route.process_name = process_name;
                    config.output.device_id = Some(dest.render_id.clone());
                    config.output.device_name = Some(dest.label.clone());
                    config.output.mic_id = dest.capture_id.clone();
                    config.output.mic_name = dest.capture_name.clone();
                    config.output.set_default_mic = set_default;
                    config::save(&config);
                    ui.set_running(true);
                    ui.set_status_text("运行中".into());
                    ui.set_status_kind(1);
                    ui.set_error_text("".into());
                    apply_virtual_mic_route(&ui, &dest, set_default, &saved_mics);
                }
                Err(e) => {
                    tracing::warn!("start failed: {e}");
                    ui.set_running(false);
                    ui.set_status_text("错误".into());
                    ui.set_status_kind(3);
                    ui.set_error_text(e.user_message().into());
                }
            }
        });
    }
}

/// 启动全局热键服务，热键等同于点一次开始/停止。
fn start_hotkey(ui: &MainWindow, hotkey: &Rc<RefCell<config::Hotkey>>) -> HotkeyServer {
    let ui_weak = ui.as_weak();
    HotkeyServer::start(
        *hotkey.borrow(),
        Arc::new(move || {
            let ui_weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_start_stop();
                }
            });
        }),
    )
}

/// 50 ms 轮询引擎状态、电平与列表变化。
#[allow(clippy::too_many_arguments)]
fn start_status_timer(
    ui: &MainWindow,
    engine: &Rc<AudioEngine>,
    cache: &Rc<RefCell<UiCache>>,
    saved_mics: &Rc<RefCell<Vec<(u32, String)>>>,
    capture_hist: &Rc<RefCell<Vec<f32>>>,
    mic_hist: &Rc<RefCell<Vec<f32>>>,
    render_hist: &Rc<RefCell<Vec<f32>>>,
) -> Timer {
    let ui_weak = ui.as_weak();
    let engine_poll = engine.clone();
    let cache = cache.clone();
    let saved_mics = saved_mics.clone();
    let capture_hist = capture_hist.clone();
    let mic_hist = mic_hist.clone();
    let render_hist = render_hist.clone();
    let last_dest_idx = Cell::new(ui.get_device_index());
    let last_mic_idx = Cell::new(ui.get_mic_index());

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        match engine_poll.status() {
            EngineStatus::Running => {
                if !ui.get_running() {
                    ui.set_running(true);
                    ui.set_status_text("运行中".into());
                    ui.set_status_kind(1);
                }
            }
            EngineStatus::Idle => {
                if ui.get_running() {
                    restore_default_mics(&saved_mics);
                    ui.set_running(false);
                    ui.set_status_text("未运行".into());
                    ui.set_status_kind(0);
                    ui.set_error_text("".into());
                }
            }
            EngineStatus::ProcessExited => {
                restore_default_mics(&saved_mics);
                ui.set_running(false);
                ui.set_status_text("进程已退出".into());
                ui.set_status_kind(2);
                ui.set_error_text("进程已退出".into());
            }
            EngineStatus::DeviceGone => {
                restore_default_mics(&saved_mics);
                ui.set_running(false);
                ui.set_status_text("设备已断开".into());
                ui.set_status_kind(3);
                ui.set_error_text("无法连接到音频设备\n设备可能已经被拔出。".into());
            }
            EngineStatus::Error(msg) => {
                restore_default_mics(&saved_mics);
                ui.set_running(false);
                ui.set_status_text("错误".into());
                ui.set_status_kind(3);
                ui.set_error_text(msg.into());
            }
        }

        let running = engine_poll.is_running();
        let cap = if running { engine_poll.sample_capture_peak() } else { 0.0 };
        let mic = if running { engine_poll.sample_mic_peak() } else { 0.0 };
        let rend = if running { engine_poll.sample_render_peak() } else { 0.0 };
        push_bar(&mut capture_hist.borrow_mut(), cap);
        push_bar(&mut mic_hist.borrow_mut(), mic);
        push_bar(&mut render_hist.borrow_mut(), rend);
        set_wave(
            &ui,
            &capture_hist.borrow(),
            &mic_hist.borrow(),
            &render_hist.borrow(),
        );

        let idx = ui.get_device_index();
        if idx != last_dest_idx.get() {
            last_dest_idx.set(idx);
            update_output_text(&ui, &cache);
        }
        let mic_idx = ui.get_mic_index();
        if mic_idx != last_mic_idx.get() {
            last_mic_idx.set(mic_idx);
            update_mic_text(&ui, &cache);
        }
        ui.set_has_mic(mic_idx != 0);
    });
    timer
}

/// 重新枚举进程、麦克风、输出设备。`initial` 为真时自动挑一只虚拟麦。
fn refill(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>, initial: bool) {
    refill_processes(ui, cache);
    refill_mics(ui, cache);
    refill_devices(ui, cache, initial);
}

fn refill_processes(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>) {
    match process::list_processes() {
        Ok(list) => cache.borrow_mut().processes = list,
        Err(e) => {
            tracing::warn!("list processes: {e}");
            ui.set_error_text(e.user_message().into());
            return;
        }
    }
    apply_process_filter(ui, cache);
}

/// 按关键字重建进程下拉，尽量保住当前选中的 PID。
fn apply_process_filter(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>) {
    let c = cache.borrow();
    let kept = c.pids.get(ui.get_process_index() as usize).copied();
    let needle = c.filter.trim().to_ascii_lowercase();
    let mut labels: Vec<slint::SharedString> = Vec::new();
    let mut pids: Vec<u32> = Vec::new();
    for p in &c.processes {
        if !needle.is_empty() && !process_matches(p, &needle) {
            continue;
        }
        labels.push(p.label().into());
        pids.push(p.pid);
    }
    let idx = kept
        .and_then(|pid| pids.iter().position(|v| *v == pid))
        .unwrap_or(0);
    drop(c);

    {
        let mut c = cache.borrow_mut();
        c.pids = pids;
    }
    ui.set_processes(ModelRc::new(VecModel::from(labels)));
    ui.set_process_index(idx as i32);
    ui.set_about_text(about_text(cache).into());
}

/// 进程名或窗口标题命中关键字。
fn process_matches(p: &ProcessInfo, needle: &str) -> bool {
    p.name.to_ascii_lowercase().contains(needle)
        || p.title
            .as_deref()
            .is_some_and(|t| t.to_ascii_lowercase().contains(needle))
}

fn refill_mics(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>) {
    let kept = {
        let c = cache.borrow();
        c.mic_ids.get(ui.get_mic_index() as usize).cloned()
    };
    let mut labels: Vec<slint::SharedString> = vec!["不混入麦克风".into()];
    let mut ids: Vec<String> = vec![String::new()];
    let mut names: Vec<String> = vec![String::new()];
    match device::list_physical_mics() {
        Ok(list) => {
            for m in list {
                labels.push(m.name.clone().into());
                names.push(m.name.clone());
                ids.push(m.id);
            }
        }
        Err(e) => tracing::warn!("list mics: {e}"),
    }
    let idx = kept
        .as_deref()
        .and_then(|id| ids.iter().position(|v| v == id))
        .unwrap_or(0);
    {
        let mut c = cache.borrow_mut();
        c.mic_ids = ids;
        c.mic_names = names;
    }

    ui.set_mics(ModelRc::new(VecModel::from(labels)));
    ui.set_mic_index(idx as i32);
    ui.set_has_mic(idx != 0);
    update_mic_text(ui, cache);
}

/// 把当前选中的麦克风名同步给路由页。
fn update_mic_text(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>) {
    let name = {
        let c = cache.borrow();
        c.mic_names
            .get(ui.get_mic_index() as usize)
            .cloned()
            .unwrap_or_default()
    };
    ui.set_mic_name(name.into());
}

fn refill_devices(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>, initial: bool) {
    let include_all = cache.borrow().all_devices;
    let kept_id = {
        let c = cache.borrow();
        c.dests
            .get(ui.get_device_index() as usize)
            .map(|d| d.render_id.clone())
    };
    match device::list_destinations(include_all) {
        Ok(list) => cache.borrow_mut().dests = list,
        Err(e) => {
            tracing::warn!("list devices: {e}");
            ui.set_error_text(e.user_message().into());
            return;
        }
    }

    let (labels, idx, cable, picked_is_mic) = {
        let c = cache.borrow();
        let labels: Vec<slint::SharedString> =
            c.dests.iter().map(|d| d.label.clone().into()).collect();
        let idx = kept_id
            .as_deref()
            .and_then(|id| c.dests.iter().position(|d| d.render_id == id))
            .or_else(|| {
                // 首次启动优先选虚拟麦克风：那是「送进游戏麦」最常用的目标。
                if initial {
                    c.dests.iter().position(|d| d.capture_id.is_some())
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let cable = c
            .dests
            .iter()
            .any(|d| vbcable::device_looks_like_cable(&d.label) || d.capture_id.is_some());
        let picked_is_mic = c.dests.get(idx).is_some_and(|d| d.capture_id.is_some());
        (labels, idx, cable, picked_is_mic)
    };

    ui.set_devices(ModelRc::new(VecModel::from(labels)));
    ui.set_device_index(idx as i32);
    ui.set_cable_installed(cable);
    if cable {
        ui.set_cable_text("VB-CABLE 已安装".into());
    } else if !ui.get_cable_busy() {
        ui.set_cable_text("未检测到 VB-CABLE".into());
    }
    if initial && kept_id.is_none() && picked_is_mic {
        cache.borrow_mut().set_default_mic = true;
        ui.set_set_default_mic(true);
    }
    update_output_text(ui, cache);
    ui.set_about_text(about_text(cache).into());
}

/// 按持久化的 ID / 名称恢复进程、输出设备与物理麦选择。
fn restore_saved_selection(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>, saved: &config::AppConfig) {
    {
        let c = cache.borrow();
        if let Some(name) = saved.route.process_name.as_deref()
            && let Some(pid) = c.processes.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| p.pid)
            && let Some(index) = c.pids.iter().position(|v| *v == pid)
        {
            ui.set_process_index(index as i32);
        }
        let device_index = saved.output.device_id.as_deref()
            .and_then(|id| c.dests.iter().position(|d| d.render_id == id))
            .or_else(|| saved.output.device_name.as_deref().and_then(|name| c.dests.iter().position(|d| d.label == name)));
        if let Some(index) = device_index {
            ui.set_device_index(index as i32);
        }
        if let Some(id) = saved.output.mic_id.as_deref()
            && let Some(index) = c.mic_ids.iter().position(|v| v == id)
        {
            ui.set_mic_index(index as i32);
        }
    }
    update_output_text(ui, cache);
    update_mic_text(ui, cache);
}

/// 退出前写回当前选择与混音值。
fn persist_config(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>, app_config: &Rc<RefCell<config::AppConfig>>) {
    let mut saved = app_config.borrow_mut();
    let c = cache.borrow();
    saved.output.all_devices = c.all_devices;
    saved.output.set_default_mic = c.set_default_mic;
    saved.output.device_id = c.dests.get(ui.get_device_index() as usize).map(|d| d.render_id.clone());
    saved.output.device_name = c.dests.get(ui.get_device_index() as usize).map(|d| d.label.clone());
    let mic_index = ui.get_mic_index() as usize;
    saved.output.mic_id = c.mic_ids.get(mic_index).filter(|v| !v.is_empty()).cloned();
    saved.output.mic_name = c.mic_names.get(mic_index).filter(|v| !v.is_empty()).cloned();
    let pid = c.pids.get(ui.get_process_index() as usize).copied();
    saved.route.process_name = pid.and_then(|pid| c.processes.iter().find(|p| p.pid == pid).map(|p| p.name.clone()));
    saved.mix.process = (ui.get_volume() / 100.0).clamp(0.0, 1.0);
    saved.mix.mic = (ui.get_mic_volume() / 100.0).clamp(0.0, 1.0);
    saved.mix.master = (ui.get_master_volume() / 100.0).clamp(0.0, 1.0);
    config::save(&saved);
}

/// 输出名称、说明短句、是否显示「设为默认麦」勾选。
fn update_output_text(ui: &MainWindow, cache: &Rc<RefCell<UiCache>>) {
    let c = cache.borrow();
    match c.dests.get(ui.get_device_index() as usize) {
        Some(d) => {
            ui.set_output_name(d.label.clone().into());
            ui.set_show_set_default_mic(d.capture_id.is_some());
            let hint = match d.capture_name.as_deref() {
                Some(name) => {
                    format!("虚拟麦克风「{name}」。游戏里把麦克风选成它，队友就能听到。")
                }
                None => "虚拟扬声器。声音播到这只设备。".to_string(),
            };
            ui.set_dest_hint(hint.into());
        }
        None => {
            ui.set_output_name("".into());
            ui.set_show_set_default_mic(false);
            ui.set_dest_hint("输出列表是虚拟扬声器与虚拟麦克风。".into());
        }
    }
}

fn about_text(cache: &Rc<RefCell<UiCache>>) -> String {
    let c = cache.borrow();
    format!(
        "版本 {} · 输出设备 {} 个 · 进程 {} 个",
        env!("CARGO_PKG_VERSION"),
        c.dests.len(),
        c.processes.len()
    )
}

/// 开始转发时按勾选切换系统默认麦，失败只提示，不影响转发。
fn apply_virtual_mic_route(
    ui: &MainWindow,
    dest: &DestDevice,
    set_default: bool,
    saved_mics: &Rc<RefCell<Vec<(u32, String)>>>,
) {
    let Some(cap_id) = dest.capture_id.as_deref() else {
        ui.set_dest_hint("当前输出是虚拟扬声器，声音播到这只设备。".into());
        return;
    };
    let name = dest.capture_name.as_deref().unwrap_or(dest.label.as_str());
    if !set_default {
        ui.set_dest_hint(format!("已混进「{name}」。").into());
        return;
    }
    *saved_mics.borrow_mut() = device::default_capture_ids();
    if let Err(e) = policy::set_default_capture(cap_id) {
        tracing::warn!("set default mic: {e}");
        ui.set_error_text(format!("已开始转发，但没能改系统默认麦。请手动选择「{name}」。").into());
        ui.set_dest_hint(format!("请把麦克风改成「{name}」，关掉噪音抑制后重启。").into());
        return;
    }
    ui.set_dest_hint(format!("已把系统默认麦改成「{name}」。").into());
}

fn restore_default_mics(saved: &Rc<RefCell<Vec<(u32, String)>>>) {
    let prev = saved.borrow_mut().split_off(0);
    if prev.is_empty() {
        return;
    }
    if let Err(e) = policy::restore_defaults(&prev) {
        tracing::warn!("restore default mic: {e}");
    }
}

fn push_bar(hist: &mut Vec<f32>, value: f32) {
    if hist.len() >= WAVE_BARS {
        hist.remove(0);
    }
    hist.push(value.clamp(0.0, 1.0));
}

fn set_wave(ui: &MainWindow, capture: &[f32], mic: &[f32], render: &[f32]) {
    ui.set_capture_wave(ModelRc::new(VecModel::from(capture.to_vec())));
    ui.set_mic_wave(ModelRc::new(VecModel::from(mic.to_vec())));
    ui.set_render_wave(ModelRc::new(VecModel::from(render.to_vec())));
}
