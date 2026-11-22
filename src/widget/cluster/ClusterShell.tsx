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
  locations: Location[];
  preview?: boolean;
  /** Preview-only: render every module compact so the card shows the full config. */
  previewShowAll?: boolean;
  onClusterFrameChange: (frame: ClusterFrameInfo | null) => void;
  onAmbientChange: (rgb: [number, number, number] | null) => void;
  onAriaChange: (aria: string | undefined) => void;
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
  locations,
  preview = false,
  previewShowAll = false,
  onClusterFrameChange,
  onAmbientChange,
  onAriaChange,
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

  const now = useClock(hasClock && needsSeconds);
  const media = useProvider("media", null, hasMedia);
  const system = useProvider("system", null, hasSystem);
  const discord = useProvider("discord", null, hasDiscord);
  const agents = useProvider("agents", null, hasAgents);
  const weather = useProvider("weather", weatherLocId, Boolean(firstWeather));

  const rawSystemHistory = useSystemHistory(hasSystem);
  const systemHistory = useMemo(() => {
    const latest = system.data;
    if (!hasSystem || !latest) return rawSystemHistory;
    const last = rawSystemHistory[rawSystemHistory.length - 1];
    if (last && last.t === latest.t) return rawSystemHistory;
    return [...rawSystemHistory, latest].slice(-60);
  }, [hasSystem, rawSystemHistory, system.data]);

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
  const [mediaActive, setMediaActive] = useState(false);
  const [systemHighLoad, setSystemHighLoad] = useState(false);
  const [agentsActive, setAgentsActive] = useState(false);
  const [discordInCall, setDiscordInCall] = useState(false);

  const motionRef = useRef(motion);
  useEffect(() => {
    motionRef.current = motion;
  }, [motion]);

  const mediaActiveRef = useRef(false);
  useEffect(() => {
    mediaActiveRef.current = mediaActive;
  }, [mediaActive]);

  const prevMediaStatusRef = useRef<string | undefined>(undefined);
  const mediaPauseTimerRef = useRef<number | null>(null);
  const loadTimerRef = useRef<number | null>(null);
  const agentsIdleTimerRef = useRef<number | null>(null);

  // Afspiller-signal: aktiv mens den spiller eller én gang i 10 s ved pause (uden at genåbne efter kollaps)
  useEffect(() => {
    if (!hasMedia) {
      if (mediaPauseTimerRef.current !== null) {
        window.clearTimeout(mediaPauseTimerRef.current);
        mediaPauseTimerRef.current = null;
      }
      prevMediaStatusRef.current = undefined;
      if (mediaActiveRef.current) {
        const timer = window.setTimeout(() => {
          runViewTransition(motionRef.current, preview, () => setMediaActive(false));
        }, 0);
        return () => window.clearTimeout(timer);
      }
      return;
    }

    const status = media.data?.status ?? "none";

    if (status === "playing") {
      if (mediaPauseTimerRef.current !== null) {
        window.clearTimeout(mediaPauseTimerRef.current);
        mediaPauseTimerRef.current = null;
      }
      prevMediaStatusRef.current = "playing";
      if (!mediaActiveRef.current) {
        const timer = window.setTimeout(() => {
          runViewTransition(motionRef.current, preview, () => {
            setMediaActive(true);
            setManualExpandedId(null);
          });
        }, 0);
        return () => window.clearTimeout(timer);
      }
      return;
    }

    if (status === "paused") {
      const enteredPause = prevMediaStatusRef.current !== "paused";
      if (!enteredPause) {
        return;
      }

      if (mediaPauseTimerRef.current !== null) {
        window.clearTimeout(mediaPauseTimerRef.current);
      }
      mediaPauseTimerRef.current = window.setTimeout(() => {
        mediaPauseTimerRef.current = null;
        if (mediaActiveRef.current) {
          runViewTransition(motionRef.current, preview, () => setMediaActive(false));
        }
      }, 10_000);

      const timer = window.setTimeout(() => {
        prevMediaStatusRef.current = "paused";
        if (!mediaActiveRef.current) {
          runViewTransition(motionRef.current, preview, () => {
            setMediaActive(true);
            setManualExpandedId(null);
          });
        }
      }, 0);
      return () => {
        window.clearTimeout(timer);
      };
    }

    // "stopped" eller "none"
    if (mediaPauseTimerRef.current !== null) {
      window.clearTimeout(mediaPauseTimerRef.current);
      mediaPauseTimerRef.current = null;
    }
    prevMediaStatusRef.current = status;
    if (mediaActiveRef.current) {
      const timer = window.setTimeout(() => {
        runViewTransition(motionRef.current, preview, () => setMediaActive(false));
      }, 0);
      return () => window.clearTimeout(timer);
    }
  }, [hasMedia, media.data?.status, preview]);

  // System-signal: CPU >= 80 % eller GPU >= 80 % i mindst 3 sekunder
  useEffect(() => {
    if (!hasSystem) {
      if (loadTimerRef.current !== null) {
        window.clearTimeout(loadTimerRef.current);
        loadTimerRef.current = null;
      }
      return;
    }
    const cpu = system.data?.cpu.usage ?? 0;
    const gpu = system.data?.gpu?.usage ?? 0;
    const isHot = cpu >= 80 || gpu >= 80;

    if (isHot) {
      if (!systemHighLoad && loadTimerRef.current === null) {
        loadTimerRef.current = window.setTimeout(() => {
          loadTimerRef.current = null;
          runViewTransition(motionRef.current, preview, () => setSystemHighLoad(true));
        }, 3_000);
      }
    } else {
      if (loadTimerRef.current !== null) {
        window.clearTimeout(loadTimerRef.current);
        loadTimerRef.current = null;
      }
      if (systemHighLoad) {
        const timer = window.setTimeout(() => {
          runViewTransition(motionRef.current, preview, () => setSystemHighLoad(false));
        }, 0);
        return () => window.clearTimeout(timer);
      }
    }
  }, [hasSystem, system.data?.cpu.usage, system.data?.gpu?.usage, systemHighLoad, preview]);

  // Agent-signal: aktiv når mindst én agent arbejder eller venter (rolig 4 s kollaps-hysterese)
  useEffect(() => {
    if (!hasAgents) {
      if (agentsIdleTimerRef.current !== null) {
        window.clearTimeout(agentsIdleTimerRef.current);
        agentsIdleTimerRef.current = null;
      }
      if (agentsActive) {
        const timer = window.setTimeout(() => {
          runViewTransition(motionRef.current, preview, () => setAgentsActive(false));
        }, 0);
        return () => window.clearTimeout(timer);
      }
      return;
    }

    const busy = Boolean(
      agents.data?.sessions.some(
        (s) => s.status === "working" || s.status === "waiting",
      ),
    );

    if (busy) {
      if (agentsIdleTimerRef.current !== null) {
        window.clearTimeout(agentsIdleTimerRef.current);
        agentsIdleTimerRef.current = null;
      }
      if (!agentsActive) {
        const timer = window.setTimeout(() => {
          runViewTransition(motionRef.current, preview, () => setAgentsActive(true));
        }, 0);
        return () => window.clearTimeout(timer);
      }
      return;
    }

    if (agentsActive && agentsIdleTimerRef.current === null) {
      agentsIdleTimerRef.current = window.setTimeout(() => {
        agentsIdleTimerRef.current = null;
        runViewTransition(motionRef.current, preview, () => setAgentsActive(false));
      }, 4_000);
    }
  }, [hasAgents, agents.data?.sessions, agentsActive, preview]);

  useEffect(() => {
    return () => {
      prevMediaStatusRef.current = undefined;
      if (mediaPauseTimerRef.current !== null) window.clearTimeout(mediaPauseTimerRef.current);
      if (loadTimerRef.current !== null) window.clearTimeout(loadTimerRef.current);
      if (agentsIdleTimerRef.current !== null) window.clearTimeout(agentsIdleTimerRef.current);
    };
  }, []);

  // Discord-signal: aktiv mens du er i et call
  useEffect(() => {
    const active = hasDiscord && Boolean(discord.data?.inCall);
    if (active === discordInCall) return;
    const timer = window.setTimeout(() => {
      runViewTransition(motionRef.current, preview, () => {
        setDiscordInCall(active);
        setManualExpandedId((prev) => {
          if (active) return null;
          if (prev && cluster.modules.some((m) => m.id === prev && m.kind === "discord")) {
            return null;
          }
          return prev;
        });
      });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [hasDiscord, discord.data?.inCall, discordInCall, preview, cluster.modules]);

  const [agentsNowMs, setAgentsNowMs] = useState(() => Date.now());
  const rawAgentsSessions = agents.data?.sessions;
  const hasTimedAgents = Boolean(
    hasAgents && rawAgentsSessions?.some((s) => s.status === "ready" || s.status === "idle"),
  );
  useEffect(() => {
    if (!hasTimedAgents || preview) return;
    const id = window.setInterval(() => setAgentsNowMs(Date.now()), 10_000);
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

  useEffect(() => {
    registerTransitionEnd(() => {
      const target = pendingCollapseTargetRef.current;
      if (target !== null) {
        commitWinSize(target);
      } else {
        setTransitionKind("idle");
      }
    });
    return () => registerTransitionEnd(null);
  }, [commitWinSize, registerTransitionEnd]);

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

      const frame = window.requestAnimationFrame(() => {
        setTransitionKind("collapse");
        setBoxHeightU(targetHeightU);
        pendingCollapseTargetRef.current = targetHeightU;
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
    const frame = window.requestAnimationFrame(() => {
      setTransitionKind("expand");
      setBoxHeightU(targetHeightU);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [targetHeightU, cluster.resizeBehavior, cluster.width, cluster.anchor, instance.id, motion, commitWinSize, boxHeightU, preview]);

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
