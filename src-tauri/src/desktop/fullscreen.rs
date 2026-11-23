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
    Foundation::{HWND, RECT},
    Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST},
    System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
    UI::{
        Shell::{
            SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        },
        WindowsAndMessaging::{GetWindowRect, ShowWindow, SW_HIDE},
    },
};

use crate::{
    providers::scheduler::Scheduler,
    state::{AppPaused, AppState, PauseReason},
    windows::widget,
};

use super::{showdesktop, zorder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullscreenTarget {
    pub monitor: isize,
    pub rect: Option<(i32, i32, i32, i32)>,
}

fn rects_intersect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.2 && a.2 > b.0 && a.1 < b.3 && a.3 > b.1
}

pub fn hwnd_covered_by_fullscreen(hwnd: HWND, target: Option<FullscreenTarget>) -> bool {
    let Some(target) = target else {
        return false;
    };
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }.0 as isize;
    if mon != 0 && mon == target.monitor {
        return true;
    }
    if let Some(fg_rect) = target.rect {
        let mut w_rect = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut w_rect) }.is_ok() {
            return rects_intersect(
                fg_rect,
                (w_rect.left, w_rect.top, w_rect.right, w_rect.bottom),
            );
        }
    }
    false
}

static CHECK_NOTIFY: LazyLock<Arc<Notify>> = LazyLock::new(|| Arc::new(Notify::new()));

pub fn trigger_check() {
    CHECK_NOTIFY.notify_one();
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let notify = Arc::clone(&CHECK_NOTIFY);
        let mut last_auto: Option<PauseReason> = None;
        let mut last_fullscreen = false;
        let mut last_target: Option<FullscreenTarget> = None;
        let mut last_overlay_target: Option<FullscreenTarget> = None;
        let mut last_hide_top_target: Option<FullscreenTarget> = None;

        loop {
            let sleep_dur = if last_overlay_target.is_some() {
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

            let current_target = if is_fullscreen {
                showdesktop::foreground_monitor_and_rect().map(|(monitor, rect)| FullscreenTarget {
                    monitor,
                    rect: rect.map(|r| (r.left, r.top, r.right, r.bottom)),
                })
            } else {
                None
            };

            if current_target != last_target {
                last_target = current_target;
                *state.fullscreen_target.write() = current_target;
            }

            if is_fullscreen != last_fullscreen {
                last_fullscreen = is_fullscreen;
                state.fullscreen_active.store(is_fullscreen, Ordering::Release);
                crate::wallpaper::notify_pause_changed(&app);
            }

            let next_overlay = general.game_overlay && is_fullscreen && !edit_mode;
            let overlay_target = if next_overlay { current_target } else { None };
            if overlay_target != last_overlay_target {
                last_overlay_target = overlay_target;
                state
                    .game_overlay_active
                    .store(overlay_target.is_some(), Ordering::Release);
                widget::apply_game_overlay(&app, overlay_target);
            } else if overlay_target.is_some() {
                zorder::reassert_game_overlay();
            }

            let pause_for_fullscreen =
                !general.game_overlay && general.pause_on_fullscreen && is_fullscreen;
            let hide_top_target = if pause_for_fullscreen {
                current_target
            } else {
                None
            };
            if hide_top_target != last_hide_top_target {
                last_hide_top_target = hide_top_target;
                sync_top_widgets(&app, hide_top_target);
            }

            let battery = general.pause_on_battery && on_battery();

            let next_auto = if pause_for_fullscreen {
                Some(PauseReason::Fullscreen)
            } else if battery {
                Some(PauseReason::Battery)
            } else {
                None
            };

            if next_auto != last_auto {
                last_auto = next_auto.clone();
                *state.auto_pause_reason.write() = next_auto;
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

fn sync_top_widgets(app: &AppHandle, target: Option<FullscreenTarget>) {
    let visible = app.state::<AppState>().widgets_visible.load(Ordering::Acquire);
    for (hwnd, label) in zorder::top_widgets() {
        let Some(window) = app.get_webview_window(&label) else {
            log::warn!("Top-widgetten {label} findes ikke længere");
            continue;
        };
        let covered = hwnd_covered_by_fullscreen(hwnd, target);
        zorder::set_fullscreen_hidden(hwnd, covered);
        if covered || !visible {
            // Tauris `hide()` er en no-op på de subklassede widget-vinduer.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            crate::windows::webview_opt::set_webview_suspended(&window, true);
            continue;
        }
        crate::windows::webview_opt::set_webview_suspended(&window, false);
        if let Err(error) = zorder::show_without_activation(hwnd) {
            log::error!("Kunne ikke genanvende laget på {label}: {error}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rects_intersect_distinguishes_same_and_adjacent_monitors() {
        let primary = (0, 0, 1920, 1080);
        let widget_on_primary = (20, 20, 284, 340);
        let widget_on_second = (1940, 20, 2204, 340);
        let spanned = (0, 0, 3840, 1080);

        assert!(rects_intersect(primary, widget_on_primary));
        assert!(!rects_intersect(primary, widget_on_second));
        assert!(rects_intersect(spanned, widget_on_second));
        // Touching edge without overlap is not an intersection
        assert!(!rects_intersect(primary, (1920, 0, 2184, 320)));
    }
}
