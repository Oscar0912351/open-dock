import {
  CaretDown,
  Headset,
  CheckSquare,
  MapPin,
  MicrophoneSlash,
  MusicNotesSimple,
  Pause,
  Play,
  Repeat,
  RepeatOnce,
  Robot,
  Shuffle,
  SkipBack,
  SkipForward,
  Hourglass,
  SpeakerSlash,
  Square,
} from "@phosphor-icons/react";
import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import type { AgentsSnapshot } from "../../shared/bindings/AgentsSnapshot";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { DiscordSnapshot } from "../../shared/bindings/DiscordSnapshot";
import type { Location } from "../../shared/bindings/Location";
import type { MediaSnapshot } from "../../shared/bindings/MediaSnapshot";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { SystemCaps } from "../../shared/bindings/SystemCaps";
import type { SystemSample } from "../../shared/bindings/SystemSample";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { TimerListSnapshot } from "../../shared/bindings/TimerListSnapshot";
import type { WeatherSnapshot } from "../../shared/bindings/WeatherSnapshot";
import {
  formatBytes,
  formatDateLong,
  formatDateShort,
  formatFrequencyGhz,
  formatPercent,
  formatSpeed,
  formatCallTimer,
  formatTemperature,
  formatTime,
  timeZoneCity,
} from "../../shared/format";
import { t, useLocale } from "../../shared/i18n";
import { WeatherIcon } from "../../shared/icons/WeatherSprite";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { resolveWmo } from "../../shared/wmo";
import {
  resolveClockSettings,
  validTimeZone,
} from "../kinds/clock/Clock";
import { RollingNumber } from "../kinds/clock/RollingNumber";
import {
  parseChecklist,
  resolveNoteSettings,
  serializeChecklist,
  type ChecklistRow,
} from "../kinds/note/Note";
import { AgentIcon } from "../kinds/agents/AgentIcon";
import {
  AgentRow,
  AgentStatusDot,
  agentStatusLine,
  filterAgentSessions,
  formatAgentSummary,
  overallAgentStatus,
  resolveAgentsSettings,
  statusToneClass,
  todoRatio,
} from "../kinds/agents/Agents";
import {
  DiscordAvatarStack,
  DiscordParticipantRow,
  DiscordVoiceControls,
} from "../kinds/discord/Discord";
import { Equalizer } from "../kinds/nowPlaying/Equalizer";
import { resolveNowPlayingSettings } from "../kinds/nowPlaying/NowPlaying";
import { formatTrackTime, Progress } from "../kinds/nowPlaying/Progress";
import { useLiveMediaPosition } from "../kinds/nowPlaying/useLiveMediaPosition";
import { formatCountdown, formatOvertime } from "../kinds/timer/formatDuration";
import { sortTimers } from "../kinds/timer/sortTimers";
import {
  chipLabel,
  Chips,
  errorMessage,
  releaseTimerFocus,
  TimerInput,
  TimerRow,
  timerItemTitle,
  timerRestMs,
} from "../kinds/timer/Timer";
import { cleanTimerLabel, parseTimerInput } from "../kinds/timer/parseTimerInput";
import "../kinds/timer/timer.css";
import { Sparkline } from "../kinds/system/Sparkline";
import { resolveSystemSettings } from "../kinds/system/System";
import { WaveChart } from "../kinds/weather/WaveChart";
import { resolveWeatherSettings } from "../kinds/weather/Weather";

export interface ClusterModuleSharedData {
  widgetId: string;
  theme: ThemeId;
  now: Date;
  editMode: boolean;
  pausedApp: boolean;
  preview?: boolean;
  locations: Location[];
  media: { status: string; data: MediaSnapshot | null };
  system: { status: string; data: SystemSample | null };
  systemHistory: SystemSample[];
  systemCaps: SystemCaps | null;
  weatherByLoc: ReadonlyMap<string, { status: string; data: WeatherSnapshot | null }>;
  discord: { status: string; data: DiscordSnapshot | null };
  agents: { status: string; data: AgentsSnapshot | null };
  agentsNowMs: number;
  /** Timer-snapshots pr. timer-modul-id — hentes i shell så de er levende
      selv når modulet er hidden (pin-regler og auto-udvidelse ved ring). */
  timerSnapshots?: ReadonlyMap<string, TimerListSnapshot>;
  timerNowMs?: number;
  /** Moduler med aktivt input rapporterer fokus hit, så et showOnHover-modul
      ikke kollapser mens brugeren skriver. */
  onModuleFocus?: (moduleId: string, focused: boolean) => void;
}

interface ModuleCardProps {
  mod: ClusterModule;
  mode: ModuleMode;
  shared: ClusterModuleSharedData;
  onToggleExpand: (id: string) => void;
}

function shouldIgnoreExpand(event: MouseEvent<HTMLElement>): boolean {
  const target = event.target as HTMLElement | null;
  return Boolean(target?.closest("button, a, input, textarea, [data-no-expand]"));
}

function viewTransitionStyle(id: string, preview?: boolean): CSSProperties | undefined {
  if (preview) return undefined;
  return { viewTransitionName: `odmod-${id.replace(/[^a-zA-Z0-9_-]/g, "")}` };
}


const TimerClusterMod = memo(function TimerClusterMod({
  mod,
  mode,
  widgetId,
  editMode,
  preview,
  snapshot,
  nowMs,
  onModuleFocus,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  widgetId: string;
  editMode: boolean;
  preview?: boolean;
  snapshot: TimerListSnapshot | null;
  nowMs: number;
  onModuleFocus?: (moduleId: string, focused: boolean) => void;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const items = useMemo(() => snapshot?.items ?? [], [snapshot]);
  const sorted = useMemo(() => sortTimers(items), [items]);
  const next = sorted[0];
  const [inputText, setInputText] = useState("");
  const [inputEditing, setInputEditing] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const interactive = !editMode && !preview;
  const full = items.length >= 8;

  // Fokus rapporteres til shell, så et showOnHover-modul ikke kollapser
  // mens brugeren skriver (og klyngen forbliver pinnet synlig).
  useEffect(() => {
    onModuleFocus?.(mod.id, inputEditing);
  }, [onModuleFocus, mod.id, inputEditing]);
  useEffect(() => () => onModuleFocus?.(mod.id, false), [onModuleFocus, mod.id]);

  const startPreset = (minutes: number) => {
    if (!interactive || full) return;
    const parsed = parseTimerInput(inputText, new Date());
    const label = parsed ? parsed.label : cleanTimerLabel(inputText);
    void ipc
      .timerAdd(mod.id, label, minutes * 60_000)
      .then(() => {
        setInputText("");
        releaseTimerFocus();
      })
      .catch((e) => setAddError(errorMessage(e)));
  };

  if (mode === "compact") {
    const finished = next?.status === "finished";
    const rest = next ? timerRestMs(next, nowMs) : 0;
    const overtimeMs = next?.finishedAt != null ? Math.max(0, nowMs - next.finishedAt) : 0;
    const timeText = !next
      ? ""
      : finished
        ? formatOvertime(overtimeMs)
        : formatCountdown(rest);
    return (
      <div
        className={`c-mod is-compact c-mod-timer${finished ? " is-finished" : ""}`}
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <div className="row g8">
            <Hourglass size="calc(15 * var(--u))" aria-hidden="true" />
            {!next ? (
              <>
                <span className="label">{t("timer.name")}</span>
                <span className="micro">{t("timer.empty")}</span>
              </>
            ) : (
              <>
                <span className="label trunc">{timerItemTitle(next)}</span>
                <span className={`value tabular${finished ? " tm-overtime-ink" : ""}`}>
                  {timeText}
                </span>
                {sorted.length > 1 && <span className="micro">+{sorted.length - 1}</span>}
              </>
            )}
          </div>
          <HintBadge label={t("cluster.more")} />
        </div>
      </div>
    );
  }

  // Expanded: input øverst, derefter op til 3 rækker + "+n flere" — eller
  // hurtig-preset-chips når der ingen timere er. Højden beregnes i shell
  // (computeTimerExpandedHeight) så intet klippes.
  const finished = next?.status === "finished";
  return (
    <div
      className={`c-mod is-exp c-mod-timer-exp${finished ? " is-finished" : ""}`}
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="col g6 tm-cluster-exp">
        <TimerInput
          value={inputText}
          onValueChange={(text) => {
            setInputText(text);
            setAddError(null);
          }}
          timerId={mod.id}
          focusId={widgetId}
          placeholder={full ? t("timer.full") : t("timer.placeholder")}
          disabled={!interactive || full}
          showHint
          error={addError}
          onEditingChange={setInputEditing}
        />
        {sorted.length > 0 ? (
          <>
            {sorted.slice(0, 3).map((item) => (
              <TimerRow
                key={item.id}
                timerId={mod.id}
                item={item}
                nowMs={nowMs}
                size="c"
                interactive={interactive}
              />
            ))}
            {sorted.length > 3 && (
              <span className="micro tm-more">{t("timer.more", { n: sorted.length - 3 })}</span>
            )}
          </>
        ) : (
          <Chips className="tm-suggest">
            {[1, 3, 5, 10, 15, 30].map((minutes) => (
              <button
                key={`p${minutes}`}
                type="button"
                className="tm-chipbtn"
                disabled={!interactive || full}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => startPreset(minutes)}
              >
                {chipLabel(minutes)}
              </button>
            ))}
          </Chips>
        )}
      </div>
    </div>
  );
});
function HintBadge({ label }: { label: string }) {
  return (
    <span className="micro c-hint">
      {label}
      <CaretDown size="calc(10 * var(--u))" weight="bold" />
    </span>
  );
}

export function ClusterModuleView({ mod, mode, shared, onToggleExpand }: ModuleCardProps) {
  if (mode === "hidden") return null;

  switch (mod.kind) {
    case "clock":
      return (
        <ClockClusterMod
          mod={mod}
          mode={mode}
          now={shared.now}
          preview={shared.preview}
          onToggleExpand={onToggleExpand}
        />
      );
    case "nowPlaying":
      return (
        <MediaClusterMod
          mod={mod}
          mode={mode}
          media={shared.media.data}
          pausedApp={shared.pausedApp}
          preview={shared.preview}
          onToggleExpand={onToggleExpand}
        />
      );
    case "system":
      return (
        <SystemClusterMod
          mod={mod}
          mode={mode}
          theme={shared.theme}
          sample={shared.system.data}
          history={shared.systemHistory}
          caps={shared.systemCaps}
          preview={shared.preview}
          onToggleExpand={onToggleExpand}
        />
      );
    case "weather": {
      const settings = resolveWeatherSettings(mod.settings);
      const locId = settings.locationId ?? shared.locations[0]?.id ?? "";
      const wx = locId ? shared.weatherByLoc.get(locId) : undefined;
      return (
        <WeatherClusterMod
          mod={mod}
          mode={mode}
          theme={shared.theme}
          location={shared.locations.find((l) => l.id === locId) ?? shared.locations[0] ?? null}
          snapshot={wx?.data ?? null}
          status={wx?.status ?? (shared.locations.length === 0 ? "empty" : "loading")}
          preview={shared.preview}
          onToggleExpand={onToggleExpand}
        />
      );
    }
    case "discord":
      return (
        <DiscordClusterMod
          mod={mod}
          mode={mode}
          data={shared.discord.data}
          preview={shared.preview}
          onToggleExpand={onToggleExpand}
        />
      );
    case "agents":
      return (
        <AgentsClusterMod
          mod={mod}
          mode={mode}
          data={shared.agents.data}
          nowMs={shared.agentsNowMs}
          preview={shared.preview}
          onToggleExpand={onToggleExpand}
        />
      );
    case "timer":
      return (
        <TimerClusterMod
          mod={mod}
          mode={mode}
          widgetId={shared.widgetId}
          editMode={shared.editMode}
          preview={shared.preview}
          snapshot={shared.timerSnapshots?.get(mod.id) ?? null}
          nowMs={shared.timerNowMs ?? Date.now()}
          onModuleFocus={shared.onModuleFocus}
          onToggleExpand={onToggleExpand}
        />
      );
    case "note":
      return (
        <NoteClusterMod
          mod={mod}
          mode={mode}
          widgetId={shared.widgetId}
          editMode={shared.editMode}
          preview={shared.preview}
          onModuleFocus={shared.onModuleFocus}
          onToggleExpand={onToggleExpand}
        />
      );
    default:
      return null;
  }
}

const ClockClusterMod = memo(function ClockClusterMod({
  mod,
  mode,
  now,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  now: Date;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveClockSettings(mod.settings);
  const tz = validTimeZone(settings.timezone);
  const timeStr = formatTime(now, tz ?? undefined, settings.showSeconds);

  if (mode === "expanded") {
    const zones = (
      settings.extraZones.length > 0 ? settings.extraZones : ["Asia/Tokyo", "America/New_York"]
    )
      .filter((z): z is string => validTimeZone(z) !== null)
      .slice(0, 2);

    return (
      <div
        className="c-mod is-exp c-mod-clock-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <div className="col">
            <div className="display-m" style={{ fontSize: "calc(30 * var(--u))" }}>
              <RollingNumber value={timeStr} />
            </div>
            <div className="meta">{formatDateLong(now, tz ?? undefined)}</div>
          </div>
          <div className="col g4" style={{ alignItems: "flex-end" }}>
            {zones.map((zone) => (
              <div className="row g8" key={zone}>
                <span className="micro">{timeZoneCity(zone)}</span>
                <span className="value" style={{ fontSize: "calc(12 * var(--u))" }}>
                  {formatTime(now, zone)}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-clock-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between">
        <div className="row g8">
          <span className="display-s" style={{ fontSize: "calc(19 * var(--u))" }}>
            <RollingNumber value={timeStr} />
          </span>
          <span className="meta">{formatDateShort(now, tz ?? undefined)}</span>
        </div>
        <HintBadge label={t("cluster.more")} />
      </div>
    </div>
  );
});

const MediaClusterMod = memo(function MediaClusterMod({
  mod,
  mode,
  media,
  pausedApp,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  media: MediaSnapshot | null;
  pausedApp: boolean;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const [mountNow] = useState(() => Date.now());
  const [scrubMs, setScrubMs] = useState<number | null>(null);
  const livePosMs = useLiveMediaPosition(
    media,
    pausedApp || Boolean(preview),
    mode === "compact" ? 2000 : 1000,
  );

  const settings = resolveNowPlayingSettings(mod.settings);
  const playing = media?.status === "playing";
  const isAnimating = playing && !pausedApp && !preview;
  const coverUrl = media?.artUrl ? toAssetUrl(media.artUrl) : "";
  const sourceArg = settings.preferredSource === "auto" ? null : settings.preferredSource;
  const durMs = media ? Math.max(0, media.durationMs) : 0;
  const anchorPos = media ? Math.max(0, media.positionMs) : 0;
  const anchorUpdatedAt = media ? media.updatedAt : undefined;
  const displayPosMs = scrubMs ?? livePosMs;

  const title = media?.title || t("media.empty.title");
  const artist = media?.artist || t("media.empty.body");
  const sourceLabel = media?.source || "Spotify";
  const trackKey = `${media?.source ?? ""}-${media?.title ?? ""}-${media?.artist ?? ""}`;
  const disabledSeekHint =
    !preview && media && !media.canSeek && durMs > 0
      ? t("media.seekUnsupported", { source: sourceLabel })
      : undefined;

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-media-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row g10">
          <div
            className="c-cover"
            style={
              {
                "--cs": 48,
                "--cr": 10,
                backgroundImage: coverUrl ? `url("${coverUrl}")` : undefined,
              } as CSSProperties
            }
          >
            {!coverUrl && <MusicNotesSimple size="calc(20 * var(--u))" />}
          </div>
          <div className="col grow" style={{ minWidth: 0 }}>
            <div className="row between">
              {media && media.sessions.length > 1 ? (
                <div className="c-sources" role="group" aria-label={t("media.sources")}>
                  {media.sessions.map((session) => (
                    <button
                      key={session.source}
                      type="button"
                      className={`od-chip c-source-chip${session.source === media.source ? " is-active" : ""}`}
                      style={{ height: "calc(16 * var(--u))", fontSize: "calc(9.5 * var(--u))" }}
                      aria-pressed={session.source === media.source}
                      title={
                        session.title
                          ? `${session.title}${session.artist ? ` – ${session.artist}` : ""}`
                          : session.source
                      }
                      onClick={(e) => {
                        e.stopPropagation();
                        if (!preview) void ipc.mediaSelectSource(session.source);
                      }}
                    >
                      {session.source}
                    </button>
                  ))}
                </div>
              ) : (
                <span
                  className="od-chip"
                  style={{ height: "calc(16 * var(--u))", fontSize: "calc(9.5 * var(--u))" }}
                >
                  {sourceLabel}
                </span>
              )}
              <Equalizer playing={isAnimating} height={10} />
            </div>
            <div className="title-s trunc" style={{ marginTop: "calc(3 * var(--u))" }}>
              {title}
            </div>
            <div className="meta trunc">{artist}</div>
          </div>
        </div>

        <div className="row g8" style={{ marginTop: "calc(6 * var(--u))" }}>
          <span className="micro">
            {durMs > 0 || displayPosMs > 0 ? formatTrackTime(displayPosMs) : t("state.dash")}
          </span>
          <div className="grow" data-no-expand>
            <Progress
              positionMs={displayPosMs}
              anchorPositionMs={anchorPos}
              anchorUpdatedAt={anchorUpdatedAt}
              durationMs={durMs}
              playing={isAnimating}
              trackKey={trackKey}
              interactive={!preview && Boolean(media?.canSeek) && durMs > 0}
              disabledHint={disabledSeekHint}
              onScrubPreview={setScrubMs}
              onSeek={(ms) => {
                if (!preview) void ipc.mediaControl({ type: "seek", ms }, sourceArg);
              }}
            />
          </div>
          <span className="micro">{durMs > 0 ? formatTrackTime(durMs) : t("state.dash")}</span>
        </div>

        <div
          className="row"
          style={{ justifyContent: "center", gap: "calc(6 * var(--u))", marginTop: "calc(4 * var(--u))" }}
        >
          {media?.shuffle !== null && media?.shuffle !== undefined && (
            <button
              type="button"
              className={`od-btn ${media.shuffle ? "np-btn-active" : "np-btn-muted"}`}
              style={{ "--b": 22 } as CSSProperties}
              aria-label={t("media.shuffle")}
              aria-pressed={media.shuffle}
              onClick={(e) => {
                e.stopPropagation();
                if (!preview) void ipc.mediaControl({ type: "shuffle" }, sourceArg);
              }}
            >
              <Shuffle size="calc(11 * var(--u))" />
            </button>
          )}
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 22 } as CSSProperties}
            disabled={!preview && media !== null && !media.canPrev}
            aria-label={t("media.prev")}
            onClick={(e) => {
              e.stopPropagation();
              if (!preview) void ipc.mediaControl({ type: "previous" }, sourceArg);
            }}
          >
            <SkipBack size="calc(10 * var(--u))" weight="fill" />
          </button>
          <button
            type="button"
            className="od-btn primary"
            style={{ "--b": 26 } as CSSProperties}
            disabled={!preview && media !== null && !media.canPlayPause}
            aria-label={playing ? t("media.pause") : t("media.play")}
            onClick={(e) => {
              e.stopPropagation();
              if (!preview) void ipc.mediaControl({ type: "playPause" }, sourceArg);
            }}
          >
            {playing ? (
              <Pause size="calc(11 * var(--u))" weight="fill" />
            ) : (
              <Play size="calc(11 * var(--u))" weight="fill" />
            )}
          </button>
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 22 } as CSSProperties}
            disabled={!preview && media !== null && !media.canNext}
            aria-label={t("media.next")}
            onClick={(e) => {
              e.stopPropagation();
              if (!preview) void ipc.mediaControl({ type: "next" }, sourceArg);
            }}
          >
            <SkipForward size="calc(10 * var(--u))" weight="fill" />
          </button>
          {media?.repeat !== null && media?.repeat !== undefined && (
            <button
              type="button"
              className={`od-btn ${media.repeat !== "none" ? "np-btn-active" : "np-btn-muted"}`}
              style={{ "--b": 22 } as CSSProperties}
              aria-label={t("media.repeat")}
              aria-pressed={media.repeat !== "none"}
              onClick={(e) => {
                e.stopPropagation();
                if (!preview) void ipc.mediaControl({ type: "repeat" }, sourceArg);
              }}
            >
              {media.repeat === "track" ? (
                <RepeatOnce size="calc(11 * var(--u))" />
              ) : (
                <Repeat size="calc(11 * var(--u))" />
              )}
            </button>
          )}
        </div>
      </div>
    );
  }

  const safeDur = Math.max(1, durMs);
  const startAt =
    anchorUpdatedAt !== undefined ? Math.max(mountNow, anchorUpdatedAt) : mountNow;
  const startPos =
    isAnimating && durMs > 0
      ? Math.min(safeDur, anchorPos + Math.max(0, startAt - (anchorUpdatedAt ?? startAt)))
      : anchorPos;
  const p0 = durMs > 0 ? Math.min(1, Math.max(0, startPos / safeDur)) : 0;
  const liveRatio = durMs > 0 ? Math.min(1, Math.max(0, displayPosMs / safeDur)) : 0;
  const remaining = durMs > 0 ? Math.max(1, safeDur - startPos) : 1;

  return (
    <div
      className="c-mod c-mod-media-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row g8">
        <div
          className="c-cover"
          style={
            {
              "--cs": 26,
              "--cr": 6,
              backgroundImage: coverUrl ? `url("${coverUrl}")` : undefined,
            } as CSSProperties
          }
        >
          {!coverUrl && <MusicNotesSimple size="calc(12 * var(--u))" />}
        </div>
        <div className="col grow" style={{ minWidth: 0 }}>
          <div className="row g4" style={{ minWidth: 0 }}>
            <div className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
              {title}
            </div>
            {media && media.sessions.length > 1 && (
              <button
                type="button"
                className="od-chip c-source-count"
                aria-label={t("media.sources")}
                title={media.sessions.map((x) => x.source).join(" · ")}
                onClick={(e) => {
                  e.stopPropagation();
                  if (!preview) {
                    const idx = media.sessions.findIndex((x) => x.source === media.source);
                    const next = media.sessions[(idx + 1) % media.sessions.length];
                    if (next) void ipc.mediaSelectSource(next.source);
                  }
                }}
              >
                {media.sessions.length}
              </button>
            )}
          </div>
          <div className="micro trunc">{artist}</div>
        </div>
        <div className="c-media-toggle">
          <div className="c-media-eq">
            <Equalizer playing={isAnimating} height={10} />
          </div>
          <button
            type="button"
            className="od-btn primary c-media-hover-btn"
            style={{ "--b": 22 } as CSSProperties}
            disabled={!preview && media !== null && !media.canPlayPause}
            aria-label={playing ? t("media.pause") : t("media.play")}
            onClick={(e) => {
              e.stopPropagation();
              if (!preview) void ipc.mediaControl({ type: "playPause" }, sourceArg);
            }}
          >
            {playing ? (
              <Pause size="calc(10 * var(--u))" weight="fill" />
            ) : (
              <Play size="calc(10 * var(--u))" weight="fill" />
            )}
          </button>
        </div>
      </div>
      <div className="c-media-prog-edge" aria-hidden="true">
        {durMs > 0 && (
          <i
            className={isAnimating ? "is-playing" : undefined}
            style={
              {
                "--p0": p0.toFixed(5),
                "--p-live": liveRatio.toFixed(5),
                "--rest": `${remaining}ms`,
              } as CSSProperties
            }
          />
        )}
      </div>
    </div>
  );
});

const SystemClusterMod = memo(function SystemClusterMod({
  mod,
  mode,
  theme,
  sample,
  history,
  caps,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  theme: ThemeId;
  sample: SystemSample | null;
  history: SystemSample[];
  caps: SystemCaps | null;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveSystemSettings(mod.settings);
  const cpuPct = sample ? Math.round(sample.cpu.usage) : 0;
  const memPct = sample && sample.mem.totalBytes > 0
    ? Math.round((sample.mem.usedBytes / sample.mem.totalBytes) * 100)
    : 0;
  const memGb = sample ? formatBytes(sample.mem.usedBytes) : t("state.dash");
  const gpuSupported = caps?.gpu ?? Boolean(sample?.gpu);
  const gpuPct = sample?.gpu?.usage !== null && sample?.gpu?.usage !== undefined
    ? Math.round(sample.gpu.usage)
    : null;
  const netDown = sample
    ? formatSpeed(sample.net.rxBps, settings.unitsBits)
    : t("state.dash");

  const [barSmooth, setBarSmooth] = useState<{
    cpu: number;
    mem: number;
    gpu: number;
    smoothCpu: boolean;
    smoothMem: boolean;
    smoothGpu: boolean;
  }>({ cpu: cpuPct, mem: memPct, gpu: gpuPct ?? 0, smoothCpu: false, smoothMem: false, smoothGpu: false });

  const gpuNum = gpuPct ?? 0;
  if (barSmooth.cpu !== cpuPct || barSmooth.mem !== memPct || barSmooth.gpu !== gpuNum) {
    setBarSmooth({
      cpu: cpuPct,
      mem: memPct,
      gpu: gpuNum,
      smoothCpu: Math.abs(cpuPct - barSmooth.cpu) >= 3,
      smoothMem: Math.abs(memPct - barSmooth.mem) >= 3,
      smoothGpu: Math.abs(gpuNum - barSmooth.gpu) >= 3,
    });
  }

  const barClass = (pct: number, smooth: boolean) => {
    const tone = pct >= 95 ? "is-crit" : pct >= 85 ? "is-warn" : "";
    return [tone, smooth ? "is-smooth" : ""].filter(Boolean).join(" ");
  };

  if (mode === "expanded") {
    const pts = history.slice(-60).map((s) => s.cpu.usage);
    const freqText = sample?.cpu.freqMhz ? formatFrequencyGhz(sample.cpu.freqMhz) : "";
    const cpuTempText =
      settings.showTemps && sample?.cpu.tempC !== null && sample?.cpu.tempC !== undefined
        ? `${t("system.cpu")} ${formatTemperature(sample.cpu.tempC)}`
        : "";
    const gpuTempText =
      settings.showTemps && sample?.gpu?.tempC !== null && sample?.gpu?.tempC !== undefined
        ? `${t("system.gpu")} ${formatTemperature(sample.gpu.tempC)}`
        : "";
    const rightMeta = [freqText, cpuTempText, gpuTempText].filter(Boolean).join(" · ");

    return (
      <div
        className="c-mod is-exp c-mod-sys-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between g8">
          <span className="label">System</span>
          <span className="micro trunc">{rightMeta}</span>
        </div>
        <div className="row between" style={{ alignItems: "flex-end", marginTop: "calc(2 * var(--u))" }}>
          <span className="display-s" style={{ fontSize: "calc(24 * var(--u))" }}>
            {sample ? cpuPct : "—"}
            <span className="unit">%</span>
          </span>
          <span className="micro trunc">
            RAM {memGb} · {gpuSupported && gpuPct !== null ? `GPU ${gpuPct} %` : `Net ${netDown}`}
          </span>
        </div>
        <div className="c-spark" style={{ marginTop: "calc(4 * var(--u))" }}>
          <Sparkline
            values={pts.length >= 2 ? pts : [cpuPct, cpuPct]}
            width={200}
            height={36}
            theme={theme}
          />
        </div>
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-sys-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between g8">
        <div className="c-sys-stat">
          <span className="micro">
            CPU <b style={{ color: "var(--ink-1)" }}>{sample ? `${cpuPct}%` : "—"}</b>
          </span>
          <div className="c-mini-bar">
            <i className={barClass(cpuPct, barSmooth.smoothCpu)} style={{ transform: `scaleX(${Math.min(1, cpuPct / 100)})` }} />
          </div>
        </div>
        <div className="c-sys-stat">
          <span className="micro">
            RAM <b style={{ color: "var(--ink-1)" }}>{sample ? `${memPct}%` : "—"}</b>
          </span>
          <div className="c-mini-bar">
            <i className={barClass(memPct, barSmooth.smoothMem)} style={{ transform: `scaleX(${Math.min(1, memPct / 100)})` }} />
          </div>
        </div>
        <div className="c-sys-stat">
          <span className="micro">
            GPU{" "}
            <b style={{ color: "var(--ink-1)" }}>
              {gpuSupported && gpuPct !== null ? `${gpuPct}%` : t("state.dash")}
            </b>
          </span>
          <div className="c-mini-bar">
            <i
              className={gpuPct !== null ? barClass(gpuPct, barSmooth.smoothGpu) : ""}
              style={{ transform: `scaleX(${gpuPct !== null ? Math.min(1, gpuPct / 100) : 0})` }}
            />
          </div>
        </div>
        <HintBadge label={t("cluster.graph")} />
      </div>
    </div>
  );
});

const WeatherClusterMod = memo(function WeatherClusterMod({
  mod,
  mode,
  theme,
  location,
  snapshot,
  status,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  theme: ThemeId;
  location: Location | null;
  snapshot: WeatherSnapshot | null;
  status: string;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveWeatherSettings(mod.settings);
  const toUnit = (c: number) => (settings.unit === "f" ? (c * 9) / 5 + 32 : c);
  const fmtT = (c: number) => formatTemperature(toUnit(c));

  const city = location?.name || t("weather.pickPlace");
  const tz = location?.tz && location.tz !== "auto" ? location.tz : undefined;
  const current = snapshot?.current;
  const wmo = current ? resolveWmo(current.code, current.isDay) : null;

  if (mode === "expanded") {
    const hours = snapshot?.hourly.slice(0, 12) ?? [];
    const temps = hours.map((h) => toUnit(h.tempC));
    const labels = [0, 3, 6, 9, 11]
      .filter((idx) => hours[idx] !== undefined)
      .map((idx, i) => {
        if (i === 0) return t("weather.now");
        const item = hours[idx];
        return item ? formatTime(new Date(item.t), tz).slice(0, 2) : "";
      });

    return (
      <div
        className="c-mod is-exp c-mod-wx-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <div className="col" style={{ minWidth: 0 }}>
            <div className="row g4 title-s">
              <MapPin size="calc(11 * var(--u))" />
              <span className="trunc">{city}</span>
            </div>
            <div className="row g8" style={{ alignItems: "baseline", marginTop: "calc(2 * var(--u))" }}>
              <span className="display-m" style={{ fontSize: "calc(30 * var(--u))" }}>
                {current ? fmtT(current.tempC) : t("state.dash")}
              </span>
              <span className="meta trunc">
                {wmo ? wmo.label : status === "error" ? t("state.error") : t("weather.pickPlace")}
                {current ? ` · ${Math.round(current.windKmh)} km/t` : ""}
              </span>
            </div>
          </div>
          <WeatherIcon id={wmo?.icon ?? "wx-cloud-sun"} size={28} />
        </div>
        {temps.length >= 2 && (
          <>
            <div className="c-wave" style={{ marginTop: "calc(4 * var(--u))" }}>
              <WaveChart values={temps} width={200} height={34} theme={theme} />
            </div>
            <div className="row between" style={{ marginTop: "calc(2 * var(--u))" }}>
              {labels.map((lbl, idx) => (
                <span
                  key={`${lbl}-${idx}`}
                  className="micro"
                  style={idx === 0 ? { color: "var(--ink-1)" } : undefined}
                >
                  {lbl}
                </span>
              ))}
            </div>
          </>
        )}
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-wx-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between g6">
        <div className="row g6 grow">
          <WeatherIcon id={wmo?.icon ?? "wx-cloud-sun"} size={15} />
          <span className="value" style={{ fontSize: "calc(13 * var(--u))" }}>
            {current ? fmtT(current.tempC) : t("state.dash")}
          </span>
          <span className="meta trunc">
            {location ? `${city}${wmo ? ` · ${wmo.label}` : ""}` : t("weather.pickPlace")}
          </span>
        </div>
        <HintBadge label={t("cluster.forecast")} />
      </div>
    </div>
  );
});

const DiscordClusterMod = memo(function DiscordClusterMod({
  mod,
  mode,
  data,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  data: DiscordSnapshot | null;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const running = Boolean(data?.running);
  const inCall = Boolean(data?.inCall);
  const participants = data?.participants ?? [];
  const channelLabel =
    data?.channelName?.trim() ||
    (inCall && data?.mode === "rpc" ? t("discord.privateCall") : t("discord.inCall"));
  const label = !running ? t("discord.closed") : inCall ? channelLabel : t("discord.notInCall");

  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!inCall || preview) return;
    const id = window.setInterval(() => {
      if (!document.hidden) setNowMs(Date.now());
    }, 1_000);
    return () => window.clearInterval(id);
  }, [inCall, preview]);

  const callMs =
    inCall && data?.callStartedAt != null
      ? Math.max(0, nowMs - data.callStartedAt)
      : null;
  const timerStr = callMs !== null ? formatCallTimer(callMs) : null;

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-discord-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="col dc-exp-inner">
          <div className="row between g8" style={{ flex: "none", width: "100%" }}>
            <div className="row g6" style={{ minWidth: 0, flex: "1 1 0" }}>
              <span className={`dc-dot${inCall ? " is-call" : ""}`} />
              <span className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
                {label}
              </span>
              {inCall && participants.length > 0 && (
                <span className="micro tabular ink3" style={{ flex: "none" }}>
                  · {participants.length}
                </span>
              )}
            </div>
            {timerStr && (
              <span className="micro tabular ink2" style={{ flex: "none" }}>
                {timerStr}
              </span>
            )}
          </div>

          {inCall && participants.length > 0 ? (
            <>
              <div className="dc-list">
                {participants.map((p) => (
                  <DiscordParticipantRow key={p.id} participant={p} />
                ))}
              </div>
              {data?.canControlVoice && (
                <DiscordVoiceControls
                  selfMute={data.selfMute}
                  selfDeaf={data.selfDeaf}
                  preview={preview}
                />
              )}
            </>
          ) : inCall ? (
            <span className="micro ink3 dc-m-foot">{t("discord.fallbackHint")}</span>
          ) : (
            <span className="micro ink3 dc-m-foot">
              {!running
                ? t("discord.closedBody")
                : data?.mode === "rpc"
                  ? t("discord.running")
                  : t("discord.fallbackHint")}
            </span>
          )}
        </div>
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-discord-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row g8" style={{ width: "100%", minWidth: 0 }}>
        {inCall && participants.length > 0 ? (
          <DiscordAvatarStack participants={participants} max={3} sizeU={20} />
        ) : (
          <span className={`c-discord-ico${inCall ? " is-call" : ""}`}>
            <Headset size="calc(13 * var(--u))" weight={inCall ? "fill" : "regular"} />
          </span>
        )}
        <span className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))", flex: "1 1 0" }}>
          {label}
        </span>
        {timerStr && <span className="micro tabular ink2">{timerStr}</span>}
        {inCall && data?.selfDeaf ? (
          <SpeakerSlash size="calc(11 * var(--u))" className="dc-badge is-warn" weight="bold" />
        ) : inCall && data?.selfMute ? (
          <MicrophoneSlash size="calc(11 * var(--u))" className="dc-badge is-warn" weight="bold" />
        ) : null}
        <span className={`dc-dot${inCall ? " is-call" : ""}`} />
        <HintBadge label={t("cluster.more")} />
      </div>
    </div>
  );
});

const AgentsClusterMod = memo(function AgentsClusterMod({
  mod,
  mode,
  data,
  nowMs,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  data: AgentsSnapshot | null;
  nowMs: number;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const resolved = resolveAgentsSettings(mod.settings);
  const allSessions = data?.sessions ?? [];
  const visible = filterAgentSessions(allSessions, resolved.recentWindowMin, nowMs);

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-agents-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="col ag-exp-inner">
          <div className="row between g8" style={{ flex: "none" }}>
            <span className="meta ag-trunc" style={{ color: "var(--ink-2)" }}>
              {visible.length === 0 ? t("agents.empty.title") : formatAgentSummary(visible)}
            </span>
            {visible.length > 0 && <AgentStatusDot status={overallAgentStatus(visible)} />}
          </div>
          {visible.length === 0 ? (
            <span className="micro ink3 ag-empty-copy">{t("agents.empty.body")}</span>
          ) : (
            <div className="ag-list">
              {visible.map((session) => (
                <AgentRow key={session.id} session={session} nowMs={nowMs} preview={preview} />
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }

  if (visible.length === 0) {
    return (
      <div
        className="c-mod c-mod-agents-cmp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row g8">
          <span className="c-discord-ico">
            <Robot size="calc(13 * var(--u))" />
          </span>
          <span className="meta ag-trunc" style={{ color: "var(--ink-3)" }}>
            {t("agents.empty.title")}
          </span>
        </div>
      </div>
    );
  }

  if (visible.length === 1) {
    const only = visible[0]!;
    const tone = statusToneClass(only.status);
    return (
      <div
        className={`c-mod c-mod-agents-cmp${only.todo ? " has-todo" : ""}`}
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row g8">
          <AgentIcon agent={only.agent} variant="compact" />
          <div className="row g6 grow" style={{ minWidth: 0 }}>
            <span className="title-s ag-trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
              {only.agentLabel}
            </span>
            <span
              className={`micro ag-trunc ${only.status === "waiting" ? "ag-status-wait" : "ink3"}`}
            >
              {agentStatusLine(only, nowMs)}
            </span>
          </div>
          {only.todo && (
            <span className="micro tabular ag-todo-chip">
              {t("agents.todoProgress", {
                done: only.todo.done,
                total: only.todo.total,
              })}
            </span>
          )}
          <AgentStatusDot status={only.status} />
          <HintBadge label={t("cluster.more")} />
        </div>
        {only.todo && (
          <div className="c-ag-prog-edge" aria-hidden="true">
            <i
              className={tone || undefined}
              style={{ transform: `scaleX(${todoRatio(only.todo).toFixed(4)})` }}
            />
          </div>
        )}
      </div>
    );
  }

  const distinctAgents = Array.from(new Set(visible.map((s) => s.agent))).slice(0, 3);
  return (
    <div
      className="c-mod c-mod-agents-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row g8">
        <div className="ag-stack">
          {distinctAgents.map((agentId) => (
            <AgentIcon key={agentId} agent={agentId} variant="compact" />
          ))}
        </div>
        <span className="title-s ag-trunc grow" style={{ fontSize: "calc(12 * var(--u))" }}>
          {formatAgentSummary(visible)}
        </span>
        <div className="ag-dots">
          {visible.slice(0, 4).map((session) => (
            <AgentStatusDot key={session.id} status={session.status} />
          ))}
        </div>
        <HintBadge label={t("cluster.more")} />
      </div>
    </div>
  );
});

const PREVIEW_CLUSTER_NOTE = "- [x] Havregryn og kaffe\n- [ ] Mælk og æbler\n- [ ] Rugbrød";

// Behold note-tekst på tværs af collapse/expand-remounts så feltet ikke
// blinker tomt mens disken genindlæses.
const noteTextCache = new Map<string, string>();

const NoteClusterMod = memo(function NoteClusterMod({
  mod,
  mode,
  widgetId,
  editMode,
  preview,
  onModuleFocus,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  widgetId: string;
  editMode: boolean;
  preview?: boolean;
  onModuleFocus?: (moduleId: string, focused: boolean) => void;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveNoteSettings(mod.settings);
  const isPreviewSample = Boolean(preview && widgetId.startsWith("preview-"));
  const [text, setText] = useState(() =>
    isPreviewSample ? PREVIEW_CLUSTER_NOTE : (noteTextCache.get(mod.id) ?? ""),
  );
  const saveTimerRef = useRef<number | null>(null);
  const latestRef = useRef(
    isPreviewSample ? PREVIEW_CLUSTER_NOTE : (noteTextCache.get(mod.id) ?? ""),
  );

  useEffect(() => {
    if (isPreviewSample) {
      return;
    }
    let active = true;
    ipc
      .noteLoad(mod.id)
      .then((content) => {
        if (!active) return;
        noteTextCache.set(mod.id, content);
        setText(content);
        latestRef.current = content;
      })
      .catch(() => {});
    return () => {
      active = false;
      if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
    };
  }, [mod.id, isPreviewSample]);

  const scheduleSave = useCallback(
    (next: string) => {
      const clipped = next.slice(0, 4000);
      setText(clipped);
      latestRef.current = clipped;
      noteTextCache.set(mod.id, clipped);
      if (isPreviewSample) return;
      if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = window.setTimeout(() => {
        saveTimerRef.current = null;
        void ipc.noteSave(mod.id, clipped).catch(() => {});
      }, 500);
    },
    [mod.id, isPreviewSample],
  );

  const flushSave = useCallback(() => {
    if (saveTimerRef.current !== null) {
      window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    if (isPreviewSample) return;
    void ipc.noteSave(mod.id, latestRef.current.slice(0, 4000)).catch(() => {});
  }, [mod.id, isPreviewSample]);

  const rows = useMemo(() => parseChecklist(text), [text]);
  const nonEmptyRows = rows.filter((r) => r.text.trim().length > 0);
  const doneCount = nonEmptyRows.filter((r) => r.done).length;

  const updateRow = (idx: number, patch: Partial<ChecklistRow>) => {
    const next = [...rows];
    const current = next[idx];
    if (!current) return;
    next[idx] = { ...current, ...patch };
    scheduleSave(serializeChecklist(next));
  };

  const enterEditing = () => {
    if (editMode || preview) return;
    void ipc.widgetSetFocusable(widgetId, true).catch(() => {});
    onModuleFocus?.(mod.id, true);
  };

  const exitEditing = () => {
    flushSave();
    if (!preview) {
      void ipc.widgetSetFocusable(widgetId, false).catch(() => {});
    }
    onModuleFocus?.(mod.id, false);
  };

  useEffect(() => () => onModuleFocus?.(mod.id, false), [onModuleFocus, mod.id]);

  const handleKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      (e.currentTarget as HTMLElement).blur();
      exitEditing();
    }
  };

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-note-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <span className="label trunc">{settings.title}</span>
          <span className="micro tabular">
            {settings.mode === "checklist" ? `${doneCount}/${nonEmptyRows.length}` : `${text.length}/4000`}
          </span>
        </div>

        {settings.mode === "checklist" ? (
          <div className="c-note-list" data-no-expand>
            {rows.slice(0, 3).map((row, idx) => (
              <div className={`c-note-row${row.done ? " is-done" : ""}`} key={idx}>
                <button
                  type="button"
                  className="c-note-check"
                  onClick={(e) => {
                    e.stopPropagation();
                    updateRow(idx, { done: !row.done });
                  }}
                >
                  {row.done ? (
                    <CheckSquare size="calc(13 * var(--u))" weight="fill" />
                  ) : (
                    <Square size="calc(13 * var(--u))" />
                  )}
                </button>
                <input
                  type="text"
                  value={row.text}
                  readOnly={editMode}
                  placeholder={idx === 0 ? t("note.placeholder") : ""}
                  onFocus={enterEditing}
                  onBlur={exitEditing}
                  onKeyDown={handleKeyDown}
                  onChange={(e) => updateRow(idx, { text: e.target.value })}
                />
              </div>
            ))}
          </div>
        ) : (
          <textarea
            className="c-note-area"
            data-no-expand
            value={text}
            readOnly={editMode}
            maxLength={4000}
            placeholder={t("note.placeholder")}
            onFocus={enterEditing}
            onBlur={exitEditing}
            onKeyDown={handleKeyDown}
            onChange={(e) => scheduleSave(e.target.value)}
          />
        )}
      </div>
    );
  }

  const firstOpen = nonEmptyRows.find((r) => !r.done)?.text || "";
  const compactMeta =
    settings.mode === "checklist"
      ? nonEmptyRows.length > 0
        ? t("cluster.checked", { done: doneCount, total: nonEmptyRows.length })
        : t("note.placeholder")
      : firstOpen || t("note.placeholder");

  return (
    <div
      className="c-mod c-mod-note-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between g8">
        <span className="meta trunc" style={{ color: "var(--ink-2)" }}>
          Note · {settings.title}
        </span>
        <div className="row g6" style={{ flex: "0 1 auto", minWidth: 0 }}>
          <span className="micro trunc">{compactMeta}</span>
          <HintBadge label={t("cluster.more")} />
        </div>
      </div>
    </div>
  );
});

export function ClusterPillBarHeader({
  modules,
  expandedId,
  shared,
  onToggleChip,
}: {
  modules: readonly ClusterModule[];
  expandedId: string | null;
  shared: ClusterModuleSharedData;
  onToggleChip: (moduleId: string) => void;
}) {
  const chips = modules.filter(
    (m) => m.kind === "clock" || m.kind === "weather" || m.kind === "system",
  );
  const cpuPct = shared.system.data ? Math.round(shared.system.data.cpu.usage) : 0;

  return (
    <div className="c-pillbar">
      {chips.map((mod, idx) => {
        const active = expandedId === mod.id;
        return (
          <div className="row g4" key={mod.id} style={{ minWidth: 0 }}>
            {idx > 0 && <span className="c-pill-divider" />}
            <button
              type="button"
              className={`c-pill-chip${active ? " is-active" : ""}`}
              onClick={() => onToggleChip(mod.id)}
            >
              {mod.kind === "clock" && (
                <span className="value tabular">{formatTime(shared.now)}</span>
              )}
              {mod.kind === "weather" && (() => {
                const ws = resolveWeatherSettings(mod.settings);
                const locId = ws.locationId ?? shared.locations[0]?.id ?? "";
                const snap = locId ? shared.weatherByLoc.get(locId)?.data : null;
                const cur = snap?.current;
                const wmo = cur ? resolveWmo(cur.code, cur.isDay) : null;
                const tempVal = cur
                  ? formatTemperature(ws.unit === "f" ? (cur.tempC * 9) / 5 + 32 : cur.tempC)
                  : t("state.dash");
                return (
                  <>
                    <WeatherIcon id={wmo?.icon ?? "wx-cloud-sun"} size={13} />
                    <span className="value tabular">{tempVal}</span>
                  </>
                );
              })()}
              {mod.kind === "system" && (
                <span className="micro tabular">
                  CPU{" "}
                  <b style={{ color: "var(--ink-1)" }}>
                    {shared.system.data ? formatPercent(cpuPct / 100) : t("state.dash")}
                  </b>
                </span>
              )}
            </button>
          </div>
        );
      })}
    </div>
  );
}
