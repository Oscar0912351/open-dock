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
        atomic::Ordering,
        Arc, LazyLock,
    },
    time::{Duration, Instant},
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
/// Gen-afsend af aktuel tilstand med dette interval — webview'ens
/// `widget://hover`-lytter kan registreres efter første emit (app-start
/// eller ny widget), og vi emitter kun ved ændring.
const REEMIT: Duration = Duration::from_secs(2);

static WAKE: LazyLock<Arc<Notify>> = LazyLock::new(|| Arc::new(Notify::new()));

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

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Sidste kendte inside-tilstand pr. widget-id; glemmes når widgetten
        // fjernes, funktionen slås fra, eller edit-mode tændes. `last_emit`
        // styrer gen-afsend af uændret tilstand (se REEMIT).
        let mut inside: HashMap<Uuid, bool> = HashMap::new();
        let mut last_emit: HashMap<Uuid, Instant> = HashMap::new();
        loop {
            let state = app.state::<AppState>();
            let targets: Vec<(Uuid, String)> = if state.edit_mode.load(Ordering::Acquire) {
                Vec::new()
            } else {
                state
                    .config
                    .get()
                    .widgets
                    .iter()
                    .filter(|w| w.reveal_on_hover)
                    .map(|w| (w.id, w.window_label()))
                    .collect()
            };
            inside.retain(|id, _| targets.iter().any(|(target, _)| *target == *id));
            last_emit.retain(|id, _| targets.iter().any(|(target, _)| *target == *id));

            if targets.is_empty() {
                WAKE.notified().await;
                continue;
            }

            let mut cursor = POINT::default();
            let cursor_ok = unsafe { GetCursorPos(&mut cursor) }.is_ok();
            let now = Instant::now();
            for (id, label) in &targets {
                let now_inside = cursor_ok
                    && widget_rect(&app, label).is_some_and(|rect| {
                        cursor.x >= rect.left - HOVER_MARGIN_PX
                            && cursor.x <= rect.right + HOVER_MARGIN_PX
                            && cursor.y >= rect.top - HOVER_MARGIN_PX
                            && cursor.y <= rect.bottom + HOVER_MARGIN_PX
                    });
                let changed = inside.insert(*id, now_inside) != Some(now_inside);
                let due = last_emit
                    .get(id)
                    .map_or(true, |at| now.saturating_duration_since(*at) >= REEMIT);
                if changed || due {
                    let _ = app.emit_to(
                        label,
                        "widget://hover",
                        WidgetHover { inside: now_inside },
                    );
                    last_emit.insert(*id, now);
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(POLL_MS)) => {}
                _ = WAKE.notified() => {}
            }
        }
    });
}
