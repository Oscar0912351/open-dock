//! "Vis kun når musen er over" — poller markørpositionen hvert 100 ms og
//! emitterer `widget://hover` til de widget-vinduer der har `reveal_on_hover`.
//!
//! Hvorfor polling og ikke mus-events: en skjult widget er klik-igennem
//! (WS_EX_TRANSPARENT), så WebView2 får aldrig mouse-move-events. Derfor
//! må vi fra Rust-siden spørge Win32 om hvor markøren er i forhold til
//! vinduet.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;
use ts_rs::TS;
use uuid::Uuid;
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect},
};

use crate::state::AppState;

const POLL_MS: u64 = 100;
/// Ekstra margin omkring widget-rektanglet, i fysiske pixels.
const HOVER_MARGIN_PX: i32 = 8;

static WAKE: LazyLock<Arc<Notify>> = LazyLock::new(|| Arc::new(Notify::new()));
static STOPPED: AtomicBool = AtomicBool::new(false);

/// Vækker hover-tasken — kaldes ved config-ændringer og edit-mode-skift.
pub fn poke() {
    WAKE.notify_one();
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WidgetHover {
    pub inside: bool,
}

fn widget_rect(app: &AppHandle, label: &str) -> Option<RECT> {
    let window = app.get_webview_window(label)?;
    let hwnd = HWND(window.hwnd().ok()?.0 as _);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(rect)
}

pub fn is_cursor_inside_widget(app: &AppHandle, label: &str) -> bool {
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return false;
    }
    widget_rect(app, label).is_some_and(|rect| {
        cursor.x >= rect.left - HOVER_MARGIN_PX
            && cursor.x <= rect.right + HOVER_MARGIN_PX
            && cursor.y >= rect.top - HOVER_MARGIN_PX
            && cursor.y <= rect.bottom + HOVER_MARGIN_PX
    })
}

pub fn shutdown() {
    STOPPED.store(true, Ordering::Release);
    WAKE.notify_one();
}

pub fn start(app: AppHandle) {
    STOPPED.store(false, Ordering::Release);
    tauri::async_runtime::spawn(async move {
        // Sidste kendte inside-tilstand pr. widget-id; glemmes når widgetten
        // fjernes, funktionen slås fra, eller edit-mode tændes.
        let mut inside: HashMap<Uuid, bool> = HashMap::new();
        loop {
            if STOPPED.load(Ordering::Acquire) {
                break;
            }
            let state = app.state::<AppState>();
            let targets: Vec<(Uuid, String)> = if state.edit_mode.load(Ordering::Acquire) {
                Vec::new()
            } else {
                state
                    .config
                    .read_with(|cfg| {
                        cfg.widgets
                            .iter()
                            .filter(|w| {
                                w.reveal_on_hover
                                    || w.cluster.as_ref().is_some_and(|c| {
                                        c.modules.iter().any(|m| {
                                            m.auto_rule
                                                == crate::config::schema::AutoRule::ShowOnHover
                                        })
                                    })
                            })
                            .map(|w| (w.id, w.window_label()))
                            .collect()
                    })
            };
            inside.retain(|id, _| targets.iter().any(|(target, _)| *target == *id));

            if targets.is_empty() {
                WAKE.notified().await;
                continue;
            }

            let mut cursor = POINT::default();
            let cursor_ok = unsafe { GetCursorPos(&mut cursor) }.is_ok();
            for (id, label) in &targets {
                let now_inside = cursor_ok
                    && widget_rect(&app, label).is_some_and(|rect| {
                        cursor.x >= rect.left - HOVER_MARGIN_PX
                            && cursor.x <= rect.right + HOVER_MARGIN_PX
                            && cursor.y >= rect.top - HOVER_MARGIN_PX
                            && cursor.y <= rect.bottom + HOVER_MARGIN_PX
                    });
                if inside.insert(*id, now_inside) != Some(now_inside) {
                    let _ = app.emit_to(
                        label,
                        "widget://hover",
                        WidgetHover { inside: now_inside },
                    );
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(POLL_MS)) => {}
                _ = WAKE.notified() => {}
            }
        }
    });
}
