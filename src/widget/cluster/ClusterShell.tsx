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
import { resolveClockSettings } from "../kinds/clock/Clock";
import { useClock } from "../kinds/clock/useClock";
import {
  computeAgentsExpandedHeight,
  filterAgentSessions,
  resolveAgentsSettings,
} from "../kinds/agents/Agents";
import { computeDiscordExpandedHeight } from "../kinds/discord/Discord";
import { resolveNowPlayingSettings } from "../kinds/nowPlaying/NowPlaying";
import { useSystemHistory } from "../kinds/system/useSystemHistory";
import { resolveWeatherSettings } from "../kinds/weather/Weather";
import type { ClusterFrameInfo } from "../WidgetShell";
import { ClusterModuleView, ClusterPillBarHeader, type ClusterModuleSharedData } from "./CompactModules";
import { useHysteresisSignal } from "./useHysteresisSignal";
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

function shouldReduceMotion(motion: MotionPreset): boolean {
  if (motion === "off") return true;
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
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
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [hasSystem]);

  const [manualExpandedId, setManualExpandedId] = useState<string | null>(null);

  // Timer-moduler rapporterer pin-tilstand via shared.timerPin; klyngen er
  // pinnet når mindst én timer-modul er det (finished eller fokuseret input).
  const timerPinsRef = useRef(new Set<string>());
  const [anyTimerPinned, setAnyTimerPinned] = useState(false);
  const reportTimerPin = useCallback((moduleId: string, pinned: boolean) => {
    const pins = timerPinsRef.current;
    if (pinned) pins.add(moduleId);
    else pins.delete(moduleId);
    setAnyTimerPinned(pins.size > 0);
  }, []);
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
      }
    }
    return map;
  }, [cluster.modules, modes, rawAgentsSessions, agentsNowMs, discordSnapshot]);

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

  // 2-trins udvid/kollaps-håndtryk mod Win32 (plan/04b §2.1)
  const [boxHeightU, setBoxHeightU] = useState<number>(initialIdleHeight);
  const [transitionKind, setTransitionKind] = useState<"expand" | "collapse" | "idle">("idle");
  const winHeightRef = useRef<number>(initialIdleHeight);
  const collapseTimerRef = useRef<number | null>(null);
  const pendingCollapseTargetRef = useRef<number | null>(null);
  const resizeSeqRef = useRef(0);

  const commitWinSize = useCallback(
    (nextH: number) => {
      winHeightRef.current = nextH;
      pendingCollapseTargetRef.current = null;
      if (collapseTimerRef.current !== null) {
        window.clearTimeout(collapseTimerRef.current);
        collapseTimerRef.current = null;
      }
      setTransitionKind("idle");
      if (preview) return;
      void ipc
        .clusterSetWindowSize(instance.id, cluster.width, nextH, cluster.anchor)
        .catch(() => {});
    },
    [instance.id, cluster.width, cluster.anchor, preview],
  );

  const liveBackdropRef = useRef(liveBackdrop);
  useEffect(() => {
    liveBackdropRef.current = liveBackdrop;
  }, [liveBackdrop]);

  useEffect(() => {
    registerTransitionEnd(() => {
      const target = pendingCollapseTargetRef.current;
      if (target !== null) {
        commitWinSize(target);
      } else {
        setTransitionKind("idle");
        if (liveBackdropRef.current && !preview) {
          void ipc
            .widgetAnimateBackdropBox(instance.id, 0, 0, cluster.anchor, 0)
            .catch(() => {});
        }
      }
    });
    return () => registerTransitionEnd(null);
  }, [commitWinSize, registerTransitionEnd, preview, instance.id, cluster.anchor]);

  useEffect(() => {
    if (preview) {
      winHeightRef.current = targetHeightU;
      return;
    }

    if (cluster.resizeBehavior === "fixed") {
      if (winHeightRef.current !== targetHeightU) {
        commitWinSize(targetHeightU);
      }
      return;
    }

    const currentWinH = winHeightRef.current;
    if (targetHeightU === currentWinH && boxHeightU === targetHeightU) {
      return;
    }

    if (collapseTimerRef.current !== null) {
      window.clearTimeout(collapseTimerRef.current);
      collapseTimerRef.current = null;
    }
    pendingCollapseTargetRef.current = null;

    if (targetHeightU > currentWinH) {
      // Udvidelse: udvid Win32-vindue FØR CSS-animation
      const seq = ++resizeSeqRef.current;
      const fromH = boxHeightU;
      winHeightRef.current = targetHeightU;
      const frame = window.requestAnimationFrame(() => {
        setTransitionKind("expand");
      });

      if (shouldReduceMotion(motion)) {
        void ipc
          .clusterSetWindowSize(instance.id, cluster.width, targetHeightU, cluster.anchor)
          .catch(() => {})
          .finally(() => {
            if (resizeSeqRef.current === seq) {
              setBoxHeightU(targetHeightU);
            }
          });
        return () => window.cancelAnimationFrame(frame);
      }

      void ipc
        .clusterSetWindowSize(instance.id, cluster.width, targetHeightU, cluster.anchor)
        .catch(() => {})
        .finally(() => {
          if (resizeSeqRef.current === seq) {
            setBoxHeightU(targetHeightU);
            if (liveBackdrop && Math.abs(fromH - targetHeightU) >= 0.25) {
              void ipc
                .widgetAnimateBackdropBox(instance.id, fromH, targetHeightU, cluster.anchor, 340)
                .catch(() => {});
            }
          }
        });
      return () => window.cancelAnimationFrame(frame);
    }

    if (targetHeightU < currentWinH) {
      // Kollaps: start CSS-overgang først, krymp Win32 efter transitionend / 300ms fallback
      if (shouldReduceMotion(motion)) {
        const frame = window.requestAnimationFrame(() => {
          setTransitionKind("idle");
          setBoxHeightU(targetHeightU);
          commitWinSize(targetHeightU);
        });
        return () => window.cancelAnimationFrame(frame);
      }

      const fromH = boxHeightU;
      const frame = window.requestAnimationFrame(() => {
        setTransitionKind("collapse");
        setBoxHeightU(targetHeightU);
        pendingCollapseTargetRef.current = targetHeightU;
        if (liveBackdrop && Math.abs(fromH - targetHeightU) >= 0.25) {
          void ipc
            .widgetAnimateBackdropBox(instance.id, fromH, targetHeightU, cluster.anchor, 280)
            .catch(() => {});
        }
        collapseTimerRef.current = window.setTimeout(() => {
          const target = pendingCollapseTargetRef.current;
          if (target !== null) {
            commitWinSize(target);
          }
        }, 300);
      });
      return () => window.cancelAnimationFrame(frame);
    }

    // Hvis currentWinH allerede matcher (fx gen-afbrudt kollaps), sæt boxHeight direkte
    const fromH = boxHeightU;
    const frame = window.requestAnimationFrame(() => {
      setTransitionKind("expand");
      setBoxHeightU(targetHeightU);
      if (liveBackdrop && !shouldReduceMotion(motion) && Math.abs(fromH - targetHeightU) >= 0.25) {
        void ipc
          .widgetAnimateBackdropBox(instance.id, fromH, targetHeightU, cluster.anchor, 340)
          .catch(() => {});
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [targetHeightU, cluster.resizeBehavior, cluster.width, cluster.anchor, instance.id, motion, commitWinSize, boxHeightU, preview, liveBackdrop]);

  useEffect(() => {
    return () => {
      if (collapseTimerRef.current !== null) window.clearTimeout(collapseTimerRef.current);
    };
  }, []);

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
    timerPin: reportTimerPin,
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
