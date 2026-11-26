import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { WidgetBoot } from "../shared/bindings/WidgetBoot";
import { usePaused } from "../shared/hooks/usePaused";
import { t, useLocale } from "../shared/i18n";
import { WeatherSprite } from "../shared/icons/WeatherSprite";
import { ipc } from "../shared/ipc";
import { ClusterShell } from "./cluster/ClusterShell";
import { StateError } from "./components/States";
import { useHoverReveal } from "./hooks/useHoverReveal";
import { useWidgetEvents } from "./hooks/useWidgetEvents";
import { resolveClockSettings } from "./kinds/clock/Clock";
import { useClock } from "./kinds/clock/useClock";
import { renderWidget, WIDGET_REGISTRY } from "./registry";
import {
  WidgetShell,
  type ClusterFrameInfo,
  type GlassPlacementInfo,
} from "./WidgetShell";

interface WidgetAppProps {
  id: string;
}

export function WidgetApp({ id }: WidgetAppProps) {
  useLocale();
  const [boot, setBoot] = useState<WidgetBoot | null>(null);
  const [ambientColor, setAmbientColor] = useState<[number, number, number] | null>(null);
  const [hiddenIdle, setHiddenIdle] = useState(false);
  const [ariaLabel, setAriaLabel] = useState<string | undefined>(undefined);
  const [clusterFrame, setClusterFrame] = useState<ClusterFrameInfo | null>(null);
  const { paused, setPaused } = usePaused();

  const rootRef = useRef<HTMLDivElement>(null);
  const plateRef = useRef<HTMLDivElement>(null);
  const scrimRef = useRef<HTMLDivElement>(null);
  const clusterTransitionEndRef = useRef<(() => void) | null>(null);

  const {
    hoverInside,
    idleHiddenActive,
    setPinnedVisible,
    setHoverInsideImmediate,
    handleHoverEvent,
  } = useHoverReveal({ id, boot, hiddenIdle });

  const {
    error,
    monitor,
    locations,
    selectedWidgetId,
    draggingWidgetId,
    placementState,
    liveLuma,
  } = useWidgetEvents({
    id,
    boot,
    setBoot,
    plateRef,
    scrimRef,
    setPaused,
    setHoverInsideImmediate,
    handleHoverEvent,
  });

  const registerClusterTransitionEnd = useCallback((handler: (() => void) | null) => {
    clusterTransitionEndRef.current = handler;
  }, []);

  const shellClusterFrame = useMemo<ClusterFrameInfo | null>(
    () =>
      clusterFrame
        ? {
            ...clusterFrame,
            onHeightTransitionEnd: () => clusterTransitionEndRef.current?.(),
          }
        : null,
    [clusterFrame],
  );

  const instance = boot?.instance;
  const kind = instance?.kind ?? "clock";
  const isEditing = boot?.editMode ?? false;
  const clockSettings = resolveClockSettings(kind === "clock" ? instance?.settings : undefined);
  const now = useClock(kind === "clock" && clockSettings.showSeconds, kind === "clock");

  const isSelected = selectedWidgetId === id;
  const isDragging = draggingWidgetId === id;
  const canCycleSize =
    kind === "cluster" || (WIDGET_REGISTRY[kind]?.allowedSizes.length ?? 0) > 1;

  const appearance = boot?.appearance;
  const theme = instance?.themeOverride ?? appearance?.themeId ?? "aurora";
  const engine = boot?.effectiveEngine ?? "tint";
  const liveBackdrop = boot?.liveBackdrop ?? false;
  const size = instance?.size ?? "m";
  const glassStrength = instance?.glassOverride ?? appearance?.glassStrength ?? 1.0;

  const resolvedPlacement =
    placementState ??
    (boot?.placement
      ? {
          monitor: boot.placement.monitor,
          wx: boot.placement.rect.x,
          wy: boot.placement.rect.y,
          luma: boot.placement.luma,
        }
      : null);

  const hasPerWidgetGlassOverride =
    Boolean(instance?.themeOverride) || instance?.glassOverride != null;
  const glassInfo: GlassPlacementInfo | null =
    monitor && resolvedPlacement
      ? {
          platePath: monitor.platePath,
          glassPlatePath: monitor.glassPlatePath,
          preblurred: !hasPerWidgetGlassOverride && Boolean(monitor.glassPlatePath),
          monitorLogicalW: monitor.logicalSize.width,
          monitorLogicalH: monitor.logicalSize.height,
          wx: resolvedPlacement.wx,
          wy: resolvedPlacement.wy,
          luma: resolvedPlacement.luma,
        }
      : null;

  const effectiveAmbient = kind === "nowPlaying" || kind === "cluster" ? ambientColor : null;
  const backdropVisible = !idleHiddenActive;
  const activeClusterFrame = kind === "cluster" ? shellClusterFrame : null;
  const needsWeatherSprite = kind === "weather" || kind === "cluster" || kind === "timer";

  const prevBackdropStateRef = useRef<{ live: boolean; visible: boolean }>({
    live: false,
    visible: true,
  });
  useEffect(() => {
    const prev = prevBackdropStateRef.current;
    prevBackdropStateRef.current = { live: liveBackdrop, visible: backdropVisible };
    if (!id || !liveBackdrop) return;
    if (!prev.live) {
      if (!backdropVisible) {
        void ipc.widgetSetBackdropVisible(id, false, 0).catch(() => {});
      }
      return;
    }
    if (prev.visible !== backdropVisible) {
      const reducedMotion =
        appearance?.motion === "off" ||
        (typeof window !== "undefined" &&
          typeof window.matchMedia === "function" &&
          window.matchMedia("(prefers-reduced-motion: reduce)").matches);
      void ipc
        .widgetSetBackdropVisible(id, backdropVisible, reducedMotion ? 0 : backdropVisible ? 180 : 420)
        .catch(() => {});
    }
  }, [id, liveBackdrop, backdropVisible, appearance?.motion]);

  return (
    <>
      {needsWeatherSprite && <WeatherSprite />}
      <WidgetShell
        rootRef={rootRef}
        plateRef={plateRef}
        scrimRef={scrimRef}
        size={size}
        theme={theme}
        engine={engine}
        liveBackdrop={liveBackdrop}
        liveLuma={liveBackdrop ? liveLuma : null}
        radius={appearance?.radius ?? "round"}
        motion={appearance?.motion ?? "full"}
        glassStrength={glassStrength}
        accent={appearance?.accent ?? null}
        autoContrast={appearance?.autoContrast ?? true}
        startDelayMs={boot?.startDelayMs ?? 0}
        kind={kind}
        paused={paused}
        ariaLabel={ariaLabel}
        ambientColor={effectiveAmbient}
        hiddenIdle={idleHiddenActive}
        isEditing={isEditing}
        isSelected={isSelected}
        isDragging={isDragging}
        canCycleSize={canCycleSize}
        onBeginDrag={() => {
          if (id) void ipc.editBeginDrag(id);
        }}
        onLongPressDrag={() => {
          if (id) void ipc.editLongPressDrag(id);
        }}
        onRemoveWidget={() => {
          if (id) void ipc.widgetRemove(id);
        }}
        onCycleSize={() => {
          if (id) void ipc.editCycleSize(id);
        }}
        glass={glassInfo}
        clusterFrame={activeClusterFrame}
        onContextMenu={(event) => {
          event.preventDefault();
          if (id) {
            void ipc.widgetContextMenu(id);
          }
        }}
      >
        {error ? (
          <StateError title={error} />
        ) : !instance ? null : kind === "cluster" ? (
          instance.cluster ? (
            <ClusterShell
              instance={instance}
              cluster={instance.cluster}
              theme={theme}
              motion={appearance?.motion ?? "full"}
              editMode={boot?.editMode ?? false}
              pausedApp={paused}
              liveBackdrop={liveBackdrop}
              locations={locations}
              clusterHover={hoverInside}
              onClusterFrameChange={setClusterFrame}
              onAmbientChange={setAmbientColor}
              onAriaChange={setAriaLabel}
              onPinVisibleChange={setPinnedVisible}
              registerTransitionEnd={registerClusterTransitionEnd}
            />
          ) : (
            <StateError title={t("error.widgetBoot")} />
          )
        ) : (
          renderWidget(kind, {
            id: instance.id,
            size,
            theme,
            settings: instance.settings,
            now,
            editMode: boot?.editMode ?? false,
            preview: false,
            pausedApp: paused,
            locations,
            onAriaChange: setAriaLabel,
            onAmbientChange: setAmbientColor,
            onHiddenIdleChange: setHiddenIdle,
            onPinVisibleChange: setPinnedVisible,
          })
        )}
      </WidgetShell>
    </>
  );
}
