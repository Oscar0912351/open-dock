import { useCallback, useMemo, useRef, useState } from "react";
import type { Appearance } from "../../shared/bindings/Appearance";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { Location } from "../../shared/bindings/Location";
import type { MonitorInfo } from "../../shared/bindings/MonitorInfo";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import type { WidgetKind } from "../../shared/bindings/WidgetKind";
import type { WidgetPlacement } from "../../shared/bindings/WidgetPlacement";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { toAssetUrl } from "../../shared/ipc";
import { ClusterShell } from "../../widget/cluster/ClusterShell";
import { computeClusterTargetHeight, DEFAULT_CLUSTER_SIGNALS, resolveModuleModes } from "../../widget/cluster/resolveLayout";
import { Agents } from "../../widget/kinds/agents/Agents";
import { Clock, resolveClockSettings } from "../../widget/kinds/clock/Clock";
import { useClock } from "../../widget/kinds/clock/useClock";
import { Discord } from "../../widget/kinds/discord/Discord";
import { Note, resolveNoteSettings } from "../../widget/kinds/note/Note";
import { NowPlaying, resolveNowPlayingSettings } from "../../widget/kinds/nowPlaying/NowPlaying";
import { resolveSystemSettings, System } from "../../widget/kinds/system/System";
import { resolveWeatherSettings, Weather } from "../../widget/kinds/weather/Weather";
import { WIDGET_REGISTRY } from "../../widget/registry";
import { WidgetShell, type ClusterFrameInfo, type GlassPlacementInfo } from "../../widget/WidgetShell";

const SIZE_DIMS: Record<WidgetSize, { width: number; height: number }> = {
  pill: { width: 204, height: 48 },
  s: { width: 132, height: 132 },
  m: { width: 276, height: 132 },
  l: { width: 276, height: 276 },
};

export interface WidgetPreviewProps {
  id?: string;
  kind: WidgetKind;
  size: WidgetSize;
  settings?: Record<string, unknown>;
  cluster?: ClusterConfig | null;
  instance?: WidgetInstance;
  appearance: Appearance;
  monitors: MonitorInfo[];
  locations: Location[];
  livePlacement?: WidgetPlacement | null;
  themeOverride?: ThemeId | null;
  glassOverride?: number | null;
  maxWidth: number;
  maxHeight: number;
  stageHeight?: number;
  interactive?: boolean;
  retriggerKey?: string | number;
  className?: string;
}

export function WidgetPreview({
  id = "preview-widget",
  kind,
  size,
  settings,
  cluster = null,
  instance,
  appearance,
  monitors,
  locations,
  livePlacement,
  themeOverride = null,
  glassOverride = null,
  maxWidth,
  maxHeight,
  stageHeight,
  interactive = false,
  retriggerKey,
  className = "",
}: WidgetPreviewProps) {
  const previewKey = `${id}:${kind}`;
  const [ambientState, setAmbientState] = useState<{
    key: string;
    color: [number, number, number] | null;
  }>({ key: "", color: null });
  const [frameState, setFrameState] = useState<{
    key: string;
    frame: ClusterFrameInfo | null;
  }>({ key: "", frame: null });
  const transitionEndRef = useRef<(() => void) | null>(null);

  const registerTransitionEnd = useCallback((fn: (() => void) | null) => {
    transitionEndRef.current = fn;
  }, []);

  const handleAmbientChange = useCallback(
    (color: [number, number, number] | null) => {
      setAmbientState({ key: previewKey, color });
    },
    [previewKey],
  );

  const handleClusterFrameChange = useCallback(
    (frame: ClusterFrameInfo | null) => {
      setFrameState({ key: previewKey, frame });
    },
    [previewKey],
  );

  const ambientColor =
    (kind === "nowPlaying" || kind === "cluster") && ambientState.key === previewKey
      ? ambientState.color
      : null;
  const clusterFrame =
    kind === "cluster" && frameState.key === previewKey ? frameState.frame : null;

  const effectiveTheme = themeOverride ?? instance?.themeOverride ?? appearance.themeId;
  const effectiveGlass = glassOverride ?? instance?.glassOverride ?? appearance.glassStrength;

  // Forhåndsvisning viser altid Wallpaper-udseende når app/widget bruger Wallpaper eller Acrylic
  const previewEngine = appearance.glassEngine === "tint" ? "tint" : "wallpaper";

  const clockSettings = resolveClockSettings(kind === "clock" ? settings : undefined);
  const now = useClock(kind === "clock" && clockSettings.showSeconds);

  const syntheticInstance = useMemo<WidgetInstance>(() => {
    if (instance) return instance;
    const defaultSettings =
      kind === "cluster" ? {} : WIDGET_REGISTRY[kind].defaultSettings;
    return {
      id,
      kind,
      size,
      scale: 1,
      placement: {
        monitor: {
          name: monitors[0]?.name ?? "\\\\.\\DISPLAY1",
          rect: monitors[0]?.rect ?? { x: 0, y: 0, width: 1920, height: 1080 },
        },
        x: 20,
        y: 20,
      },
      layer: "desktop",
      clickThrough: false,
      themeOverride,
      glassOverride,
      glassOverrideEngine: null,
      settings: settings ?? defaultSettings,
      cluster,
      createdAt: BigInt(0),
    };
  }, [instance, id, kind, size, monitors, themeOverride, glassOverride, settings, cluster]);

  const baseDims = useMemo(() => {
    if (kind === "cluster" && cluster) {
      if (clusterFrame) {
        return { width: clusterFrame.width, height: clusterFrame.boxHeight };
      }
      const idleModes = resolveModuleModes(
        cluster.modules,
        DEFAULT_CLUSTER_SIGNALS,
        null,
        cluster.resizeBehavior,
        cluster.presetId === "pillBar",
      );
      const h = computeClusterTargetHeight(
        cluster.modules,
        idleModes,
        cluster.presetId,
        cluster.resizeBehavior,
        cluster.maxHeight,
      );
      return { width: cluster.width, height: h };
    }
    return SIZE_DIMS[size];
  }, [kind, cluster, clusterFrame, size]);

  const targetMonitor = useMemo(() => {
    if (livePlacement) {
      const m = monitors.find((item) => item.name === livePlacement.monitor);
      if (m) return m;
    }
    if (instance) {
      const byName = monitors.find((m) => m.name === instance.placement.monitor.name);
      if (byName) return byName;
    }
    return monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null;
  }, [monitors, livePlacement, instance]);

  const plateInfo = useMemo<GlassPlacementInfo | null>(() => {
    if (!targetMonitor?.platePath) return null;
    const monW = targetMonitor.logicalSize.width || 1920;
    const monH = targetMonitor.logicalSize.height || 1080;

    if (livePlacement) {
      return {
        platePath: targetMonitor.platePath,
        monitorLogicalW: monW,
        monitorLogicalH: monH,
        wx: livePlacement.rect.x,
        wy: livePlacement.rect.y,
        luma: livePlacement.luma,
      };
    }

    const wx = Math.max(0, (monW - baseDims.width) / 2);
    const wy = Math.max(0, (monH - baseDims.height) / 2);
    return {
      platePath: targetMonitor.platePath,
      monitorLogicalW: monW,
      monitorLogicalH: monH,
      wx,
      wy,
      luma: targetMonitor.avgLuma,
    };
  }, [targetMonitor, livePlacement, baseDims.width, baseDims.height]);

  const fitScale = Math.min(
    1,
    maxWidth / Math.max(1, baseDims.width),
    maxHeight / Math.max(1, baseDims.height),
  );
  const scaledW = Math.max(1, Math.round(baseDims.width * fitScale));
  const scaledH = Math.max(1, Math.round(baseDims.height * fitScale));

  const plateUrl = toAssetUrl(targetMonitor?.platePath ?? "");

  const shellClusterFrame = useMemo<ClusterFrameInfo | null>(
    () =>
      clusterFrame
        ? {
            ...clusterFrame,
            onHeightTransitionEnd: () => transitionEndRef.current?.(),
          }
        : null,
    [clusterFrame],
  );

  const renderWidget = () => {
    switch (kind) {
      case "clock":
        return (
          <Clock
            now={now}
            settings={clockSettings}
            size={size === "l" ? "m" : size}
            theme={effectiveTheme}
          />
        );
      case "nowPlaying":
        return (
          <NowPlaying
            size={size}
            theme={effectiveTheme}
            settings={resolveNowPlayingSettings(settings)}
            pausedApp={false}
            onAmbientChange={handleAmbientChange}
          />
        );
      case "system":
        return (
          <System
            size={size}
            theme={effectiveTheme}
            settings={resolveSystemSettings(settings)}
          />
        );
      case "weather":
        return (
          <Weather
            size={size === "pill" ? "s" : size}
            theme={effectiveTheme}
            settings={resolveWeatherSettings(settings)}
            locations={locations}
          />
        );
      case "note":
        return (
          <Note
            id={id}
            settings={resolveNoteSettings(settings)}
            editMode={false}
            preview
          />
        );
      case "discord":
        return (
          <Discord
            size={size === "l" ? "m" : size}
          />
        );
      case "agents":
        return <Agents size={size === "pill" ? "s" : size} />;
      case "cluster":
        if (!cluster) return null;
        return (
          <ClusterShell
            instance={syntheticInstance}
            cluster={cluster}
            theme={effectiveTheme}
            motion={appearance.motion}
            editMode={false}
            pausedApp={false}
            locations={locations}
            preview
            onClusterFrameChange={handleClusterFrameChange}
            onAmbientChange={handleAmbientChange}
            onAriaChange={() => {}}
            registerTransitionEnd={registerTransitionEnd}
          />
        );
      default:
        return null;
    }
  };

  const inner = (
    <div
      key={retriggerKey}
      className="cc-preview-box"
      style={{
        width: `${scaledW}px`,
        height: `${scaledH}px`,
      }}
    >
      <div
        className="cc-preview-scale"
        style={{
          width: `${baseDims.width}px`,
          height: `${baseDims.height}px`,
          transform: `scale(${fitScale.toFixed(4)})`,
          transformOrigin: "top left",
          pointerEvents: interactive ? "auto" : "none",
        }}
      >
        <WidgetShell
          size={size}
          theme={effectiveTheme}
          engine={previewEngine}
          radius={appearance.radius}
          motion={appearance.motion}
          glassStrength={effectiveGlass}
          accent={appearance.accent}
          autoContrast={appearance.autoContrast}
          startDelayMs={0}
          kind={kind}
          paused={false}
          preview
          ambientColor={ambientColor}
          glass={plateInfo}
          clusterFrame={shellClusterFrame}
        >
          {renderWidget()}
        </WidgetShell>
      </div>
    </div>
  );

  if (stageHeight === undefined) {
    return inner;
  }

  return (
    <div
      className={`cc-preview-stage${className ? ` ${className}` : ""}`}
      style={{ height: `${stageHeight}px` }}
    >
      {plateUrl && (
        <div
          className="cc-preview-bg"
          aria-hidden="true"
          style={{ backgroundImage: `url("${plateUrl}")` }}
        />
      )}
      {inner}
    </div>
  );
}
