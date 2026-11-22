import { create } from "zustand";
import type { AppPaused } from "../shared/bindings/AppPaused";
import type { Config } from "../shared/bindings/Config";
import type { MonitorInfo } from "../shared/bindings/MonitorInfo";
import type { WidgetInstance } from "../shared/bindings/WidgetInstance";
import type { WidgetPlacement } from "../shared/bindings/WidgetPlacement";
import { setLocale, t, type Locale } from "../shared/i18n";
import { ipc, on } from "../shared/ipc";
import { reportError } from "../shared/logging";

export type ControlRoute = "gallery" | "desktop" | "wallpapers" | "settings";

export interface ControlToast {
  id: string;
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  undo?: boolean;
  durationMs: number;
}

interface PushToastInput {
  message: string;
  actionLabel?: string;
  onAction?: () => void;
  undo?: boolean;
  durationMs?: number;
}

interface ControlStore {
  config: Config | null;
  monitors: MonitorInfo[];
  placements: Record<string, WidgetPlacement>;
  route: ControlRoute;
  selectedWidgetId: string | null;
  editMode: boolean;
  visible: boolean;
  paused: AppPaused;
  isWindows11: boolean;
  appVersion: string;
  locale: Locale;
  toasts: ControlToast[];
  setRoute: (route: ControlRoute, selectId?: string | null) => void;
  setSelectedWidgetId: (id: string | null) => void;
  pushToast: (toast: PushToastInput) => void;
  dismissToast: (id: string) => void;
}

const toastTimers = new Map<string, number>();

function parseRouteNavigation(raw: string): { route: ControlRoute; selectId?: string } {
  const [pathPart = "", queryPart = ""] = raw.split("?");
  const cleaned = pathPart.replace(/^\//, "").trim();
  const route: ControlRoute =
    cleaned === "desktop" || cleaned === "themes"
      ? "desktop"
      : cleaned === "wallpapers" || cleaned === "settings" || cleaned === "gallery"
        ? cleaned
        : "gallery";
  const params = new URLSearchParams(queryPart);
  const selectId = params.get("select") ?? undefined;
  return { route, selectId };
}

function readInitialRoute(): { route: ControlRoute; selectedWidgetId: string | null } {
  const raw = new URLSearchParams(window.location.search).get("route");
  if (!raw) return { route: "gallery", selectedWidgetId: null };
  const parsed = parseRouteNavigation(raw);
  return { route: parsed.route, selectedWidgetId: parsed.selectId ?? null };
}

const initialRouteState = readInitialRoute();

export const useControlStore = create<ControlStore>((set, get) => ({
  config: null,
  monitors: [],
  placements: {},
  route: initialRouteState.route,
  selectedWidgetId: initialRouteState.selectedWidgetId,
  editMode: false,
  visible: true,
  paused: { paused: false, reason: null },
  isWindows11: true,
  appVersion: "0.1.0",
  locale: "da",
  toasts: [],
  setRoute: (route, selectId) => {
    set({
      route,
      selectedWidgetId: selectId !== undefined ? selectId : null,
    });
  },
  setSelectedWidgetId: (selectedWidgetId) => set({ selectedWidgetId }),
  pushToast: (input) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const durationMs = input.durationMs ?? (input.undo ? 5000 : 4000);
    const toast: ControlToast = {
      id,
      message: input.message,
      actionLabel: input.actionLabel,
      onAction: input.onAction,
      undo: input.undo,
      durationMs,
    };
    set((prev) => {
      const removed = prev.toasts.length >= 2 ? prev.toasts.slice(0, prev.toasts.length - 1) : [];
      for (const old of removed) {
        const timer = toastTimers.get(old.id);
        if (timer !== undefined) {
          window.clearTimeout(timer);
          toastTimers.delete(old.id);
        }
      }
      return { toasts: [...prev.toasts.slice(-1), toast] };
    });
    const timer = window.setTimeout(() => {
      toastTimers.delete(id);
      get().dismissToast(id);
    }, durationMs);
    toastTimers.set(id, timer);
  },
  dismissToast: (id) => {
    const timer = toastTimers.get(id);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      toastTimers.delete(id);
    }
    set((prev) => ({ toasts: prev.toasts.filter((item) => item.id !== id) }));
  },
}));

export function formatWidgetTitle(w: WidgetInstance): string {
  if (w.kind === "cluster" && w.cluster?.presetId) {
    const pid = w.cluster.presetId;
    if (pid === "pillBar") return t("cluster.presets.pillBar");
    if (pid.startsWith("miniTrio")) return t("cluster.presets.miniTrio");
    if (pid === "allInOne") return t("cluster.presets.allInOne");
  }
  return t(`widgets.${w.kind}.name`);
}

export function showUndoRemoveToast(instance: WidgetInstance) {
  useControlStore.getState().pushToast({
    message: t("toast.removed"),
    actionLabel: t("toast.undo"),
    undo: true,
    durationMs: 5000,
    onAction: () => {
      void ipc.widgetRestore(instance).catch((err) => {
        void reportError(`widget_restore: ${String(err)}`);
      });
    },
  });
}

export function initControlStore(): () => void {
  let active = true;
  const unlisteners: Array<() => void> = [];
  const store = useControlStore.getState();

  const applyConfig = (cfg: Config) => {
    setLocale(cfg.general.language);
    useControlStore.setState((prev) => {
      const stillExists =
        prev.selectedWidgetId !== null &&
        cfg.widgets.some((w) => w.id === prev.selectedWidgetId);
      return {
        config: cfg,
        locale: cfg.general.language,
        selectedWidgetId: stillExists ? prev.selectedWidgetId : null,
      };
    });
  };

  void ipc
    .configGet()
    .then((cfg) => {
      if (active) applyConfig(cfg);
    })
    .catch((err) => void reportError(`configGet: ${String(err)}`));

  void ipc
    .monitorsList()
    .then((monitors) => {
      if (active) useControlStore.setState({ monitors });
    })
    .catch((err) => void reportError(`monitorsList: ${String(err)}`));

  void ipc
    .appStatus()
    .then((status) => {
      if (!active) return;
      useControlStore.setState({
        visible: status.visible,
        paused: { paused: status.paused, reason: status.reason },
        editMode: status.editMode,
        isWindows11: status.isWindows11,
        appVersion: status.version,
      });
    })
    .catch((err) => void reportError(`appStatus: ${String(err)}`));

  void ipc
    .configTakeRestoreNotice()
    .then((notice) => {
      if (active && notice) {
        store.pushToast({ message: notice, durationMs: 5000 });
      }
    })
    .catch(() => {});

  const sub = <T,>(
    fn: (cb: (payload: T) => void) => Promise<() => void>,
    cb: (payload: T) => void,
  ) => {
    fn((payload) => {
      if (active) cb(payload);
    })
      .then((dispose) => {
        if (active) unlisteners.push(dispose);
        else dispose();
      })
      .catch((err) => void reportError(`control listen: ${String(err)}`));
  };

  sub(on.config, (cfg) => applyConfig(cfg));
  sub(on.configRestored, (msg) => {
    useControlStore.getState().pushToast({ message: msg, durationMs: 5000 });
  });
  sub(on.trayNotice, (msg) => {
    useControlStore.getState().pushToast({ message: msg, durationMs: 4000 });
  });
  sub(on.monitors, (monitors) => {
    useControlStore.setState({ monitors });
  });
  sub(on.placement, (placement) => {
    useControlStore.setState((prev) => ({
      placements: { ...prev.placements, [placement.id]: placement },
    }));
  });
  sub(on.editMode, (state) => {
    useControlStore.setState({ editMode: state.on });
  });
  sub(on.visible, (state) => {
    useControlStore.setState({ visible: state.visible });
  });
  sub(on.paused, (paused) => {
    useControlStore.setState({ paused });
  });
  sub(on.widgetRemoved, (instance) => {
    showUndoRemoveToast(instance);
  });
  sub(on.navigate, (raw) => {
    const parsed = parseRouteNavigation(raw);
    useControlStore.getState().setRoute(parsed.route, parsed.selectId);
  });

  return () => {
    active = false;
    for (const dispose of unlisteners) dispose();
  };
}
