import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Location } from "../shared/bindings/Location";
import type { MonitorInfo } from "../shared/bindings/MonitorInfo";
import type { WidgetBoot } from "../shared/bindings/WidgetBoot";
import { usePaused } from "../shared/hooks/usePaused";
import { setLocale, t, type Locale } from "../shared/i18n";
import { WeatherSprite } from "../shared/icons/WeatherSprite";
import { ipc, on, toAssetUrl } from "../shared/ipc";
import { reportError } from "../shared/logging";
import { ClusterShell } from "./cluster/ClusterShell";
import { StateError } from "./components/States";
import { WIDGET_REGISTRY } from "./registry";
import {
  computeScrimAlpha,
  WidgetShell,
  type ClusterFrameInfo,
  type GlassPlacementInfo,
} from "./WidgetShell";
import { Agents, resolveAgentsSettings } from "./kinds/agents/Agents";
import { Clock, resolveClockSettings } from "./kinds/clock/Clock";
import { useClock } from "./kinds/clock/useClock";
import { Discord } from "./kinds/discord/Discord";
import { Note, resolveNoteSettings } from "./kinds/note/Note";
import { NowPlaying, resolveNowPlayingSettings } from "./kinds/nowPlaying/NowPlaying";
import { resolveSystemSettings, System } from "./kinds/system/System";
import { resolveWeatherSettings, Weather } from "./kinds/weather/Weather";

interface WidgetAppProps {
  id: string;
}

export function WidgetApp({ id }: WidgetAppProps) {
  const [, setLocaleState] = useState<Locale>("da");
  const [error, setError] = useState<string | null>(() => (id ? null : t("error.widgetBoot")));
  const [boot, setBoot] = useState<WidgetBoot | null>(null);
  const [monitor, setMonitor] = useState<MonitorInfo | null>(null);
  const [locations, setLocations] = useState<Location[]>([]);
  const [ambientColor, setAmbientColor] = useState<[number, number, number] | null>(null);
  const [hiddenIdle, setHiddenIdle] = useState(false);
  const [ariaLabel, setAriaLabel] = useState<string | undefined>(undefined);
  const [clusterFrame, setClusterFrame] = useState<ClusterFrameInfo | null>(null);
  const [selectedWidgetId, setSelectedWidgetId] = useState<string | null>(null);
  const [draggingWidgetId, setDraggingWidgetId] = useState<string | null>(null);
  const [gameOverlayActive, setGameOverlayActive] = useState(false);
  const [placementState, setPlacementState] = useState<{
    monitor: string;
    wx: number;
    wy: number;
    luma: number;
  } | null>(null);
  const lastDragEndRef = useRef(0);
  const draggingIdRef = useRef<string | null>(null);
  const clusterTransitionEndRef = useRef<(() => void) | null>(null);
  const { paused, setPaused } = usePaused();

  useEffect(() => {
    draggingIdRef.current = draggingWidgetId;
  }, [draggingWidgetId]);

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
  const clockSettings = resolveClockSettings(kind === "clock" ? instance?.settings : undefined);
  const now = useClock(kind === "clock" && clockSettings.showSeconds, kind === "clock");

  const rootRef = useRef<HTMLDivElement>(null);
  const plateRef = useRef<HTMLDivElement>(null);
  const placementRef = useRef<{ monitor: string; wx: number; wy: number; luma: number } | null>(null);
  const bootRef = useRef<WidgetBoot | null>(null);

  useEffect(() => {
    bootRef.current = boot;
  }, [boot]);

  useEffect(() => {
    let active = true;
    if (!id) return;

    ipc
      .widgetReady(id)
      .then((nextBoot) => {
        if (!active) return;
        setBoot(nextBoot);
        setMonitor(nextBoot.monitor);
        setPaused(nextBoot.paused);
        setGameOverlayActive(nextBoot.gameOverlayActive);
        if (nextBoot.placement) {
          const initial = {
            monitor: nextBoot.placement.monitor,
            wx: nextBoot.placement.rect.x,
            wy: nextBoot.placement.rect.y,
            luma: nextBoot.placement.luma,
          };
          placementRef.current = initial;
          setPlacementState(initial);
        }
      })
      .catch((reason: unknown) => {
        if (!active) return;
        void reportError(`widget_ready (${id}): ${String(reason)}`);
        setError(t("error.widgetBoot"));
      });

    ipc
      .configGet()
      .then((cfg) => {
        if (!active) return;
        setLocations(cfg.weather.locations);
        setLocale(cfg.general.language);
        setLocaleState(cfg.general.language);
      })
      .catch(() => {});

    return () => {
      active = false;
    };
  }, [id, setPaused]);

  useEffect(() => {
    if (!instance || !boot) return;
    const shouldIgnore =
      (gameOverlayActive || hiddenIdle || instance.clickThrough) && !boot.editMode;
    void getCurrentWindow().setIgnoreCursorEvents(shouldIgnore).catch(() => {});
  }, [gameOverlayActive, hiddenIdle, instance, boot]);

  useEffect(() => {
    if (!id) return;
    let active = true;
    const unlisteners: Array<() => void> = [];

    on.placement((placement) => {
      if (!active || placement.id !== id) return;
      const prev = placementRef.current;
      const next = {
        monitor: placement.monitor,
        wx: placement.rect.x,
        wy: placement.rect.y,
        luma: placement.luma,
      };
      placementRef.current = next;
      if (!draggingIdRef.current) {
        setPlacementState(next);
      }

      const plate = plateRef.current;
      if (plate) {
        plate.style.setProperty("--wx", `${placement.rect.x}px`);
        plate.style.setProperty("--wy", `${placement.rect.y}px`);
      }

      const currentBoot = bootRef.current;
      const root = rootRef.current;
      if (root && currentBoot) {
        const theme = currentBoot.instance.themeOverride ?? currentBoot.appearance.themeId;
        const scrim = computeScrimAlpha(
          theme,
          placement.luma,
          currentBoot.appearance.autoContrast,
        );
        root.style.setProperty("--scrim-a", scrim.toFixed(4));
      }

      if (prev && prev.monitor !== placement.monitor) {
        void ipc.monitorsList().then((monitors) => {
          if (!active) return;
          const matched = monitors.find((m) => m.name === placement.monitor);
          if (matched) setMonitor(matched);
        });
      }
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`widget://placement: ${String(reason)}`);
    });

    on.monitors((monitors) => {
      if (!active) return;
      const targetName = placementRef.current?.monitor;
      const matched =
        (targetName ? monitors.find((m) => m.name === targetName) : undefined) ??
        monitors.find((m) => m.isPrimary) ??
        monitors[0] ??
        null;
      if (!matched) return;
      setMonitor(matched);
      const plate = plateRef.current;
      if (plate) {
        const pathToUse =
          plate.classList.contains("is-preblurred") && matched.glassPlatePath
            ? matched.glassPlatePath
            : matched.platePath;
        plate.style.setProperty("--plate", pathToUse ? `url("${toAssetUrl(pathToUse)}")` : "none");
        plate.style.setProperty("--mon-w", `${matched.logicalSize.width}px`);
        plate.style.setProperty("--mon-h", `${matched.logicalSize.height}px`);
      }
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`desktop://monitors: ${String(reason)}`);
    });

    on.glass((glassState) => {
      if (!active) return;
      setBoot((prev) =>
        prev
          ? {
              ...prev,
              appearance: glassState.appearance,
              effectiveEngine: glassState.effectiveEngine,
              liveBackdrop: glassState.liveBackdrop,
            }
          : prev,
      );
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`widget://glass: ${String(reason)}`);
    });

    on.config((config) => {
      if (!active) return;
      setLocations(config.weather.locations);
      setLocale(config.general.language);
      setLocaleState(config.general.language);
      const updated = config.widgets.find((w) => w.id === id);
      if (!updated) return;
      setBoot((prev) =>
        prev
          ? {
              ...prev,
              instance: updated,
              appearance: config.appearance,
            }
          : prev,
      );
      const root = rootRef.current;
      const cachedPlacement = placementRef.current;
      if (root && cachedPlacement) {
        const effectiveTheme = updated.themeOverride ?? config.appearance.themeId;
        const scrim = computeScrimAlpha(
          effectiveTheme,
          cachedPlacement.luma,
          config.appearance.autoContrast,
        );
        root.style.setProperty("--scrim-a", scrim.toFixed(4));
      }
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`config://changed: ${String(reason)}`);
    });

    on.editMode((state) => {
      if (!active) return;
      setBoot((prev) => (prev ? { ...prev, editMode: state.on } : prev));
      if (!state.on) {
        setSelectedWidgetId(null);
        setDraggingWidgetId(null);
      }
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`edit://mode: ${String(reason)}`);
    });

    on.editSelected((state) => {
      if (!active) return;
      setSelectedWidgetId(state.id);
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`edit://selected: ${String(reason)}`);
    });

    on.editDragging((state) => {
      if (!active) return;
      if (state.id === null && draggingIdRef.current !== null) {
        lastDragEndRef.current = Date.now();
        if (placementRef.current) {
          setPlacementState(placementRef.current);
        }
      }
      setDraggingWidgetId(state.id);
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`edit://dragging: ${String(reason)}`);
    });

    on.gameOverlay((state) => {
      if (!active) return;
      setGameOverlayActive(state.active);
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`app://game-overlay: ${String(reason)}`);
    });

    return () => {
      active = false;
      for (const dispose of unlisteners) {
        dispose();
      }
    };
  }, [id]);

  const isEditing = boot?.editMode ?? false;
  const isSelected = selectedWidgetId === id;
  const isDragging = draggingWidgetId === id;
  const canCycleSize =
    kind === "cluster" || (WIDGET_REGISTRY[kind]?.allowedSizes.length ?? 0) > 1;

  useEffect(() => {
    if (!isEditing || !id) return;

    let leftMouseDown = false;
    const handlePointerDown = (event: PointerEvent) => {
      if (event.button === 0) leftMouseDown = true;
    };
    const handlePointerUp = (event: PointerEvent) => {
      if (event.button === 0) leftMouseDown = false;
    };
    const handlePointerCancel = () => {
      leftMouseDown = false;
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      const step = event.shiftKey ? 8 : 1;
      switch (event.key) {
        case "ArrowLeft":
          event.preventDefault();
          void ipc.editNudge(id, -step, 0);
          break;
        case "ArrowRight":
          event.preventDefault();
          void ipc.editNudge(id, step, 0);
          break;
        case "ArrowUp":
          event.preventDefault();
          void ipc.editNudge(id, 0, -step);
          break;
        case "ArrowDown":
          event.preventDefault();
          void ipc.editNudge(id, 0, step);
          break;
        case "Delete":
          event.preventDefault();
          void ipc.widgetRemove(id);
          break;
        case "Escape":
          if (
            !leftMouseDown &&
            draggingIdRef.current === null &&
            Date.now() - lastDragEndRef.current > 250
          ) {
            event.preventDefault();
            void ipc.editSet(false);
          }
          break;
      }
    };

    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerCancel);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerCancel);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [isEditing, id]);

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
  const effectiveHiddenIdle = (kind === "nowPlaying" || kind === "cluster") && hiddenIdle;
  const backdropVisible = !(effectiveHiddenIdle && !isEditing);
  const activeClusterFrame = kind === "cluster" ? shellClusterFrame : null;
  const needsWeatherSprite = kind === "weather" || kind === "cluster";

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
        .widgetSetBackdropVisible(id, backdropVisible, reducedMotion ? 0 : 420)
        .catch(() => {});
    }
  }, [id, liveBackdrop, backdropVisible, appearance?.motion]);

  return (
    <>
      {needsWeatherSprite && <WeatherSprite />}
      <WidgetShell
        rootRef={rootRef}
        plateRef={plateRef}
        size={size}
        theme={theme}
        engine={engine}
        liveBackdrop={liveBackdrop}
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
        hiddenIdle={effectiveHiddenIdle && !isEditing}
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
        ) : !instance ? null : kind === "clock" ? (
          <Clock
            now={now}
            settings={clockSettings}
            size={size === "l" ? "m" : size}
            theme={theme}
          />
        ) : kind === "nowPlaying" ? (
          <NowPlaying
            size={size}
            theme={theme}
            settings={resolveNowPlayingSettings(instance.settings)}
            pausedApp={paused}
            onAmbientChange={setAmbientColor}
            onHiddenIdleChange={setHiddenIdle}
            onAriaChange={setAriaLabel}
          />
        ) : kind === "system" ? (
          <System
            size={size}
            theme={theme}
            settings={resolveSystemSettings(instance.settings)}
            onAriaChange={setAriaLabel}
          />
        ) : kind === "weather" ? (
          <Weather
            size={size}
            theme={theme}
            settings={resolveWeatherSettings(instance.settings)}
            locations={locations}
            onAriaChange={setAriaLabel}
          />
        ) : kind === "discord" ? (
          <Discord
            size={size}
            theme={theme}
            onAriaChange={setAriaLabel}
          />
        ) : kind === "agents" ? (
          <Agents
            size={size}
            theme={theme}
            settings={resolveAgentsSettings(instance.settings)}
            onAriaChange={setAriaLabel}
          />
        ) : kind === "note" ? (
          <Note
            id={instance.id}
            settings={resolveNoteSettings(instance.settings)}
            editMode={boot?.editMode ?? false}
            onAriaChange={setAriaLabel}
          />
        ) : kind === "cluster" && instance.cluster ? (
          <ClusterShell
            instance={instance}
            cluster={instance.cluster}
            theme={theme}
            motion={appearance?.motion ?? "full"}
            editMode={boot?.editMode ?? false}
            pausedApp={paused}
            liveBackdrop={liveBackdrop}
            locations={locations}
            onClusterFrameChange={setClusterFrame}
            onAmbientChange={setAmbientColor}
            onAriaChange={setAriaLabel}
            registerTransitionEnd={registerClusterTransitionEnd}
          />
        ) : (
          <StateError title={t("error.widgetBoot")} />
        )}
      </WidgetShell>
    </>
  );
}
