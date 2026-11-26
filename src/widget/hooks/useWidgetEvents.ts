import { useEffect, useRef, useState, type Dispatch, type RefObject, type SetStateAction } from "react";
import type { Location } from "../../shared/bindings/Location";
import type { MonitorInfo } from "../../shared/bindings/MonitorInfo";
import type { WidgetBoot } from "../../shared/bindings/WidgetBoot";
import type { WidgetLiveLuma } from "../../shared/bindings/WidgetLiveLuma";
import { setLocale, t } from "../../shared/i18n";
import { ipc, on, toAssetUrl } from "../../shared/ipc";
import { reportError } from "../../shared/logging";
import { computeScrimAlpha } from "../WidgetShell";

interface UseWidgetEventsOptions {
  id: string;
  plateRef: RefObject<HTMLDivElement | null>;
  scrimRef: RefObject<HTMLDivElement | null>;
  boot: WidgetBoot | null;
  setBoot: Dispatch<SetStateAction<WidgetBoot | null>>;
  setPaused: (paused: boolean) => void;
  setHoverInsideImmediate: (inside: boolean) => void;
  handleHoverEvent: (inside: boolean) => void;
}

export function useWidgetEvents({
  id,
  boot,
  setBoot,
  plateRef,
  scrimRef,
  setPaused,
  setHoverInsideImmediate,
  handleHoverEvent,
}: UseWidgetEventsOptions) {
  const [error, setError] = useState<string | null>(() => (id ? null : t("error.widgetBoot")));
  const [monitor, setMonitor] = useState<MonitorInfo | null>(null);
  const [locations, setLocations] = useState<Location[]>([]);
  const [selectedWidgetId, setSelectedWidgetId] = useState<string | null>(null);
  const [draggingWidgetId, setDraggingWidgetId] = useState<string | null>(null);
  const [placementState, setPlacementState] = useState<{
    monitor: string;
    wx: number;
    wy: number;
    luma: number;
  } | null>(null);
  const [liveLuma, setLiveLuma] = useState<number | null>(null);

  const placementRef = useRef<{ monitor: string; wx: number; wy: number; luma: number } | null>(null);
  const liveLumaRef = useRef<number | null>(null);
  const liveLumaSeqRef = useRef(-1);
  const bootRef = useRef<WidgetBoot | null>(null);
  const lastDragEndRef = useRef(0);
  const draggingIdRef = useRef<string | null>(null);

  useEffect(() => {
    bootRef.current = boot;
  }, [boot]);

  useEffect(() => {
    draggingIdRef.current = draggingWidgetId;
  }, [draggingWidgetId]);

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
        setHoverInsideImmediate(nextBoot.hoverInside);
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
      })
      .catch(() => {});

    return () => {
      active = false;
    };
  }, [id, setPaused, setHoverInsideImmediate]);

  useEffect(() => {
    if (!id) return;
    let active = true;
    const unlisteners: Array<() => void> = [];

    const applyLiveLuma = (payload: WidgetLiveLuma) => {
      if (!active || payload.seq <= liveLumaSeqRef.current) return;
      liveLumaSeqRef.current = payload.seq;
      const next = payload.luma ?? null;
      liveLumaRef.current = next;
      setLiveLuma(next);
    };

    const pullLiveLuma = () => {
      if (!active) return;
      void ipc
        .widgetLiveLuma(id)
        .then(applyLiveLuma)
        .catch((reason: unknown) => {
          if (active) void reportError(`widget_live_luma (${id}): ${String(reason)}`);
        });
    };

    on.liveLuma(applyLiveLuma).then((dispose) => {
      if (active) {
        unlisteners.push(dispose);
        pullLiveLuma();
      } else {
        dispose();
      }
    }).catch((reason: unknown) => {
      void reportError(`widget://live-luma: ${String(reason)}`);
    });

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
      const scrimNode = scrimRef.current;
      if (draggingIdRef.current && scrimNode && currentBoot && liveLumaRef.current == null) {
        const theme = currentBoot.instance.themeOverride ?? currentBoot.appearance.themeId;
        const scrim = computeScrimAlpha(
          theme,
          placement.luma,
          currentBoot.appearance.autoContrast,
        );
        scrimNode.style.setProperty("--scrim-a", scrim.toFixed(4));
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
      pullLiveLuma();
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

    on.widgetHover((state) => {
      if (!active) return;
      handleHoverEvent(state.inside);
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    }).catch((reason: unknown) => {
      void reportError(`widget://hover: ${String(reason)}`);
    });

    return () => {
      active = false;
      for (const dispose of unlisteners) {
        dispose();
      }
    };
  }, [id, plateRef, scrimRef, handleHoverEvent]);

  const isEditing = boot?.editMode ?? false;

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

  return {
    error,
    monitor,
    locations,
    selectedWidgetId,
    draggingWidgetId,
    placementState,
    liveLuma,
  };
}
