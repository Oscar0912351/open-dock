import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { Location } from "../../shared/bindings/Location";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { MotionPreset } from "../../shared/bindings/MotionPreset";
import type { SystemCaps } from "../../shared/bindings/SystemCaps";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { WeatherSnapshot } from "../../shared/bindings/WeatherSnapshot";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import { useProvider } from "../../shared/hooks/useProvider";
import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { reportFailure } from "../../shared/logging";
import { resolveClockSettings } from "../kinds/clock/Clock";
import { useClock } from "../kinds/clock/useClock";
import {
  computeAgentsExpandedHeight,
  filterAgentSessions,
  resolveAgentsSettings,
} from "../kinds/agents/Agents";
import { computeDiscordExpandedHeight } from "../kinds/discord/Discord";
import { resolveNowPlayingSettings } from "../kinds/nowPlaying/NowPlaying";
import { computeTimerExpandedHeight } from "../kinds/timer/Timer";
import { useClusterTimers } from "../kinds/timer/useTimers";
import { useSystemHistory } from "../kinds/system/useSystemHistory";
import { resolveWeatherSettings } from "../kinds/weather/Weather";
import type { ClusterFrameInfo } from "../WidgetShell";
import { ClusterModuleView, ClusterPillBarHeader, type ClusterModuleSharedData } from "./CompactModules";
import { useHysteresisSignal } from "./useHysteresisSignal";
import { shouldReduceMotion, useClusterWindowResize } from "./useClusterWindowResize";
import { useMediaActivitySignal } from "./useMediaActivity";
import {
  computeClusterTargetHeight,
  DEFAULT_CLUSTER_SIGNALS,
  nextManualExpandedId,
  resolveModuleModes,
} from "./resolveLayout";
import "./cluster.css";

interface ClusterShellProps {
  instance: WidgetInstance;
  cluster: ClusterConfig;
  theme: ThemeId;
  motion: MotionPreset;
  editMode: boolean;
  pausedApp?: boolean;
  liveBackdrop?: boolean;
  locations: Location[];
  preview?: boolean;
  /** Om musen er over widget-vinduet (Rust hover-poll) — driver
      showOnHover-moduler. */
  clusterHover?: boolean;
  /** Preview-only: render every module compact so the card shows the full config. */
  previewShowAll?: boolean;
  onClusterFrameChange: (frame: ClusterFrameInfo | null) => void;
  onAmbientChange: (rgb: [number, number, number] | null) => void;
  onAriaChange: (aria: string | undefined) => void;
  /** Aggregateret pin fra timer-moduler — holder klyngen synlig under
      "vis kun når musen er over". */
  onPinVisibleChange?: (pinned: boolean) => void;
  registerTransitionEnd: (handler: (() => void) | null) => void;
}

function runViewTransition(motion: MotionPreset, preview: boolean, update: () => void) {
  if (preview || shouldReduceMotion(motion) || typeof document.startViewTransition !== "function") {
    update();
    return;
  }
  try {
    const transition = document.startViewTransition(() => {
      flushSync(update);
    });
    void transition?.ready?.catch(() => {});
    void transition?.finished?.catch(() => {});
  } catch {
    update();
  }
}

export function ClusterShell({
  instance,
  cluster,
  theme,
  motion,
  editMode,
  pausedApp = false,
  liveBackdrop = false,
  locations,
  preview = false,
  clusterHover = false,
  previewShowAll = false,
  onClusterFrameChange,
  onAmbientChange,
  onAriaChange,
  onPinVisibleChange,
  registerTransitionEnd,
}: ClusterShellProps) {
  const hasMedia = cluster.modules.some((m) => m.kind === "nowPlaying");
  const hasSystem = cluster.modules.some((m) => m.kind === "system");
  const hasDiscord = cluster.modules.some((m) => m.kind === "discord");
  const hasAgents = cluster.modules.some((m) => m.kind === "agents");
  const firstWeather = cluster.modules.find((m) => m.kind === "weather");
  const hasClock = cluster.modules.some((m) => m.kind === "clock");
  const needsSeconds = cluster.modules.some(
    (m) => m.kind === "clock" && resolveClockSettings(m.settings).showSeconds,
  );

  const weatherLocId = firstWeather
    ? (resolveWeatherSettings(firstWeather.settings).locationId ?? locations[0]?.id ?? null)
    : null;

  const now = useClock(hasClock && needsSeconds, hasClock);
  const media = useProvider("media", null, hasMedia);
  const system = useProvider("system", null, hasSystem);
  const discord = useProvider("discord", null, hasDiscord);
  const agents = useProvider("agents", null, hasAgents);
  const weather = useProvider("weather", weatherLocId, Boolean(firstWeather));

  const [systemCaps, setSystemCaps] = useState<SystemCaps | null>(null);

  useEffect(() => {
    if (!hasSystem) return;
    let active = true;
    ipc
      .systemCaps()
      .then((caps) => {
        if (active) setSystemCaps(caps);
      })
      .catch((reason: unknown) => reportFailure("system_caps", reason));
    return () => {
      active = false;
    };
  }, [hasSystem]);

  const [manualExpandedId, setManualExpandedId] = useState<string | null>(null);

  // Timer-snapshots hentes her i shell (ikke i selve modulet), så de er
  // levende selv når timer-modulet er hidden — finished timere skal kunne
  // pinne klyngen og auto-udvide modulet uanset synlighed.
  const timerModuleIds = useMemo(
    () => cluster.modules.filter((m) => m.kind === "timer").map((m) => m.id),
    [cluster.modules],
  );
  const { snapshots: timerSnapshots, nowMs: timerNowMs } = useClusterTimers(
    timerModuleIds,
    !preview && timerModuleIds.length > 0,
  );

  // Moduler med aktivt input (timer/note) rapporterer fokus via
  // shared.onModuleFocus — et showOnHover-modul kollapser ikke mens der skrives.
  const focusedRef = useRef(new Set<string>());
  const [focusedModuleIds, setFocusedModuleIds] = useState<ReadonlySet<string>>(() => new Set());
  const reportModuleFocus = useCallback((moduleId: string, focused: boolean) => {
    const set = focusedRef.current;
    if (focused) set.add(moduleId);
    else set.delete(moduleId);
    setFocusedModuleIds(new Set(set));
  }, []);

  const timerActiveModuleIds = useMemo(() => {
    const active = new Set<string>();
    for (const id of timerModuleIds) {
      if ((timerSnapshots.get(id)?.items.length ?? 0) > 0) active.add(id);
    }
    return active;
  }, [timerModuleIds, timerSnapshots]);

  const timerFinishedModuleIds = useMemo(() => {
    const finished = new Set<string>();
    for (const id of timerModuleIds) {
      const snap = timerSnapshots.get(id);
      if (snap?.items.some((item) => item.status === "finished")) finished.add(id);
    }
    return finished;
  }, [timerModuleIds, timerSnapshots]);

  // Klyngen er pinnet ("vis kun når musen er over") når mindst én timer er
  // sat (kørende, pauset eller færdig) eller et timer-input har fokus.
  const anyTimerPinned =
    timerActiveModuleIds.size > 0 ||
    timerFinishedModuleIds.size > 0 ||
    timerModuleIds.some((id) => focusedModuleIds.has(id));
  useEffect(() => {
    onPinVisibleChange?.(anyTimerPinned);
  }, [anyTimerPinned, onPinVisibleChange]);
  useEffect(() => () => onPinVisibleChange?.(false), [onPinVisibleChange]);

  const motionRef = useRef(motion);
  useEffect(() => {
    motionRef.current = motion;
  }, [motion]);

  const applyTransition = useCallback(
    (update: () => void) => runViewTransition(motionRef.current, preview, update),
    [preview],
  );

  const clearManualExpansion = useCallback(() => setManualExpandedId(null), []);
  const clearDiscordManualExpansion = useCallback(() => {
    setManualExpandedId((prev) =>
      prev && cluster.modules.some((m) => m.id === prev && m.kind === "discord") ? null : prev,
    );
  }, [cluster.modules]);

  // Afspiller: aktiv mens den spiller eller i 10 s efter pause-start
  const mediaActive = useMediaActivitySignal({
    enabled: hasMedia,
    status: media.data?.status,
    apply: applyTransition,
    onEnter: clearManualExpansion,
  });

  // System: CPU eller GPU >= 80 % i mindst 3 sekunder
  const systemHighLoad = useHysteresisSignal({
    input:
      hasSystem &&
      ((system.data?.cpu.usage ?? 0) >= 80 || (system.data?.gpu?.usage ?? 0) >= 80),
    enterMs: 3_000,
    apply: applyTransition,
  });

  // Agenter: mindst én arbejder eller venter (rolig 4 s kollaps-hysterese)
  const agentsActive = useHysteresisSignal({
    input:
      hasAgents &&
      Boolean(
        agents.data?.sessions.some((s) => s.status === "working" || s.status === "waiting"),
      ),
    exitMs: 4_000,
    apply: applyTransition,
  });

  // Discord: aktiv mens du er i et call
  const discordInCall = useHysteresisSignal({
    input: hasDiscord && Boolean(discord.data?.inCall),
    apply: applyTransition,
    onEnter: clearManualExpansion,
    onExit: clearDiscordManualExpansion,
  });

  // Hover: driver showOnHover-moduler. Edit-mode tæller som hover så
  // indholdet kan redigeres. Når hover ophører ryddes en manuel udvidelse
  // af et ufokuseret showOnHover-modul, så det kan glide tilbage i hidden.
  const clearShowOnHoverManual = useCallback(() => {
    setManualExpandedId((prev) =>
      prev &&
      cluster.modules.some((m) => m.id === prev && m.autoRule === "showOnHover") &&
      !focusedRef.current.has(prev)
        ? null
        : prev,
    );
  }, [cluster.modules]);

  const hoverActive = useHysteresisSignal({
    input: !preview && (clusterHover || editMode),
    apply: applyTransition,
    onExit: clearShowOnHoverManual,
  });

  // En nyligt færdig timer tager udvidelsespladsen rent: ryd manuel
  // udvidelse så timeren selv står som expanded.
  useHysteresisSignal({
    input: timerFinishedModuleIds.size > 0,
    apply: applyTransition,
    onEnter: clearManualExpansion,
  });

  const [agentsNowMs, setAgentsNowMs] = useState(() => Date.now());
  const rawAgentsSessions = agents.data?.sessions;
  const hasTimedAgents = Boolean(
    hasAgents && rawAgentsSessions?.some((s) => s.status === "ready" || s.status === "idle"),
  );
  useEffect(() => {
    if (!hasTimedAgents || preview) return;
    const id = window.setInterval(() => {
      if (!document.hidden) setAgentsNowMs(Date.now());
    }, 10_000);
    return () => window.clearInterval(id);
  }, [hasTimedAgents, preview]);

  const isPillBar = cluster.presetId === "pillBar";

  const modes = useMemo(
    () => {
      if (previewShowAll) {
        return new Map(cluster.modules.map((m) => [m.id, "compact" as ModuleMode]));
      }
      return resolveModuleModes(
        cluster.modules,
        {
          mediaActive: hasMedia && mediaActive,
          systemHighLoad: hasSystem && systemHighLoad,
          agentsActive: hasAgents && agentsActive,
          discordInCall: hasDiscord && discordInCall,
          clusterHover: hoverActive,
          focusedModuleIds,
          timerActiveModuleIds,
          timerFinishedModuleIds,
        },
        manualExpandedId,
        cluster.resizeBehavior,
        isPillBar,
      );
    },
    [
      cluster.modules,
      previewShowAll,
      hasMedia,
      mediaActive,
      hasSystem,
      systemHighLoad,
      hasAgents,
      agentsActive,
      hasDiscord,
      discordInCall,
      hoverActive,
      focusedModuleIds,
      timerActiveModuleIds,
      timerFinishedModuleIds,
      manualExpandedId,
      cluster.resizeBehavior,
      isPillBar,
    ],
  );

  const systemExpanded = useMemo(
    () => cluster.modules.some((m) => m.kind === "system" && modes.get(m.id) === "expanded"),
    [cluster.modules, modes],
  );
  const systemHistory = useSystemHistory(hasSystem && systemExpanded, system.data);

  // Beregn initial Rust-højde så første boot ikke trigger unødig resize
  const initialIdleHeight = useMemo(() => {
    const idleModes = resolveModuleModes(
      cluster.modules,
      DEFAULT_CLUSTER_SIGNALS,
      null,
      cluster.resizeBehavior,
    );
    return computeClusterTargetHeight(
      cluster.modules,
      idleModes,
      cluster.presetId,
      cluster.resizeBehavior,
      cluster.maxHeight,
    );
  }, [cluster.modules, cluster.presetId, cluster.resizeBehavior, cluster.maxHeight]);

  const discordSnapshot = discord.data;
  const heightOverrides = useMemo(() => {
    const map = new Map<string, number>();
    const sessions = rawAgentsSessions ?? [];
    for (const mod of cluster.modules) {
      if ((modes.get(mod.id) ?? mod.defaultMode) !== "expanded") continue;
      if (mod.kind === "agents") {
        const { recentWindowMin } = resolveAgentsSettings(mod.settings);
        const visible = filterAgentSessions(sessions, recentWindowMin, agentsNowMs);
        map.set(mod.id, computeAgentsExpandedHeight(visible));
      } else if (mod.kind === "discord") {
        map.set(mod.id, computeDiscordExpandedHeight(discordSnapshot));
      } else if (mod.kind === "timer") {
        const items = timerSnapshots.get(mod.id)?.items ?? [];
        map.set(mod.id, computeTimerExpandedHeight(items, focusedModuleIds.has(mod.id)));
      }
    }
    return map;
  }, [cluster.modules, modes, rawAgentsSessions, agentsNowMs, discordSnapshot, timerSnapshots, focusedModuleIds]);

  const visibleModuleCount = useMemo(
    () =>
      cluster.modules.filter((m) => (modes.get(m.id) ?? m.defaultMode) !== "hidden").length,
    [cluster.modules, modes],
  );

  const effectiveMaxHeight =
    cluster.resizeBehavior === "adaptive" && (heightOverrides.size > 0 || visibleModuleCount > 5)
      ? Math.max(cluster.maxHeight, 480)
      : cluster.maxHeight;

  const targetHeightU = useMemo(
    () =>
      computeClusterTargetHeight(
        cluster.modules,
        modes,
        cluster.presetId,
        cluster.resizeBehavior,
        effectiveMaxHeight,
        heightOverrides,
      ),
    [
      cluster.modules,
      modes,
      cluster.presetId,
      cluster.resizeBehavior,
      effectiveMaxHeight,
      heightOverrides,
    ],
  );

  const { boxHeightU, transitionKind } = useClusterWindowResize({
    initialIdleHeight,
    targetHeightU,
    cluster,
    instanceId: instance.id,
    motion,
    preview,
    liveBackdrop,
    registerTransitionEnd,
  });

  const hasExpandedModule = useMemo(
    () => cluster.modules.some((m) => modes.get(m.id) === "expanded"),
    [cluster.modules, modes],
  );

  const effectiveBoxHeightU =
    preview || cluster.resizeBehavior === "fixed" ? targetHeightU : boxHeightU;
  const effectiveTransitionKind =
    preview || cluster.resizeBehavior === "fixed" ? "idle" : transitionKind;

  // Opdater WidgetShell's clusterFrame
  useEffect(() => {
    onClusterFrameChange({
      width: cluster.width,
      boxHeight: effectiveBoxHeightU,
      padY: isPillBar ? 8 : 10,
      anchor: cluster.anchor,
      resizeBehavior: cluster.resizeBehavior,
      transitionKind: effectiveTransitionKind,
    });
  }, [
    cluster.width,
    cluster.anchor,
    cluster.resizeBehavior,
    effectiveBoxHeightU,
    isPillBar,
    effectiveTransitionKind,
    onClusterFrameChange,
  ]);

  // Ambient cover-farve på hele klyngen når Afspiller er synlig
  useEffect(() => {
    const mediaMod = cluster.modules.find((m) => m.kind === "nowPlaying");
    if (!mediaMod) {
      onAmbientChange(null);
      return;
    }
    const mode = modes.get(mediaMod.id) ?? "hidden";
    const ambientEnabled = resolveNowPlayingSettings(mediaMod.settings).ambient;
    if (mode !== "hidden" && ambientEnabled && media.data?.artColor) {
      onAmbientChange(media.data.artColor);
    } else {
      onAmbientChange(null);
    }
  }, [cluster.modules, modes, media.data?.artColor, onAmbientChange]);

  useEffect(() => {
    const title = cluster.presetId
      ? cluster.presetId === "pillBar"
        ? t("cluster.presets.pillBar")
        : cluster.presetId.startsWith("miniTrio")
          ? t("cluster.presets.miniTrio")
          : t("cluster.presets.allInOne")
      : t("cluster.title");
    onAriaChange(title);
  }, [cluster.presetId, onAriaChange]);

  const handleToggleExpand = useCallback(
    (moduleId: string) => {
      runViewTransition(motionRef.current, preview, () => {
        setManualExpandedId((prev) => nextManualExpandedId(prev, moduleId));
      });
    },
    [preview],
  );

  const weatherByLoc = useMemo(() => {
    const map = new Map<string, { status: string; data: WeatherSnapshot | null }>();
    if (weatherLocId) {
      map.set(weatherLocId, { status: weather.status, data: weather.data });
    }
    return map;
  }, [weatherLocId, weather.status, weather.data]);

  const shared: ClusterModuleSharedData = {
    widgetId: instance.id,
    theme,
    now,
    editMode,
    pausedApp,
    preview,
    locations,
    media: { status: media.status, data: media.data },
    system: { status: system.status, data: system.data },
    systemHistory,
    systemCaps,
    weatherByLoc,
    discord: { status: discord.status, data: discord.data },
    agents: { status: agents.status, data: agents.data },
    agentsNowMs,
    timerSnapshots,
    timerNowMs,
    onModuleFocus: reportModuleFocus,
  };

  if (isPillBar) {
    const expandedMod = cluster.modules.find((m) => modes.get(m.id) === "expanded") ?? null;
    const hasChips = cluster.modules.some(
      (m) => m.kind === "clock" || m.kind === "weather" || m.kind === "system",
    );
    return (
      <div className="cluster-content">
        {hasChips && (
          <ClusterPillBarHeader
            modules={cluster.modules}
            expandedId={expandedMod && expandedMod.kind !== "nowPlaying" ? expandedMod.id : null}
            shared={shared}
            onToggleChip={handleToggleExpand}
          />
        )}
        {expandedMod && hasExpandedModule && (
          <>
            {hasChips && <div className="c-sep" />}
            <ClusterModuleView
              mod={expandedMod}
              mode="expanded"
              shared={shared}
              onToggleExpand={handleToggleExpand}
            />
          </>
        )}
      </div>
    );
  }

  const visibleModules = cluster.modules.filter((m) => (modes.get(m.id) ?? m.defaultMode) !== "hidden");

  return (
    <div className="cluster-content">
      {visibleModules.map((mod, idx) => {
        const mode = modes.get(mod.id) ?? mod.defaultMode;
        return (
          <Fragment key={mod.id}>
            {idx > 0 && <div className="c-sep" />}
            <ClusterModuleView
              mod={mod}
              mode={mode}
              shared={shared}
              onToggleExpand={handleToggleExpand}
            />
          </Fragment>
        );
      })}
    </div>
  );
}
