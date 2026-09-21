//! Slint 桥：混音、浅色 UI、快捷键。

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
use crate::process;
use crate::vbcable;

slint::include_modules!();

/// 声纹柱数量。
const WAVE_BARS: usize = 48;

/// 创建窗口、填列表、跑事件循环。退出时 stop 引擎。
pub fn run_ui() -> Result<(), Error> {
    let ui = MainWindow::new().map_err(|e| Error::InvalidArgs(e.to_string()))?;
    let engine = Rc::new(AudioEngine::new());
    let pids = Rc::new(RefCell::new(Vec::<u32>::new()));
    let dests = Rc::new(RefCell::new(Vec::<DestDevice>::new()));
    let mic_ids = Rc::new(RefCell::new(Vec::<String>::new()));
    let saved_mics = Rc::new(RefCell::new(Vec::<(u32, String)>::new()));
    let capture_hist = Rc::new(RefCell::new(vec![0.0f32; WAVE_BARS]));
    let mic_hist = Rc::new(RefCell::new(vec![0.0f32; WAVE_BARS]));
    let render_hist = Rc::new(RefCell::new(vec![0.0f32; WAVE_BARS]));
    let hotkey = Rc::new(RefCell::new(config::load()));
    ui.set_hotkey_label(hotkey.borrow().label().into());
    fill_lists(&ui, &pids, &dests, &mic_ids);
    select_virtual_mic(&ui, &dests);
    set_wave(
        &ui,
        &capture_hist.borrow(),
        &mic_hist.borrow(),
        &render_hist.borrow(),
    );

    {
        let ui_weak = ui.as_weak();
        let pids = pids.clone();
        let dests = dests.clone();
        let mic_ids = mic_ids.clone();
        ui.on_refresh(move || {
            if let Some(ui) = ui_weak.upgrade() {
                fill_lists(&ui, &pids, &dests, &mic_ids);
            }
        });
    }

    {
        let engine = engine.clone();
        ui.on_volume_edited(move |v| {
            engine.set_volume((v / 100.0).clamp(0.0, 1.0));
        });
    }
    {
        let engine = engine.clone();
        ui.on_mic_volume_edited(move |v| {
            engine.set_mic_volume((v / 100.0).clamp(0.0, 1.0));
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_install_cable(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
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
                    let Some(ui) = ui_weak.upgrade() else {
                        return;
                    };
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

    let hotkey_server = Rc::new({
        let ui_weak = ui.as_weak();
        HotkeyServer::start(*hotkey.borrow(), Arc::new(move || {
            let ui_weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_start_stop();
                }
            });
        }))
    });

    {
        let ui_weak = ui.as_weak();
        let hotkey = hotkey.clone();
        let packed = hotkey_server.clone();
        ui.on_hotkey_captured(move |text, ctrl, shift, alt| {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            ui.set_listening_hotkey(false);
            let Some(vk) = config::slint_key_to_vk(text.as_str()) else {
                return;
            };
            let mut mods = 0u32;
            if alt {
                mods |= 1;
            }
            if ctrl {
                mods |= 2;
            }
            if shift {
                mods |= 4;
            }
            let hk = config::Hotkey { mods, vk };
            config::save(hk);
            *hotkey.borrow_mut() = hk;
            packed.set(hk);
            ui.set_hotkey_label(hk.label().into());
        });
    }

    {
        let ui_weak = ui.as_weak();
        let hotkey = hotkey.clone();
        let packed = hotkey_server.clone();
        ui.on_reset_hotkey(move || {
            let hk = config::Hotkey::default();
            config::save(hk);
            *hotkey.borrow_mut() = hk;
            packed.set(hk);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_listening_hotkey(false);
                ui.set_hotkey_label(hk.label().into());
            }
        });
    }
    let _hotkey_server = hotkey_server;
    {
        let ui_weak = ui.as_weak();
        let engine = engine.clone();
        let pids = pids.clone();
        let dests = dests.clone();
        let mic_ids = mic_ids.clone();
        let saved_mics = saved_mics.clone();
        ui.on_start_stop(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };
            if engine.is_running() {
                engine.stop();
                restore_default_mics(&saved_mics);
                ui.set_running(false);
                ui.set_status_text("未运行".into());
                ui.set_error_text("".into());
                return;
            }
            let pid = pids.borrow().get(ui.get_process_index() as usize).copied();
            let dest = dests.borrow().get(ui.get_device_index() as usize).cloned();
            let (Some(pid), Some(dest)) = (pid, dest) else {
                ui.set_error_text("请选择进程和输出设备".into());
                return;
            };
            let mic_idx = ui.get_mic_index() as usize;
            let mic_id = mic_ids
                .borrow()
                .get(mic_idx)
                .filter(|s| !s.is_empty())
                .cloned();
            engine.set_volume((ui.get_volume() / 100.0).clamp(0.0, 1.0));
            engine.set_mic_volume((ui.get_mic_volume() / 100.0).clamp(0.0, 1.0));
            match engine.start(pid, &dest.render_id, mic_id.as_deref()) {
                Ok(()) => {
                    ui.set_running(true);
                    ui.set_status_text("运行中".into());
                    ui.set_error_text("".into());
                    apply_virtual_mic_route(&ui, &dest, &saved_mics);
                }
                Err(e) => {
                    tracing::warn!("start failed: {e}");
                    ui.set_running(false);
                    ui.set_status_text("错误".into());
                    ui.set_error_text(e.user_message().into());
                }
            }
        });
    }

    let ui_weak = ui.as_weak();
    let engine_poll = engine.clone();
    let capture_hist_t = capture_hist.clone();
    let mic_hist_t = mic_hist.clone();
    let render_hist_t = render_hist.clone();
    let dests_t = dests.clone();
    let saved_mics_t = saved_mics.clone();
    let last_dest_idx = Cell::new(ui.get_device_index());
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        match engine_poll.status() {
            EngineStatus::Running => {
                if !ui.get_running() {
                    ui.set_running(true);
                    ui.set_status_text("运行中".into());
                }
            }
            EngineStatus::Idle => {
                if ui.get_running() {
                    restore_default_mics(&saved_mics_t);
                    ui.set_running(false);
                    ui.set_status_text("未运行".into());
                }
            }
            EngineStatus::ProcessExited => {
                restore_default_mics(&saved_mics_t);
                ui.set_running(false);
                ui.set_status_text("进程已退出".into());
                ui.set_error_text("进程已退出".into());
            }
            EngineStatus::DeviceGone => {
                restore_default_mics(&saved_mics_t);
                ui.set_running(false);
                ui.set_status_text("设备已断开".into());
                ui.set_error_text("无法连接到音频设备\n设备可能已经被拔出。".into());
            }
            EngineStatus::Error(msg) => {
                restore_default_mics(&saved_mics_t);
                ui.set_running(false);
                ui.set_status_text("错误".into());
                ui.set_error_text(msg.into());
            }
        }
        let cap = if engine_poll.is_running() {
            engine_poll.sample_capture_peak()
        } else {
            0.0
        };
        let mic = if engine_poll.is_running() {
            engine_poll.sample_mic_peak()
        } else {
            0.0
        };
        let rend = if engine_poll.is_running() {
            engine_poll.sample_render_peak()
        } else {
            0.0
        };
        ui.set_capture_level(cap);
        ui.set_mic_level(mic);
        ui.set_render_level(rend);
        push_bar(&mut capture_hist_t.borrow_mut(), cap);
        push_bar(&mut mic_hist_t.borrow_mut(), mic);
        push_bar(&mut render_hist_t.borrow_mut(), rend);
        set_wave(
            &ui,
            &capture_hist_t.borrow(),
            &mic_hist_t.borrow(),
            &render_hist_t.borrow(),
        );
        let idx = ui.get_device_index();
        if idx != last_dest_idx.get() {
            last_dest_idx.set(idx);
            if !ui.get_running() {
                update_dest_hint(&ui, &dests_t, true);
            }
        }
    });

    ui.run()
        .map_err(|e| Error::InvalidArgs(e.to_string()))?;
    restore_default_mics(&saved_mics);
    engine.stop();
    Ok(())
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

fn fill_lists(
    ui: &MainWindow,
    pids: &Rc<RefCell<Vec<u32>>>,
    dests: &Rc<RefCell<Vec<DestDevice>>>,
    mic_ids: &Rc<RefCell<Vec<String>>>,
) {
    match process::list_processes() {
        Ok(list) => {
            let labels: Vec<slint::SharedString> = list.iter().map(|p| p.label().into()).collect();
            *pids.borrow_mut() = list.iter().map(|p| p.pid).collect();
            ui.set_processes(ModelRc::new(VecModel::from(labels)));
            if ui.get_process_index() as usize >= pids.borrow().len() {
                ui.set_process_index(0);
            }
        }
        Err(e) => {
            tracing::warn!("list processes: {e}");
            ui.set_error_text(e.user_message().into());
        }
    }

    match device::list_physical_mics() {
        Ok(list) => {
            let mut labels = vec![slint::SharedString::from("不混入麦克风")];
            let mut ids = vec![String::new()];
            for m in list {
                labels.push(m.name.clone().into());
                ids.push(m.id);
            }
            *mic_ids.borrow_mut() = ids;
            ui.set_mics(ModelRc::new(VecModel::from(labels)));
            if ui.get_mic_index() as usize >= mic_ids.borrow().len() {
                ui.set_mic_index(0);
            }
        }
        Err(e) => tracing::warn!("list mics: {e}"),
    }

    match device::list_destinations() {
        Ok(list) => {
            let labels: Vec<slint::SharedString> =
                list.iter().map(|d| d.label.clone().into()).collect();
            let cable = list
                .iter()
                .any(|d| vbcable::device_looks_like_cable(&d.label) || d.capture_id.is_some());
            ui.set_cable_installed(cable);
            if cable {
                ui.set_cable_text("VB-CABLE 已安装".into());
            } else if !ui.get_cable_busy() {
                ui.set_cable_text("未检测到 VB-CABLE".into());
            }
            *dests.borrow_mut() = list;
            ui.set_devices(ModelRc::new(VecModel::from(labels)));
            if ui.get_device_index() as usize >= dests.borrow().len() {
                ui.set_device_index(0);
            }
            update_dest_hint(ui, dests, false);
        }
        Err(e) => {
            tracing::warn!("list devices: {e}");
            ui.set_error_text(e.user_message().into());
        }
    }
}

fn select_virtual_mic(ui: &MainWindow, dests: &Rc<RefCell<Vec<DestDevice>>>) {
    if let Some(i) = dests.borrow().iter().position(|d| d.capture_id.is_some()) {
        ui.set_device_index(i as i32);
        ui.set_set_default_mic(true);
        update_dest_hint(ui, dests, true);
    }
}

fn update_dest_hint(ui: &MainWindow, dests: &Rc<RefCell<Vec<DestDevice>>>, check_default: bool) {
    let dest = dests.borrow().get(ui.get_device_index() as usize).cloned();
    match dest {
        Some(d) if d.capture_id.is_some() => {
            ui.set_show_set_default_mic(true);
            if check_default {
                ui.set_set_default_mic(true);
            }
            let name = d.capture_name.as_deref().unwrap_or(d.label.as_str());
            ui.set_dest_hint(
                format!("虚拟麦克风「{name}」。")
                    .into(),
            );
        }
        Some(_) => {
            ui.set_show_set_default_mic(false);
            ui.set_dest_hint("虚拟扬声器。声音播到这只设备。".into());
        }
        None => {
            ui.set_show_set_default_mic(false);
            ui.set_dest_hint("输出列表是虚拟扬声器和虚拟麦克风。".into());
        }
    }
}

fn apply_virtual_mic_route(
    ui: &MainWindow,
    dest: &DestDevice,
    saved_mics: &Rc<RefCell<Vec<(u32, String)>>>,
) {
    let Some(cap_id) = dest.capture_id.as_deref() else {
        ui.set_dest_hint("请选一只虚拟麦克风。".into());
        return;
    };
    let name = dest.capture_name.as_deref().unwrap_or(dest.label.as_str());
    if !ui.get_set_default_mic() {
        ui.set_dest_hint(
            format!(
                "已混进「{name}」。"
            )
            .into(),
        );
        return;
    }
    *saved_mics.borrow_mut() = device::default_capture_ids();
    if let Err(e) = policy::set_default_capture(cap_id) {
        tracing::warn!("set default mic: {e}");
        ui.set_error_text(
            format!("已开始转发，但没能改系统默认麦。请在手动选择「{name}」。")
                .into(),
        );
        ui.set_dest_hint(
            format!("请把麦克风改成「{name}」，关掉噪音抑制后重启。").into(),
        );
        return;
    }
    ui.set_dest_hint(
        format!(
            "已把系统默认麦改成「{name}」。"
        )
        .into(),
    );
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
