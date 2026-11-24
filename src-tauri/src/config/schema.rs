use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;
use uuid::Uuid;

use crate::error::AppError;

pub const CONFIG_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatchValue<T> {
    #[default]
    Unset,
    Null,
    Set(T),
}

impl<T> PatchValue<T> {
    pub fn apply_to(self, slot: &mut Option<T>) {
        match self {
            Self::Unset => {}
            Self::Null => *slot = None,
            Self::Set(val) => *slot = Some(val),
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> PatchValue<U> {
        match self {
            Self::Unset => PatchValue::Unset,
            Self::Null => PatchValue::Null,
            Self::Set(v) => PatchValue::Set(f(v)),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for PatchValue<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Option::<T>::deserialize(deserializer)? {
            Some(v) => Ok(Self::Set(v)),
            None => Ok(Self::Null),
        }
    }
}

impl<T: Serialize> Serialize for PatchValue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Unset | Self::Null => serializer.serialize_none(),
            Self::Set(v) => v.serialize(serializer),
        }
    }
}

impl<T: TS + 'static> TS for PatchValue<T> {
    type WithoutGenerics = T::WithoutGenerics;
    type OptionInnerType = Self;
    const IS_OPTION: bool = true;

    fn name(cfg: &ts_rs::Config) -> String {
        format!("{} | null", T::name(cfg))
    }

    fn inline(cfg: &ts_rs::Config) -> String {
        format!("{} | null", T::inline(cfg))
    }

    fn inline_flattened(cfg: &ts_rs::Config) -> String {
        format!("{} | null", T::inline_flattened(cfg))
    }

    fn ident(cfg: &ts_rs::Config) -> String {
        T::ident(cfg)
    }

    fn decl(cfg: &ts_rs::Config) -> String {
        T::decl(cfg)
    }

    fn decl_concrete(cfg: &ts_rs::Config) -> String {
        T::decl_concrete(cfg)
    }

    fn visit_dependencies(v: &mut impl ts_rs::TypeVisitor) {
        v.visit::<T>();
        T::visit_dependencies(v);
    }

    fn output_path() -> Option<std::path::PathBuf> {
        T::output_path()
    }
}

pub fn clamp_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return 1.0;
    }
    scale.clamp(0.75, 1.35)
}

pub fn clamp_glass_strength(strength: f32) -> f32 {
    if !strength.is_finite() {
        return 1.0;
    }
    strength.clamp(0.6, 1.4)
}

pub fn normalize_system_interval(ms: u32) -> u32 {
    match ms {
        0..=1499 => 1000,
        1500..=3499 => 2000,
        _ => 5000,
    }
}

pub fn normalize_wallpaper_fps(fps: u32) -> u32 {
    match fps {
        0..=26 => 24,
        27..=44 => 30,
        _ => 60,
    }
}

pub fn clamp_wallpaper_speed(speed: f32) -> f32 {
    if !speed.is_finite() {
        return 1.0;
    }
    speed.clamp(0.25, 2.0)
}

pub fn clamp_wallpaper_brightness(brightness: f32) -> f32 {
    if !brightness.is_finite() {
        return 1.0;
    }
    brightness.clamp(0.6, 1.2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum Language {
    #[default]
    Da,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum ControlTheme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct General {
    pub language: Language,
    #[serde(default)]
    pub control_theme: ControlTheme,
    pub autostart: bool,
    pub system_interval_ms: u32,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: Language::Da,
            control_theme: ControlTheme::Dark,
            autostart: true,
            system_interval_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/", optional_fields)]
pub struct GeneralPatch {
    pub language: Option<Language>,
    pub control_theme: Option<ControlTheme>,
    pub autostart: Option<bool>,
    pub system_interval_ms: Option<u32>,
}

impl General {
    pub fn apply_patch(&mut self, patch: GeneralPatch) {
        if let Some(language) = patch.language {
            self.language = language;
        }
        if let Some(control_theme) = patch.control_theme {
            self.control_theme = control_theme;
        }
        if let Some(autostart) = patch.autostart {
            self.autostart = autostart;
        }
        if let Some(interval_ms) = patch.system_interval_ms {
            self.system_interval_ms = normalize_system_interval(interval_ms);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum ThemeId {
    Aurora,
    Laerk,
    Grafit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum GlassEngine {
    Wallpaper,
    Acrylic,
    Tint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum RadiusPreset {
    Soft,
    Round,
    Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum MotionPreset {
    Full,
    Calm,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
#[repr(u8)]
pub enum Layer {
    Desktop = 0,
    Top = 1,
}

impl Layer {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Top,
            _ => Self::Desktop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum WidgetKind {
    Clock,
    NowPlaying,
    System,
    Weather,
    Note,
    Discord,
    Cluster,
    Agents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum WidgetSize {
    Pill,
    S,
    M,
    L,
}

impl WidgetKind {
    pub fn allowed_sizes(self) -> &'static [WidgetSize] {
        match self {
            Self::Clock | Self::Discord => &[WidgetSize::Pill, WidgetSize::S, WidgetSize::M],
            Self::NowPlaying | Self::System => {
                &[WidgetSize::Pill, WidgetSize::S, WidgetSize::M, WidgetSize::L]
            }
            Self::Weather | Self::Note => &[WidgetSize::S, WidgetSize::M, WidgetSize::L],
            Self::Cluster | Self::Agents => &[WidgetSize::S, WidgetSize::M, WidgetSize::L],
        }
    }

    pub fn allows_size(self, size: WidgetSize) -> bool {
        self.allowed_sizes().contains(&size)
    }

    pub fn default_size(self) -> WidgetSize {
        match self {
            Self::Clock | Self::NowPlaying | Self::System | Self::Discord | Self::Agents => {
                WidgetSize::M
            }
            Self::Weather | Self::Note | Self::Cluster => WidgetSize::M,
        }
    }
}

impl WidgetSize {
    pub fn base_size(self) -> (f64, f64) {
        match self {
            Self::Pill => (204.0, 48.0),
            Self::S => (132.0, 132.0),
            Self::M => (276.0, 132.0),
            Self::L => (276.0, 276.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct Appearance {
    pub theme_id: ThemeId,
    pub accent: Option<String>,
    pub glass_engine: GlassEngine,
    pub glass_strength: f32,
    pub radius: RadiusPreset,
    pub motion: MotionPreset,
    pub auto_contrast: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme_id: ThemeId::Aurora,
            accent: None,
            glass_engine: GlassEngine::Wallpaper,
            glass_strength: 1.0,
            radius: RadiusPreset::Round,
            motion: MotionPreset::Full,
            auto_contrast: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/", optional_fields)]
pub struct AppearancePatch {
    pub theme_id: Option<ThemeId>,
    pub accent: PatchValue<String>,
    pub glass_engine: Option<GlassEngine>,
    pub glass_strength: Option<f32>,
    pub radius: Option<RadiusPreset>,
    pub motion: Option<MotionPreset>,
    pub auto_contrast: Option<bool>,
}

impl Appearance {
    pub fn apply_patch(&mut self, patch: AppearancePatch) {
        if let Some(theme_id) = patch.theme_id {
            self.theme_id = theme_id;
        }
        patch.accent.apply_to(&mut self.accent);
        if let Some(glass_engine) = patch.glass_engine {
            self.glass_engine = glass_engine;
        }
        if let Some(glass_strength) = patch.glass_strength {
            self.glass_strength = clamp_glass_strength(glass_strength);
        }
        if let Some(radius) = patch.radius {
            self.radius = radius;
        }
        if let Some(motion) = patch.motion {
            self.motion = motion;
        }
        if let Some(auto_contrast) = patch.auto_contrast {
            self.auto_contrast = auto_contrast;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct PointF {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct SizeF {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct MonitorKey {
    pub name: String,
    pub rect: RectI,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct Placement {
    pub monitor: MonitorKey,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct Location {
    pub id: Uuid,
    pub name: String,
    pub admin: Option<String>,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WeatherConfig {
    pub locations: Vec<Location>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum ModuleMode {
    Hidden,
    Compact,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum AutoRule {
    None,
    ExpandWhenPlaying,
    CollapseWhenIdle,
    ExpandWhenHighLoad,
    ExpandWhenActive,
    ExpandWhenInCall,
    FillRemainingSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum ResizeBehavior {
    Fixed,
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum VerticalAnchor {
    Top,
    Bottom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct ClusterModule {
    pub id: Uuid,
    pub kind: WidgetKind,
    pub default_mode: ModuleMode,
    pub auto_rule: AutoRule,
    #[ts(type = "Record<string, unknown>")]
    pub settings: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct ClusterConfig {
    pub preset_id: Option<String>,
    pub width: u32,
    pub max_height: u32,
    pub resize_behavior: ResizeBehavior,
    pub anchor: VerticalAnchor,
    pub modules: Vec<ClusterModule>,
}

impl ClusterConfig {
    pub fn module_height(kind: WidgetKind, mode: ModuleMode) -> f64 {
        match (kind, mode) {
            (_, ModuleMode::Hidden) => 0.0,
            (WidgetKind::Clock, ModuleMode::Compact) => 32.0,
            (WidgetKind::Clock, ModuleMode::Expanded) => 76.0,
            (WidgetKind::NowPlaying, ModuleMode::Compact) => 40.0,
            (WidgetKind::NowPlaying, ModuleMode::Expanded) => 102.0,
            (WidgetKind::System, ModuleMode::Compact) => 32.0,
            (WidgetKind::System, ModuleMode::Expanded) => 104.0,
            (WidgetKind::Weather, ModuleMode::Compact) => 32.0,
            (WidgetKind::Weather, ModuleMode::Expanded) => 112.0,
            (WidgetKind::Note, ModuleMode::Compact) => 32.0,
            (WidgetKind::Note, ModuleMode::Expanded) => 104.0,
            (WidgetKind::Discord, ModuleMode::Compact) => 36.0,
            (WidgetKind::Discord, ModuleMode::Expanded) => 108.0,
            (WidgetKind::Agents, ModuleMode::Compact) => 32.0,
            (WidgetKind::Agents, ModuleMode::Expanded) => 132.0,
            (WidgetKind::Cluster, _) => 48.0,
        }
    }

    pub fn initial_height(&self) -> f64 {
        if self.resize_behavior == ResizeBehavior::Fixed {
            return (self.max_height.max(48)) as f64;
        }
        if self.preset_id.as_deref() == Some("pillBar") {
            return 48.0;
        }
        let mut modes: Vec<ModuleMode> = self
            .modules
            .iter()
            .map(|m| match m.auto_rule {
                AutoRule::ExpandWhenPlaying
                | AutoRule::CollapseWhenIdle
                | AutoRule::ExpandWhenInCall => ModuleMode::Hidden,
                AutoRule::ExpandWhenHighLoad | AutoRule::ExpandWhenActive => ModuleMode::Compact,
                AutoRule::None | AutoRule::FillRemainingSpace => {
                    m.default_mode
                }
            })
            .collect();

        if !modes.contains(&ModuleMode::Expanded) {
            if let Some((idx, _)) = self
                .modules
                .iter()
                .enumerate()
                .find(|(idx, m)| {
                    modes[*idx] != ModuleMode::Hidden
                        && m.auto_rule == AutoRule::FillRemainingSpace
                })
            {
                modes[idx] = ModuleMode::Expanded;
            }
        }

        let mut visible_count = 0usize;
        let mut sum_h = 0.0_f64;
        for (m, mode) in self.modules.iter().zip(modes.iter().copied()) {
            if mode != ModuleMode::Hidden {
                visible_count += 1;
                sum_h += Self::module_height(m.kind, mode);
            }
        }
        if visible_count == 0 {
            return 48.0;
        }
        let gaps = (visible_count.saturating_sub(1) as f64) * 7.0;
        let raw = 20.0 + sum_h + gaps;
        raw.clamp(48.0, (self.max_height.max(48)) as f64)
    }

    pub fn initial_base_size(&self) -> (f64, f64) {
        ((self.width.max(132)) as f64, self.initial_height())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WidgetInstance {
    pub id: Uuid,
    pub kind: WidgetKind,
    pub size: WidgetSize,
    pub scale: f32,
    pub placement: Placement,
    pub layer: Layer,
    pub click_through: bool,
    #[serde(default)]
    pub theme_override: Option<ThemeId>,
    #[serde(default)]
    pub glass_override: Option<f32>,
    #[serde(default)]
    pub glass_override_engine: Option<GlassEngine>,
    #[serde(default)]
    #[ts(type = "Record<string, unknown>")]
    pub settings: serde_json::Value,
    #[serde(default)]
    pub cluster: Option<ClusterConfig>,
    pub created_at: i64,
}

impl WidgetInstance {
    pub fn base_size(&self) -> (f64, f64) {
        if self.kind == WidgetKind::Cluster {
            if let Some(cluster) = &self.cluster {
                return cluster.initial_base_size();
            }
        }
        self.size.base_size()
    }

    pub fn window_label(&self) -> String {
        format!("w-{}", self.id)
    }

    pub fn apply_patch(&mut self, patch: WidgetPatch) -> Result<(), AppError> {
        if let Some(size) = patch.size {
            if !self.kind.allows_size(size) {
                return Err(AppError::SizeNotAllowed);
            }
            self.size = size;
        }
        if let Some(scale) = patch.scale {
            self.scale = clamp_scale(scale);
        }
        if let Some(layer) = patch.layer {
            self.layer = layer;
        }
        if let Some(click_through) = patch.click_through {
            self.click_through = click_through;
        }
        patch.theme_override.apply_to(&mut self.theme_override);
        patch
            .glass_override
            .map(clamp_glass_strength)
            .apply_to(&mut self.glass_override);
        patch
            .glass_override_engine
            .apply_to(&mut self.glass_override_engine);
        if let Some(settings) = patch.settings {
            self.settings = settings;
        }
        patch.cluster.apply_to(&mut self.cluster);
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/", optional_fields)]
pub struct WidgetPatch {
    pub size: Option<WidgetSize>,
    pub scale: Option<f32>,
    pub layer: Option<Layer>,
    pub click_through: Option<bool>,
    pub theme_override: PatchValue<ThemeId>,
    pub glass_override: PatchValue<f32>,
    pub glass_override_engine: PatchValue<GlassEngine>,
    #[ts(optional, type = "Record<string, unknown>")]
    pub settings: Option<serde_json::Value>,
    pub cluster: PatchValue<ClusterConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum WallpaperKind {
    Shader,
    Photo,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum WallpaperMood {
    Dark,
    Nature,
    Minimal,
    Warm,
    Architecture,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum WallpaperFit {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum WallpaperResolution {
    #[default]
    Native,
    Balanced,
}

fn default_wp_fps() -> u32 {
    30
}

fn default_wp_speed() -> f32 {
    1.0
}

fn default_wp_brightness() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WallpaperEngineSettings {
    #[serde(default = "default_wp_fps")]
    pub fps_cap: u32,
    #[serde(default)]
    pub resolution: WallpaperResolution,
    #[serde(default = "default_true")]
    pub pause_when_covered: bool,
    #[serde(default = "default_true")]
    pub pause_on_fullscreen: bool,
    #[serde(default = "default_true")]
    pub pause_on_battery: bool,
    #[serde(default = "default_true")]
    pub sync_glass_plates: bool,
    #[serde(default = "default_true")]
    pub lossless_png_apply: bool,
}

impl Default for WallpaperEngineSettings {
    fn default() -> Self {
        Self {
            fps_cap: 30,
            resolution: WallpaperResolution::Native,
            pause_when_covered: true,
            pause_on_fullscreen: true,
            pause_on_battery: true,
            sync_glass_plates: true,
            lossless_png_apply: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/", optional_fields)]
pub struct WallpaperEngineSettingsPatch {
    pub fps_cap: Option<u32>,
    pub resolution: Option<WallpaperResolution>,
    pub pause_when_covered: Option<bool>,
    pub pause_on_fullscreen: Option<bool>,
    pub pause_on_battery: Option<bool>,
    pub sync_glass_plates: Option<bool>,
    pub lossless_png_apply: Option<bool>,
}

impl WallpaperEngineSettings {
    pub fn apply_patch(&mut self, patch: WallpaperEngineSettingsPatch) {
        if let Some(fps_cap) = patch.fps_cap {
            self.fps_cap = normalize_wallpaper_fps(fps_cap);
        }
        if let Some(resolution) = patch.resolution {
            self.resolution = resolution;
        }
        if let Some(pause_when_covered) = patch.pause_when_covered {
            self.pause_when_covered = pause_when_covered;
        }
        if let Some(pause_on_fullscreen) = patch.pause_on_fullscreen {
            self.pause_on_fullscreen = pause_on_fullscreen;
        }
        if let Some(pause_on_battery) = patch.pause_on_battery {
            self.pause_on_battery = pause_on_battery;
        }
        if let Some(sync_glass_plates) = patch.sync_glass_plates {
            self.sync_glass_plates = sync_glass_plates;
        }
        if let Some(lossless_png_apply) = patch.lossless_png_apply {
            self.lossless_png_apply = lossless_png_apply;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct MonitorWallpaper {
    pub wallpaper_id: String,
    #[serde(default)]
    pub fit: WallpaperFit,
    #[serde(default = "default_wp_speed")]
    pub speed: f32,
    #[serde(default = "default_wp_brightness")]
    pub brightness: f32,
    #[serde(default = "default_true")]
    pub muted: bool,
}

impl MonitorWallpaper {
    pub fn new(wallpaper_id: String, fit: WallpaperFit) -> Self {
        Self {
            wallpaper_id,
            fit,
            speed: 1.0,
            brightness: 1.0,
            muted: true,
        }
    }

    pub fn apply_patch(&mut self, patch: MonitorWallpaperPatch) {
        if let Some(fit) = patch.fit {
            self.fit = fit;
        }
        if let Some(speed) = patch.speed {
            self.speed = clamp_wallpaper_speed(speed);
        }
        if let Some(brightness) = patch.brightness {
            self.brightness = clamp_wallpaper_brightness(brightness);
        }
        if let Some(muted) = patch.muted {
            self.muted = muted;
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/", optional_fields)]
pub struct MonitorWallpaperPatch {
    pub fit: Option<WallpaperFit>,
    pub speed: Option<f32>,
    pub brightness: Option<f32>,
    pub muted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct CustomWallpaper {
    pub id: String,
    pub title: String,
    pub kind: WallpaperKind,
    pub file_path: String,
    pub thumb_path: String,
    pub width: u32,
    pub height: u32,
    pub file_size_bytes: u64,
    pub added_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WallpaperConfig {
    #[serde(default)]
    pub engine: WallpaperEngineSettings,
    #[serde(default)]
    pub assignments: HashMap<String, MonitorWallpaper>,
    #[serde(default)]
    pub custom_items: Vec<CustomWallpaper>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct Config {
    pub version: u32,
    pub general: General,
    pub appearance: Appearance,
    pub weather: WeatherConfig,
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    pub widgets: Vec<WidgetInstance>,
    pub onboarded: bool,
}
