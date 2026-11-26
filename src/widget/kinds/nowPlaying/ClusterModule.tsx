import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { MusicNotesSimple, Pause, Play, Repeat, RepeatOnce, Shuffle, SkipBack, SkipForward } from "@phosphor-icons/react";
import { memo, useState, type CSSProperties, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import type { MediaSnapshot } from "../../../shared/bindings/MediaSnapshot";
import { t, useLocale } from "../../../shared/i18n";
import { ipc, toAssetUrl } from "../../../shared/ipc";
import { Equalizer } from "./Equalizer";
import { resolveNowPlayingSettings } from "./NowPlaying";
import { formatTrackTime, Progress } from "./Progress";
import { useLiveMediaPosition } from "./useLiveMediaPosition";
import { shouldIgnoreExpand, viewTransitionStyle } from "../../cluster/clusterModuleShared";

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
  const positionKnown = media?.positionKnown ?? false;
  const disabledSeekHint =
    !preview && media && durMs > 0 && !positionKnown
      ? t("media.positionUnknown", { source: sourceLabel })
      : !preview && media && !media.canSeek && durMs > 0
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
            {positionKnown && (durMs > 0 || displayPosMs > 0) ? formatTrackTime(displayPosMs) : t("state.dash")}
          </span>
          <div className="grow" data-no-expand>
            <Progress
              positionMs={displayPosMs}
              anchorPositionMs={anchorPos}
              anchorUpdatedAt={anchorUpdatedAt}
              durationMs={durMs}
              playing={isAnimating && positionKnown}
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
            className={isAnimating && positionKnown ? "is-playing" : undefined}
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

export function NowPlayingClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
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
}
