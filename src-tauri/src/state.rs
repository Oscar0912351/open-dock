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
    Fullscreen,
    Battery,
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
    pub auto_pause_reason: RwLock<Option<PauseReason>>,
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
            auto_pause_reason: RwLock::new(None),
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
    use crate::{
        config::schema::{
            AppearancePatch, AutoRule, ClusterConfig, ClusterModule, Config, ControlTheme,
            CustomWallpaper, General, GeneralPatch, Hotkeys, Language, Layer, Location,
            ModuleMode, MonitorKey, MonitorWallpaper, MonitorWallpaperPatch, MotionPreset,
            Placement, PointF, RadiusPreset, RectF, RectI, ResizeBehavior, SizeF, ThemeId,
            VerticalAnchor, WallpaperConfig, WallpaperEngineSettings, WallpaperEngineSettingsPatch,
            WallpaperFit, WallpaperKind, WallpaperMood, WallpaperResolution, WeatherConfig,
            WidgetKind, WidgetPatch, WidgetSize,
        },
        desktop::snap::{EditGuides, Guide, GuideAxis},
        providers::{
            agents::{AgentSession, AgentStatus, AgentTodoProgress, AgentsSnapshot},
            discord::{
                DiscordConnectionMode, DiscordParticipant, DiscordRpcStatus, DiscordSnapshot,
                DiscordVoiceAction,
            },
            gpu::SystemCaps,
            media::{MediaAction, MediaSnapshot, PlaybackStatus, RepeatMode},
            scheduler::{ProviderKind, ProviderSnapshot},
            system::{CpuSample, DiskSample, GpuSample, MemSample, NetSample, SystemSample},
            weather::{GeoResult, WeatherCurrent, WeatherDaily, WeatherHourly, WeatherSnapshot},
        },
        wallpaper::{
            catalog::WallpaperItem,
            engine::{EngineRunState, WallpaperEngineStatus},
            quality::WindowsQualityStatus,
        },
    };
    use ts_rs::TS;

    #[test]
    fn export_bindings() {
        let config = ts_rs::Config::default();
        Language::export_all(&config).unwrap();
        ControlTheme::export_all(&config).unwrap();
        Hotkeys::export_all(&config).unwrap();
        General::export_all(&config).unwrap();
        GeneralPatch::export_all(&config).unwrap();
        ThemeId::export_all(&config).unwrap();
        GlassEngine::export_all(&config).unwrap();
        RadiusPreset::export_all(&config).unwrap();
        MotionPreset::export_all(&config).unwrap();
        Layer::export_all(&config).unwrap();
        WidgetKind::export_all(&config).unwrap();
        WidgetSize::export_all(&config).unwrap();
        Appearance::export_all(&config).unwrap();
        AppearancePatch::export_all(&config).unwrap();
        RectI::export_all(&config).unwrap();
        RectF::export_all(&config).unwrap();
        PointF::export_all(&config).unwrap();
        SizeF::export_all(&config).unwrap();
        GuideAxis::export_all(&config).unwrap();
        Guide::export_all(&config).unwrap();
        EditGuides::export_all(&config).unwrap();
        EditSelected::export_all(&config).unwrap();
        EditDragging::export_all(&config).unwrap();
        MonitorKey::export_all(&config).unwrap();
        Placement::export_all(&config).unwrap();
        Location::export_all(&config).unwrap();
        WeatherConfig::export_all(&config).unwrap();
        WallpaperKind::export_all(&config).unwrap();
        WallpaperMood::export_all(&config).unwrap();
        WallpaperFit::export_all(&config).unwrap();
        WallpaperResolution::export_all(&config).unwrap();
        WallpaperEngineSettings::export_all(&config).unwrap();
        WallpaperEngineSettingsPatch::export_all(&config).unwrap();
        MonitorWallpaper::export_all(&config).unwrap();
        MonitorWallpaperPatch::export_all(&config).unwrap();
        CustomWallpaper::export_all(&config).unwrap();
        WallpaperConfig::export_all(&config).unwrap();
        WallpaperItem::export_all(&config).unwrap();
        WindowsQualityStatus::export_all(&config).unwrap();
        EngineRunState::export_all(&config).unwrap();
        WallpaperEngineStatus::export_all(&config).unwrap();
        ModuleMode::export_all(&config).unwrap();
        AutoRule::export_all(&config).unwrap();
        ResizeBehavior::export_all(&config).unwrap();
        VerticalAnchor::export_all(&config).unwrap();
        ClusterModule::export_all(&config).unwrap();
        ClusterConfig::export_all(&config).unwrap();
        WidgetInstance::export_all(&config).unwrap();
        WidgetPatch::export_all(&config).unwrap();
        Config::export_all(&config).unwrap();
        MonitorInfo::export_all(&config).unwrap();
        WidgetPlacement::export_all(&config).unwrap();
        WidgetGlassState::export_all(&config).unwrap();
        WidgetBoot::export_all(&config).unwrap();
        PauseReason::export_all(&config).unwrap();
        AppPaused::export_all(&config).unwrap();
        AppStats::export_all(&config).unwrap();
        AppVisible::export_all(&config).unwrap();
        GameOverlayState::export_all(&config).unwrap();
        AppStatus::export_all(&config).unwrap();
        EditModeState::export_all(&config).unwrap();
        ProviderKind::export_all(&config).unwrap();
        ProviderSnapshot::export_all(&config).unwrap();
        PlaybackStatus::export_all(&config).unwrap();
        RepeatMode::export_all(&config).unwrap();
        MediaSnapshot::export_all(&config).unwrap();
        MediaAction::export_all(&config).unwrap();
        SystemCaps::export_all(&config).unwrap();
        CpuSample::export_all(&config).unwrap();
        MemSample::export_all(&config).unwrap();
        GpuSample::export_all(&config).unwrap();
        NetSample::export_all(&config).unwrap();
        DiskSample::export_all(&config).unwrap();
        SystemSample::export_all(&config).unwrap();
        WeatherCurrent::export_all(&config).unwrap();
        WeatherHourly::export_all(&config).unwrap();
        WeatherDaily::export_all(&config).unwrap();
        WeatherSnapshot::export_all(&config).unwrap();
        GeoResult::export_all(&config).unwrap();
        DiscordConnectionMode::export_all(&config).unwrap();
        DiscordParticipant::export_all(&config).unwrap();
        DiscordRpcStatus::export_all(&config).unwrap();
        DiscordVoiceAction::export_all(&config).unwrap();
        DiscordSnapshot::export_all(&config).unwrap();
        AgentStatus::export_all(&config).unwrap();
        AgentTodoProgress::export_all(&config).unwrap();
        AgentSession::export_all(&config).unwrap();
        AgentsSnapshot::export_all(&config).unwrap();
        crate::update::ReleaseManifest::export_all(&config).unwrap();
        crate::update::UpdateCheckState::export_all(&config).unwrap();
        crate::update::UpdateStatus::export_all(&config).unwrap();
        crate::update::UpdateProgress::export_all(&config).unwrap();
    }
}
