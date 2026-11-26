use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
use sysinfo::System;
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    config::{
        schema::{Appearance, GlassEngine, WidgetInstance},
        store::ConfigStore,
    },
    desktop::monitors::{MonitorEntry, MonitorInfo, WidgetPlacement},
};

const MAX_REMOVED_HISTORY: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum PauseReason {
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AppPaused {
    pub paused: bool,
    pub reason: Option<PauseReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct EditModeState {
    pub on: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct EditSelected {
    pub id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct EditDragging {
    pub id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WidgetGlassState {
    pub appearance: Appearance,
    pub effective_engine: GlassEngine,
    pub live_backdrop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AppStats {
    pub cpu_pct: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AppVisible {
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct GameOverlayState {
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct AppStatus {
    pub visible: bool,
    pub paused: bool,
    pub reason: Option<PauseReason>,
    pub edit_mode: bool,
    pub is_windows_11: bool,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WidgetBoot {
    pub instance: WidgetInstance,
    pub appearance: Appearance,
    pub effective_engine: GlassEngine,
    pub live_backdrop: bool,
    pub monitor: Option<MonitorInfo>,
    pub placement: Option<WidgetPlacement>,
    pub start_delay_ms: u32,
    pub edit_mode: bool,
    pub hover_inside: bool,
    pub paused: bool,
    pub game_overlay_active: bool,
}

pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub paused: AtomicBool,
    pub widgets_visible: AtomicBool,
    pub manual_paused: AtomicBool,
    pub fullscreen_active: AtomicBool,
    pub fullscreen_target: RwLock<Option<crate::desktop::fullscreen::FullscreenTarget>>,
    pub game_overlay_active: AtomicBool,
    pub effective_pause: RwLock<AppPaused>,
    pub tray_notice_shown: AtomicBool,
    pub quitting: AtomicBool,
    pub edit_mode: AtomicBool,
    pub dragging: AtomicBool,
    pub selected_widget: RwLock<Option<Uuid>>,
    pub monitors: RwLock<Vec<MonitorEntry>>,
    pub start_delays: RwLock<HashMap<Uuid, u32>>,
    pub cluster_sizes: RwLock<HashMap<Uuid, (f64, f64)>>,
    pub removed_history: Mutex<VecDeque<WidgetInstance>>,
    pub process_monitor: Mutex<Option<System>>,
    pub live_plate_overrides: RwLock<HashMap<String, PathBuf>>,
    pub wallpaper_status: RwLock<crate::wallpaper::engine::WallpaperEngineStatus>,
}

impl AppState {
    pub fn new(config: Arc<ConfigStore>, monitors: Vec<MonitorEntry>) -> Self {
        Self {
            config,
            paused: AtomicBool::new(false),
            widgets_visible: AtomicBool::new(true),
            manual_paused: AtomicBool::new(false),
            fullscreen_active: AtomicBool::new(false),
            fullscreen_target: RwLock::new(None),
            game_overlay_active: AtomicBool::new(false),
            effective_pause: RwLock::new(AppPaused {
                paused: false,
                reason: None,
            }),
            tray_notice_shown: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            edit_mode: AtomicBool::new(false),
            dragging: AtomicBool::new(false),
            selected_widget: RwLock::new(None),
            monitors: RwLock::new(monitors),
            start_delays: RwLock::new(HashMap::new()),
            cluster_sizes: RwLock::new(HashMap::new()),
            removed_history: Mutex::new(VecDeque::with_capacity(MAX_REMOVED_HISTORY)),
            process_monitor: Mutex::new(None),
            live_plate_overrides: RwLock::new(HashMap::new()),
            wallpaper_status: RwLock::new(
                crate::wallpaper::engine::WallpaperEngineStatus::default(),
            ),
        }
    }

    pub fn push_removed(&self, instance: WidgetInstance) {
        let mut history = self.removed_history.lock();
        if history.len() >= MAX_REMOVED_HISTORY {
            history.pop_front();
        }
        history.push_back(instance);
    }

    pub fn pop_removed(&self, target_id: Option<Uuid>) -> Option<WidgetInstance> {
        let mut history = self.removed_history.lock();
        match target_id {
            Some(id) => {
                if let Some(pos) = history.iter().rposition(|item| item.id == id) {
                    history.remove(pos)
                } else {
                    None
                }
            }
            None => history.pop_back(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_ts_types_have_export_attribute() {
        assert!(PauseReason::output_path().is_some());
        assert!(AppPaused::output_path().is_some());
        assert!(EditModeState::output_path().is_some());
        assert!(EditSelected::output_path().is_some());
        assert!(EditDragging::output_path().is_some());
        assert!(WidgetGlassState::output_path().is_some());
        assert!(AppStats::output_path().is_some());
        assert!(AppVisible::output_path().is_some());
        assert!(GameOverlayState::output_path().is_some());
        assert!(AppStatus::output_path().is_some());
        assert!(WidgetBoot::output_path().is_some());
        assert!(crate::desktop::hover::WidgetHover::output_path().is_some());
        assert!(crate::desktop::backdrop::WidgetLiveLuma::output_path().is_some());
    }
}
