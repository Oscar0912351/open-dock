import {
  CaretDown,
  Headset,
  CheckSquare,
  MapPin,
  MusicNotesSimple,
  Pause,
  Play,
  Repeat,
  RepeatOnce,
  Shuffle,
  SkipBack,
  SkipForward,
  Square,
} from "@phosphor-icons/react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { DiscordSnapshot } from "../../shared/bindings/DiscordSnapshot";
import type { Location } from "../../shared/bindings/Location";
import type { MediaSnapshot } from "../../shared/bindings/MediaSnapshot";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { SystemCaps } from "../../shared/bindings/SystemCaps";
import type { SystemSample } from "../../shared/bindings/SystemSample";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { WeatherSnapshot } from "../../shared/bindings/WeatherSnapshot";
import {
  formatBytes,
  formatDateLong,
  formatDateShort,
  formatFrequencyGhz,
  formatPercent,
  formatSpeed,
  formatTemperature,
  formatTime,
  timeZoneCity, formatDuration,
} from "../../shared/format";
import { t } from "../../shared/i18n";
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
import { Equalizer } from "../kinds/nowPlaying/Equalizer";
import { resolveNowPlayingSettings } from "../kinds/nowPlaying/NowPlaying";
import { formatTrackTime, Progress } from "../kinds/nowPlaying/Progress";
import { useLiveMediaPosition } from "../kinds/nowPlaying/useLiveMediaPosition";
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

  const handleSurfaceClick = (event: MouseEvent<HTMLDivElement>) => {
    if (shouldIgnoreExpand(event)) return;
    onToggleExpand(mod.id);
  };

  switch (mod.kind) {
    case "clock":
      return (
        <ClockClusterMod
          mod={mod}
          mode={mode}
          now={shared.now}
          preview={shared.preview}
          onClick={handleSurfaceClick}
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
          onClick={handleSurfaceClick}
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
          onClick={handleSurfaceClick}
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
          onClick={handleSurfaceClick}
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
          onClick={handleSurfaceClick}
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
          onClick={handleSurfaceClick}
        />
      );
    default:
      return null;
  }
}

function ClockClusterMod({
  mod,
  mode,
  now,
  preview,
  onClick,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  now: Date;
  preview?: boolean;
  onClick: (e: MouseEvent<HTMLDivElement>) => void;
}) {
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
}

function MediaClusterMod({
  mod,
  mode,
  media,
  pausedApp,
  preview,
  onClick,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  media: MediaSnapshot | null;
  pausedApp: boolean;
  preview?: boolean;
  onClick: (e: MouseEvent<HTMLDivElement>) => void;
}) {
  const [mountNow] = useState(() => Date.now());
  const [scrubMs, setScrubMs] = useState<number | null>(null);
  const livePosMs = useLiveMediaPosition(media, pausedApp || Boolean(preview));

  const settings = resolveNowPlayingSettings(mod.settings);
  const playing = media?.status === "playing";
  const isAnimating = playing && !pausedApp && !preview;
  const coverUrl = media?.artUrl ? toAssetUrl(media.artUrl) : "";
  const sourceArg = settings.preferredSource === "auto" ? null : settings.preferredSource;
  const durMs = media ? Math.max(0, Number(media.durationMs)) : 0;
  const anchorPos = media ? Math.max(0, Number(media.positionMs)) : 0;
  const anchorUpdatedAt = media ? Number(media.updatedAt) : undefined;
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
  const edgeAnimKey = `${trackKey}:${anchorPos}:${anchorUpdatedAt ?? 0}:${durMs}:${isAnimating ? 1 : 0}`;

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
            key={edgeAnimKey}
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
}

function SystemClusterMod({
  mod,
  mode,
  theme,
  sample,
  history,
  caps,
  preview,
  onClick,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  theme: ThemeId;
  sample: SystemSample | null;
  history: SystemSample[];
  caps: SystemCaps | null;
  preview?: boolean;
  onClick: (e: MouseEvent<HTMLDivElement>) => void;
}) {
  const settings = resolveSystemSettings(mod.settings);
  const cpuPct = sample ? Math.round(sample.cpu.usage) : 0;
  const memPct = sample && Number(sample.mem.totalBytes) > 0
    ? Math.round((Number(sample.mem.usedBytes) / Number(sample.mem.totalBytes)) * 100)
    : 0;
  const memGb = sample ? formatBytes(Number(sample.mem.usedBytes)) : t("state.dash");
  const gpuSupported = caps?.gpu ?? Boolean(sample?.gpu);
  const gpuPct = sample?.gpu?.usage !== null && sample?.gpu?.usage !== undefined
    ? Math.round(sample.gpu.usage)
    : null;
  const netDown = sample
    ? formatSpeed(sample.net.rxBps, settings.unitsBits)
    : t("state.dash");

  const barToneClass = (pct: number) => (pct >= 95 ? "is-crit" : pct >= 85 ? "is-warn" : "");

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
            <i className={barToneClass(cpuPct)} style={{ transform: `scaleX(${Math.min(1, cpuPct / 100)})` }} />
          </div>
        </div>
        <div className="c-sys-stat">
          <span className="micro">
            RAM <b style={{ color: "var(--ink-1)" }}>{sample ? `${memPct}%` : "—"}</b>
          </span>
          <div className="c-mini-bar">
            <i className={barToneClass(memPct)} style={{ transform: `scaleX(${Math.min(1, memPct / 100)})` }} />
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
              className={gpuPct !== null ? barToneClass(gpuPct) : ""}
              style={{ transform: `scaleX(${gpuPct !== null ? Math.min(1, gpuPct / 100) : 0})` }}
            />
          </div>
        </div>
        <HintBadge label={t("cluster.graph")} />
      </div>
    </div>
  );
}

function WeatherClusterMod({
  mod,
  mode,
  theme,
  location,
  snapshot,
  status,
  preview,
  onClick,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  theme: ThemeId;
  location: Location | null;
  snapshot: WeatherSnapshot | null;
  status: string;
  preview?: boolean;
  onClick: (e: MouseEvent<HTMLDivElement>) => void;
}) {
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
        return item ? formatTime(new Date(Number(item.t)), tz).slice(0, 2) : "";
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
        <div className="row g6" style={{ minWidth: 0 }}>
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
}

function DiscordClusterMod({
  mod,
  mode,
  data,
  preview,
  onClick,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  data: DiscordSnapshot | null;
  preview?: boolean;
  onClick: (e: MouseEvent<HTMLDivElement>) => void;
}) {
  const running = Boolean(data?.running);
  const inCall = Boolean(data?.inCall);
  const label = !running
    ? t("discord.closed")
    : inCall
      ? t("discord.inCall")
      : t("discord.notInCall");
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!inCall || preview) return;
    const id = window.setInterval(() => setNowMs(Date.now()), 10_000);
    return () => window.clearInterval(id);
  }, [inCall, preview]);
  const callMs =
    inCall && data?.callStartedAt != null
      ? Math.max(0, nowMs - Number(data.callStartedAt))
      : null;
  const sub =
    callMs !== null ? formatDuration(callMs) : running ? t("discord.running") : t("discord.closedBody");

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-discord-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row g10">
          <div className={`dc-icon${inCall ? " is-call" : ""}`}>
            <Headset size="calc(16 * var(--u))" weight={inCall ? "fill" : "regular"} />
          </div>
          <div className="col grow" style={{ minWidth: 0 }}>
            <div className="row between">
              <span className="title-s trunc">{label}</span>
              <span className={`dc-dot${inCall ? " is-call" : ""}`} />
            </div>
            <div className="meta trunc">{sub}</div>
            <div className="micro ink3" style={{ marginTop: "calc(4 * var(--u))", lineHeight: 1.35 }}>
              {t("discord.detection")}
            </div>
          </div>
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
      <div className="row g8">
        <span className={`c-discord-ico${inCall ? " is-call" : ""}`}>
          <Headset size="calc(13 * var(--u))" weight={inCall ? "fill" : "regular"} />
        </span>
        <span className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
          {label}
        </span>
        <span className="grow" />
        {callMs !== null && <span className="micro tabular ink3">{formatDuration(callMs)}</span>}
        <span className={`dc-dot${inCall ? " is-call" : ""}`} />
      </div>
    </div>
  );
}

const PREVIEW_CLUSTER_NOTE = "- [x] Havregryn og kaffe\n- [ ] Mælk og æbler\n- [ ] Rugbrød";

// Behold note-tekst på tværs af collapse/expand-remounts så feltet ikke
// blinker tomt mens disken genindlæses.
const noteTextCache = new Map<string, string>();

function NoteClusterMod({
  mod,
  mode,
  widgetId,
  editMode,
  preview,
  onClick,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  widgetId: string;
  editMode: boolean;
  preview?: boolean;
  onClick: (e: MouseEvent<HTMLDivElement>) => void;
}) {
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
  };

  const exitEditing = () => {
    flushSave();
    if (!preview) {
      void ipc.widgetSetFocusable(widgetId, false).catch(() => {});
    }
  };

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
}

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
