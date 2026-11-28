import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { reportFailure } from "./logging";
import type { AgentsSnapshot } from "./bindings/AgentsSnapshot";
import type { Appearance } from "./bindings/Appearance";
import type { AppearancePatch } from "./bindings/AppearancePatch";
import type { AppPaused } from "./bindings/AppPaused";
import type { AppStats } from "./bindings/AppStats";
import type { AppStatus } from "./bindings/AppStatus";
import type { AppVisible } from "./bindings/AppVisible";
import type { ClusterConfig } from "./bindings/ClusterConfig";
import type { Config } from "./bindings/Config";
import type { CpuSensorStatus } from "./bindings/CpuSensorStatus";
import type { DiscordRpcStatus } from "./bindings/DiscordRpcStatus";
import type { DiscordSnapshot } from "./bindings/DiscordSnapshot";
import type { DiscordVoiceAction } from "./bindings/DiscordVoiceAction";
import type { EditDragging } from "./bindings/EditDragging";
import type { EditGuides } from "./bindings/EditGuides";
import type { EditModeState } from "./bindings/EditModeState";
import type { EditSelected } from "./bindings/EditSelected";
import type { GameOverlayState } from "./bindings/GameOverlayState";
import type { GeneralPatch } from "./bindings/GeneralPatch";
import type { GeoResult } from "./bindings/GeoResult";
import type { MediaAction } from "./bindings/MediaAction";
import type { MediaSnapshot } from "./bindings/MediaSnapshot";
import type { MonitorInfo } from "./bindings/MonitorInfo";
import type { MonitorWallpaperPatch } from "./bindings/MonitorWallpaperPatch";
import type { ProviderKind } from "./bindings/ProviderKind";
import type { ProviderSnapshot } from "./bindings/ProviderSnapshot";
import type { SystemCaps } from "./bindings/SystemCaps";
import type { SystemSample } from "./bindings/SystemSample";
import type { UpdateProgress } from "./bindings/UpdateProgress";
import type { UpdateStatus } from "./bindings/UpdateStatus";
import type { VerticalAnchor } from "./bindings/VerticalAnchor";
import type { WallpaperEngineSettingsPatch } from "./bindings/WallpaperEngineSettingsPatch";
import type { WallpaperEngineStatus } from "./bindings/WallpaperEngineStatus";
import type { WallpaperFit } from "./bindings/WallpaperFit";
import type { WallpaperItem } from "./bindings/WallpaperItem";
import type { TimerListSnapshot } from "./bindings/TimerListSnapshot";
import type { WeatherSnapshot } from "./bindings/WeatherSnapshot";
import type { WidgetBoot } from "./bindings/WidgetBoot";
import type { WidgetHover } from "./bindings/WidgetHover";
import type { WidgetLiveLuma } from "./bindings/WidgetLiveLuma";
import type { WidgetGlassState } from "./bindings/WidgetGlassState";
import type { WidgetInstance } from "./bindings/WidgetInstance";
import type { WidgetKind } from "./bindings/WidgetKind";
import type { WidgetPatch } from "./bindings/WidgetPatch";
import type { WidgetPlacement } from "./bindings/WidgetPlacement";
import type { WidgetSize } from "./bindings/WidgetSize";
import type { WindowsQualityStatus } from "./bindings/WindowsQualityStatus";

export function toAssetUrl(path: string): string {
  return path ? convertFileSrc(path) : "";
}

export const ipc = {
  configGet: () => invoke<Config>("config_get"),
  configTakeRestoreNotice: () => invoke<string | null>("config_take_restore_notice"),
  appearanceGet: () => invoke<Appearance>("appearance_get"),
  appearancePatch: (patch: AppearancePatch) => invoke<Config>("appearance_patch", { patch }),
  generalPatch: (patch: GeneralPatch) => invoke<Config>("general_patch", { patch }),
  onboardingComplete: () => invoke<Config>("onboarding_complete"),
  widgetAdd: (kind: WidgetKind, size: WidgetSize, monitor?: string | null) =>
    invoke<WidgetInstance>("widget_add", { kind, size, monitor: monitor ?? null }),
  widgetUpdate: (id: string, patch: WidgetPatch) =>
    invoke<WidgetInstance>("widget_update", { id, patch }),
  widgetRemove: (id: string) => invoke<void>("widget_remove", { id }),
  widgetDuplicate: (id: string) => invoke<WidgetInstance>("widget_duplicate", { id }),
  widgetRestore: (instance?: WidgetInstance | null) =>
    invoke<WidgetInstance>("widget_restore", { instance: instance ?? null }),
  widgetMove: (id: string, monitor: string, x: number, y: number) =>
    invoke<WidgetInstance>("widget_move", { id, monitor, x, y }),
  widgetReady: (id: string) => invoke<WidgetBoot>("widget_ready", { id }),
  widgetSetFocusable: (id: string, focusable: boolean) =>
    invoke<void>("widget_set_focusable", { id, focusable }),
  widgetContextMenu: (id: string) => invoke<void>("widget_context_menu", { id }),
  widgetSetBackdropVisible: (id: string, visible: boolean, durationMs: number) =>
    invoke<void>("widget_set_backdrop_visible", { id, visible, durationMs }),
  widgetLiveLuma: (id: string) => invoke<WidgetLiveLuma>("widget_live_luma", { id }),
  widgetSetIdleHidden: (id: string, hidden: boolean) =>
    invoke<void>("widget_set_idle_hidden", { id, hidden }),
  widgetAnimateBackdropBox: (
    id: string,
    fromHU: number,
    toHU: number,
    anchor: VerticalAnchor,
    durationMs: number,
  ) => invoke<void>("widget_animate_backdrop_box", { id, fromHU, toHU, anchor, durationMs }),
  clusterAddPreset: (presetId: string, monitor?: string | null) =>
    invoke<WidgetInstance[]>("cluster_add_preset", { presetId, monitor: monitor ?? null }),
  clusterAdd: (clusters: ClusterConfig[], monitor?: string | null) =>
    invoke<WidgetInstance[]>("cluster_add", { clusters, monitor: monitor ?? null }),
  clusterSetWindowSize: (
    id: string,
    width: number,
    height: number,
    anchor: VerticalAnchor,
  ) => invoke<void>("cluster_set_window_size", { id, width, height, anchor }),
  editSet: (on: boolean) => invoke<void>("edit_set", { on }),
  editBeginDrag: (id: string) => invoke<void>("edit_begin_drag", { id }),
  editLongPressDrag: (id: string) => invoke<void>("edit_long_press_drag", { id }),
  editNudge: (id: string, dx: number, dy: number) => invoke<void>("edit_nudge", { id, dx, dy }),
  editCycleSize: (id: string) => invoke<WidgetInstance>("edit_cycle_size", { id }),
  monitorsList: () => invoke<MonitorInfo[]>("monitors_list"),
  providerSubscribe: (kind: ProviderKind, key?: string | null) =>
    invoke<ProviderSnapshot | null>("provider_subscribe", { kind, key: key ?? null }),
  providerUnsubscribe: (kind: ProviderKind, key?: string | null) =>
    invoke<void>("provider_unsubscribe", { kind, key: key ?? null }),
  mediaControl: (action: MediaAction, source?: string | null) =>
    invoke<void>("media_control", { action, source: source ?? null }),
  systemHistory: () => invoke<SystemSample[]>("system_history"),
  systemCaps: () => invoke<SystemCaps>("system_caps"),
  cpuSensorStatus: () => invoke<CpuSensorStatus>("cpu_sensor_status"),
  cpuSensorInstall: () => invoke<CpuSensorStatus>("cpu_sensor_install"),
  cpuSensorUninstall: () => invoke<CpuSensorStatus>("cpu_sensor_uninstall"),
  weatherSearch: (query: string) => invoke<GeoResult[]>("weather_search", { query }),
  weatherLocationAdd: (result: GeoResult) => invoke<Config>("weather_location_add", { result }),
  weatherLocationRemove: (id: string) => invoke<Config>("weather_location_remove", { id }),
  weatherRefresh: (locationId: string) => invoke<void>("weather_refresh", { locationId }),
  mediaSelectSource: (source: string | null) => invoke<void>("media_select_source", { source }),
  timerGet: (id: string) => invoke<TimerListSnapshot>("timer_get", { id }),
  timerAdd: (id: string, label: string, durationMs: number) =>
    invoke<void>("timer_add", { id, label, durationMs }),
  timerPause: (id: string, item: string) => invoke<void>("timer_pause", { id, item }),
  timerResume: (id: string, item: string) => invoke<void>("timer_resume", { id, item }),
  timerAdjust: (id: string, item: string, deltaMs: number) =>
    invoke<void>("timer_adjust", { id, item, deltaMs }),
  timerRemove: (id: string, item: string) => invoke<void>("timer_remove", { id, item }),
  timerPreviewSound: (sound: string, volume: number) =>
    invoke<void>("timer_preview_sound", { sound, volume }),
  noteLoad: (id: string) => invoke<string>("note_load", { id }),
  noteSave: (id: string, text: string) => invoke<void>("note_save", { id, text }),

  appOpenControl: (route?: string | null) =>
    invoke<void>("app_open_control", { route: route ?? null }),
  appToggleDevtools: () => invoke<void>("app_toggle_devtools"),
  appQuit: () => invoke<void>("app_quit"),
  appSetVisible: (visible: boolean) => invoke<void>("app_set_visible", { visible }),
  appSetPaused: (paused: boolean) => invoke<void>("app_set_paused", { paused }),
  appStatus: () => invoke<AppStatus>("app_status"),
  appStats: () => invoke<AppStats>("app_stats"),
  appOpenFolder: (kind: "logs" | "data") => invoke<void>("app_open_folder", { kind }),
  layoutExport: (path: string) => invoke<void>("layout_export", { path }),
  layoutImport: (path: string) => invoke<Config>("layout_import", { path }),
  appReset: () => invoke<Config>("app_reset"),
  wallpaperCatalog: () => invoke<WallpaperItem[]>("wallpaper_catalog"),
  wallpaperApply: (
    wallpaperId: string,
    monitor?: string | null,
    fit?: WallpaperFit | null,
  ) => invoke<Config>("wallpaper_apply", { wallpaperId, monitor: monitor ?? null, fit: fit ?? null }),
  wallpaperUpdateAssignment: (monitor: string | null, patch: MonitorWallpaperPatch) =>
    invoke<Config>("wallpaper_update_assignment", { monitor, patch }),
  wallpaperEnginePatch: (patch: WallpaperEngineSettingsPatch) =>
    invoke<Config>("wallpaper_engine_patch", { patch }),
  wallpaperStopLive: (monitor?: string | null) =>
    invoke<Config>("wallpaper_stop_live", { monitor: monitor ?? null }),
  wallpaperImport: (path: string) => invoke<WallpaperItem>("wallpaper_import", { path }),
  wallpaperDeleteCustom: (id: string) => invoke<Config>("wallpaper_delete_custom", { id }),
  wallpaperEngineStatus: () => invoke<WallpaperEngineStatus>("wallpaper_engine_status"),
  wallpaperWindowsQualityGet: () => invoke<WindowsQualityStatus>("wallpaper_windows_quality_get"),
  wallpaperWindowsQualitySet: (enabled: boolean) =>
    invoke<WindowsQualityStatus>("wallpaper_windows_quality_set", { enabled }),
  agentFocus: (agent: string, pid?: number | null, project?: string | null) =>
    invoke<boolean>("agent_focus", { agent, pid: pid ?? null, project: project ?? null }),
  discordRpcStatus: () => invoke<DiscordRpcStatus>("discord_rpc_status"),
  discordRpcConnectCentral: () => invoke<DiscordRpcStatus>("discord_rpc_connect_central"),
  discordRpcConnect: (clientId: string, clientSecret: string, redirectUri?: string | null) =>
    invoke<DiscordRpcStatus>("discord_rpc_connect", {
      clientId,
      clientSecret,
      redirectUri: redirectUri ?? null,
    }),
  discordRpcDisconnect: () => invoke<DiscordRpcStatus>("discord_rpc_disconnect"),
  discordVoiceAction: (action: DiscordVoiceAction) =>
    invoke<void>("discord_voice_action", { action }),
  updateCheck: () => invoke<UpdateStatus>("update_check"),
  updateStatus: () => invoke<UpdateStatus | null>("update_status"),
  updateInstall: () => invoke<void>("update_install"),
};

function listenHere<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  return getCurrentWebviewWindow().listen<T>(event, handler);
}

export const on = {
  config: (callback: (config: Config) => void) =>
    listenHere<Config>("config://changed", (event) => callback(event.payload)),
  configRestored: (callback: (message: string) => void) =>
    listenHere<string>("config://restored", (event) => callback(event.payload)),
  editMode: (callback: (state: EditModeState) => void) =>
    listenHere<EditModeState>("edit://mode", (event) => callback(event.payload)),
  editSelected: (callback: (state: EditSelected) => void) =>
    listenHere<EditSelected>("edit://selected", (event) => callback(event.payload)),
  editDragging: (callback: (state: EditDragging) => void) =>
    listenHere<EditDragging>("edit://dragging", (event) => callback(event.payload)),
  editGuides: (callback: (guides: EditGuides) => void) =>
    listenHere<EditGuides>("edit://guides", (event) => callback(event.payload)),
  paused: (callback: (state: AppPaused) => void) =>
    listenHere<AppPaused>("app://paused", (event) => callback(event.payload)),
  visible: (callback: (state: AppVisible) => void) =>
    listenHere<AppVisible>("app://visible", (event) => callback(event.payload)),
  gameOverlay: (callback: (state: GameOverlayState) => void) =>
    listenHere<GameOverlayState>("app://game-overlay", (event) => callback(event.payload)),
  trayNotice: (callback: (message: string) => void) =>
    listenHere<string>("app://tray-notice", (event) => callback(event.payload)),
  monitors: (callback: (monitors: MonitorInfo[]) => void) =>
    listenHere<MonitorInfo[]>("desktop://monitors", (event) => callback(event.payload)),
  placement: (callback: (placement: WidgetPlacement) => void) =>
    listenHere<WidgetPlacement>("widget://placement", (event) => callback(event.payload)),
  glass: (callback: (glass: WidgetGlassState) => void) =>
    listenHere<WidgetGlassState>("widget://glass", (event) => callback(event.payload)),
  widgetRemoved: (callback: (instance: WidgetInstance) => void) =>
    listenHere<WidgetInstance>("widget://removed", (event) => callback(event.payload)),
  widgetHover: (callback: (state: WidgetHover) => void) =>
    listenHere<WidgetHover>("widget://hover", (event) => callback(event.payload)),
  liveLuma: (callback: (state: WidgetLiveLuma) => void) =>
    listenHere<WidgetLiveLuma>("widget://live-luma", (event) => callback(event.payload)),
  navigate: (callback: (route: string) => void) =>
    listenHere<string>("app://navigate", (event) => callback(event.payload)),
  media: (callback: (snapshot: MediaSnapshot) => void) =>
    listenHere<MediaSnapshot>("provider://media", (event) => callback(event.payload)),
  system: (callback: (sample: SystemSample) => void) =>
    listenHere<SystemSample>("provider://system", (event) => callback(event.payload)),
  weather: (callback: (snapshot: WeatherSnapshot) => void) =>
    listenHere<WeatherSnapshot>("provider://weather", (event) => callback(event.payload)),
  discord: (callback: (snapshot: DiscordSnapshot) => void) =>
    listenHere<DiscordSnapshot>("provider://discord", (event) => callback(event.payload)),
  discordRpcStatus: (callback: (status: DiscordRpcStatus) => void) =>
    listenHere<DiscordRpcStatus>("discord://rpc-status", (event) => callback(event.payload)),
  timerState: (callback: (snapshot: TimerListSnapshot) => void) =>
    listenHere<TimerListSnapshot>("timer://state", (event) => callback(event.payload)),
  agents: (callback: (snapshot: AgentsSnapshot) => void) =>
    listenHere<AgentsSnapshot>("provider://agents", (event) => callback(event.payload)),
  wallpaperStatus: (callback: (status: WallpaperEngineStatus) => void) =>
    listenHere<WallpaperEngineStatus>("wallpaper://status", (event) => callback(event.payload)),
  wallpaperThumbsReady: (callback: () => void) =>
    listenHere<void>("wallpaper://thumbs-ready", () => callback()),
  updateStatus: (callback: (status: UpdateStatus) => void) =>
    listenHere<UpdateStatus>("update://status", (event) => callback(event.payload)),
  updateProgress: (callback: (progress: UpdateProgress) => void) =>
    listenHere<UpdateProgress>("update://progress", (event) => callback(event.payload)),
};

/** Registrerer flere lyttere; returnerer én dispose. Lyttere der når at
    resolve efter dispose afmeldes straks; registreringsfejl logges. */
export function subscribeAll(entries: ReadonlyArray<readonly [context: string, pending: Promise<UnlistenFn>]>): () => void {
  let disposed = false;
  const unlisteners: UnlistenFn[] = [];
  for (const [context, pending] of entries) {
    pending.then(
      (un) => { if (disposed) un(); else unlisteners.push(un); },
      (reason: unknown) => reportFailure(context, reason),
    );
  }
  return () => { disposed = true; for (const un of unlisteners.splice(0)) un(); };
}

export const windowControls = {
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  isMaximized: () => getCurrentWindow().isMaximized(),
  onResized: (cb: () => void) => getCurrentWindow().onResized(cb),
  close: () => getCurrentWindow().close(),
};
