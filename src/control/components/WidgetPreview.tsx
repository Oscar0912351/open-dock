import { memo, useCallback, useMemo, useRef, useState } from "react";
import type { Appearance } from "../../shared/bindings/Appearance";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { GlassEngine } from "../../shared/bindings/GlassEngine";
import type { Location } from "../../shared/bindings/Location";
import type { MonitorInfo } from "../../shared/bindings/MonitorInfo";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import type { WidgetKind } from "../../shared/bindings/WidgetKind";
import type { WidgetPlacement } from "../../shared/bindings/WidgetPlacement";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { useLocale } from "../../shared/i18n";
import { toAssetUrl } from "../../shared/ipc";
import { ClusterShell } from "../../widget/cluster/ClusterShell";
import { computeClusterTargetHeight, DEFAULT_CLUSTER_SIGNALS, resolveModuleModes } from "../../widget/cluster/resolveLayout";
import { renderWidget, WIDGET_REGISTRY } from "../../widget/registry";
import { WidgetShell, WIDGET_SIZES, type ClusterFrameInfo, type GlassPlacementInfo } from "../../widget/WidgetShell";

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
  /** Cluster previews only: show every module compact instead of the idle collapse. */
  previewShowAll?: boolean;
  retriggerKey?: string | number;
  className?: string;
}

export const WidgetPreview = memo(function WidgetPreview({
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
  previewShowAll = false,
  retriggerKey,
  className = "",
}: WidgetPreviewProps) {
  useLocale();
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

  // Forhåndsvisning viser den valgte motor ensartet på tværs af alle widgets i scenen.
  const previewEngine: GlassEngine = appearance.glassEngine;

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
      revealOnHover: false,
      themeOverride,
      glassOverride,
      glassOverrideEngine: null,
      settings: settings ?? defaultSettings,
      cluster,
      createdAt: 0,
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
    return WIDGET_SIZES[size];
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

  const renderContent = () => {
    if (kind === "cluster") {
      if (!cluster) return null;
      return (
        <ClusterShell
          instance={syntheticInstance}
          cluster={cluster}
          theme={effectiveTheme}
          motion={appearance.motion}
          editMode={false}
          pausedApp={true}
          locations={locations}
          preview
          previewShowAll={previewShowAll}
          onClusterFrameChange={handleClusterFrameChange}
          onAmbientChange={handleAmbientChange}
          onAriaChange={() => {}}
          registerTransitionEnd={registerTransitionEnd}
        />
      );
    }
    return renderWidget(kind, {
      id,
      size,
      theme: effectiveTheme,
      settings: settings ?? {},
      editMode: false,
      preview: true,
      pausedApp: true,
      locations,
      onAmbientChange: handleAmbientChange,
    });
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
          {renderContent()}
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
});
