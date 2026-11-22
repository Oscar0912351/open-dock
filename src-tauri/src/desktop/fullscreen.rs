use std::{
    sync::{
        atomic::Ordering,
        Arc, LazyLock,
    },
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;
use windows::Win32::{
    System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
    UI::Shell::{
        SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    },
};

use crate::{
    providers::scheduler::Scheduler,
    state::{AppPaused, AppState, PauseReason},
    windows::widget,
};

use super::{showdesktop, zorder};

static CHECK_NOTIFY: LazyLock<Arc<Notify>> = LazyLock::new(|| Arc::new(Notify::new()));

pub fn trigger_check() {
    CHECK_NOTIFY.notify_one();
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let notify = Arc::clone(&CHECK_NOTIFY);
        let mut last_auto: Option<PauseReason> = None;
        let mut last_fullscreen = false;
        let mut last_overlay = false;

        loop {
            let sleep_dur = if last_overlay {
                Duration::from_secs(2)
            } else {
                Duration::from_secs(5)
            };
            tokio::select! {
                _ = tokio::time::sleep(sleep_dur) => {}
                _ = notify.notified() => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
            }

            let state = app.state::<AppState>();
            let general = state.config.get().general;
            let edit_mode = state.edit_mode.load(Ordering::Acquire);
            let has_live_wallpaper = state.wallpaper_status.read().active_surfaces > 0;
            let need_fullscreen_check =
                general.game_overlay || general.pause_on_fullscreen || has_live_wallpaper;

            let is_fullscreen = if need_fullscreen_check {
                !showdesktop::foreground_is_desktop()
                    && !showdesktop::foreground_is_shell_or_own()
                    && (shell_reports_fullscreen()
                        || showdesktop::foreground_is_borderless_fullscreen())
            } else {
                false
            };

            if is_fullscreen != last_fullscreen {
                last_fullscreen = is_fullscreen;
                state.fullscreen_active.store(is_fullscreen, Ordering::Release);
                crate::wallpaper::notify_pause_changed(&app);
            }

            let next_overlay = general.game_overlay && is_fullscreen && !edit_mode;
            if next_overlay != last_overlay {
                last_overlay = next_overlay;
                state.game_overlay_active.store(next_overlay, Ordering::Release);
                widget::apply_game_overlay(&app, next_overlay);
            } else if next_overlay {
                zorder::reassert_game_overlay();
            }

            let pause_for_fullscreen =
                !general.game_overlay && general.pause_on_fullscreen && is_fullscreen;
            let battery = general.pause_on_battery && on_battery();

            let next_auto = if pause_for_fullscreen {
                Some(PauseReason::Fullscreen)
            } else if battery {
                Some(PauseReason::Battery)
            } else {
                None
            };

            if next_auto != last_auto {
                let was_fullscreen = last_auto == Some(PauseReason::Fullscreen);
                let now_fullscreen = next_auto == Some(PauseReason::Fullscreen);
                last_auto = next_auto.clone();
                *state.auto_pause_reason.write() = next_auto;
                if was_fullscreen != now_fullscreen {
                    sync_top_widgets(&app, now_fullscreen);
                }
                recompute_pause(&app);
            }
        }
    });
}

fn on_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    let ok = unsafe { GetSystemPowerStatus(&mut status) }.is_ok();
    ok && status.ACLineStatus == 0
}

fn shell_reports_fullscreen() -> bool {
    let result = unsafe { SHQueryUserNotificationState() };
    match result {
        // Bemærk: QUNS_BUSY udelades bevidst — Windows returnerer QUNS_BUSY ved
        // proceslinje-pil (^), Start-menu og system-popups. Borderless-fuldskærm
        // fanges i stedet præcist af `foreground_is_borderless_fullscreen()`.
        Ok(state) => matches!(
            state,
            QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
        ),
        Err(error) => {
            log::warn!("SHQueryUserNotificationState fejlede: {error}");
            false
        }
    }
}

fn sync_top_widgets(app: &AppHandle, fullscreen: bool) {
    let visible = app.state::<AppState>().widgets_visible.load(Ordering::Acquire);
    for label in zorder::top_widget_labels() {
        let Some(window) = app.get_webview_window(&label) else {
            log::warn!("Top-widgetten {label} findes ikke længere");
            continue;
        };
        if fullscreen || !visible {
            // Tauris `hide()` er en no-op på de subklassede widget-vinduer.
            if let Ok(raw) = window.hwnd() {
                let hwnd = windows::Win32::Foundation::HWND(raw.0 as _);
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                        hwnd,
                        windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
                    );
                }
            }
            crate::windows::webview_opt::set_webview_suspended(&window, true);
            continue;
        }
        crate::windows::webview_opt::set_webview_suspended(&window, false);
        match window.hwnd() {
            Ok(raw) => {
                if let Err(error) =
                    zorder::show_without_activation(windows::Win32::Foundation::HWND(raw.0 as _))
                {
                    log::error!("Kunne ikke genanvende laget på {label}: {error}");
                }
            }
            Err(error) => log::error!("Kunne ikke læse HWND for {label}: {error}"),
        }
    }
}

pub fn recompute_pause(app: &AppHandle) {
    let state = app.state::<AppState>();
    let visible = state.widgets_visible.load(Ordering::Acquire);
    let manual = state.manual_paused.load(Ordering::Acquire);
    let auto_reason = state.auto_pause_reason.read().clone();

    let ui_reason = if manual {
        Some(PauseReason::User)
    } else {
        auto_reason
    };
    let ui_paused = ui_reason.is_some();
    let scheduler_paused = ui_paused || !visible;

    state.paused.store(ui_paused, Ordering::Release);
    if let Some(scheduler) = app.try_state::<Scheduler>() {
        scheduler.set_paused(scheduler_paused);
    }

    let payload = AppPaused {
        paused: ui_paused,
        reason: ui_reason.clone(),
    };
    let changed = {
        let mut guard = state.effective_pause.write();
        if guard.paused != payload.paused || guard.reason != payload.reason {
            *guard = payload.clone();
            true
        } else {
            false
        }
    };

    if changed {
        if payload.paused {
            log::info!("Pause aktiveret ({:?}); providers sættes på pause", payload.reason);
        } else {
            log::info!("Pause afsluttet; providers genoptager");
        }
        if let Err(error) = app.emit("app://paused", &payload) {
            log::error!("Kunne ikke sende app://paused: {error}");
        }
        crate::wallpaper::notify_pause_changed(app);
        crate::tray::sync(app);
    }
}
