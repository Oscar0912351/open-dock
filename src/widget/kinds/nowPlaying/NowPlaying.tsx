import {
  MusicNotesSimple,
  Pause,
  Play,
  Repeat,
  RepeatOnce,
  Shuffle,
  SkipBack,
  SkipForward,
} from "@phosphor-icons/react";
import { useEffect, useState, type CSSProperties } from "react";
import type { MediaSnapshot } from "../../../shared/bindings/MediaSnapshot";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { ipc, toAssetUrl } from "../../../shared/ipc";
import { Skeleton, StateEmpty } from "../../components/States";
import { Equalizer } from "./Equalizer";
import { formatTrackTime, Progress } from "./Progress";
import { useLiveMediaPosition } from "./useLiveMediaPosition";
import "./nowPlaying.css";

export interface NowPlayingSettings {
  preferredSource: string;
  ambient: boolean;
  showProgress: boolean;
  hideWhenIdle: boolean;
}

export const defaultNowPlayingSettings: NowPlayingSettings = {
  preferredSource: "auto",
  ambient: true,
  showProgress: true,
  hideWhenIdle: false,
};

export function resolveNowPlayingSettings(
  raw: Record<string, unknown> | undefined,
): NowPlayingSettings {
  if (!raw) return defaultNowPlayingSettings;
  return {
    preferredSource:
      typeof raw.preferredSource === "string" && raw.preferredSource.trim()
        ? raw.preferredSource
        : "auto",
    ambient: raw.ambient === undefined ? true : Boolean(raw.ambient),
    showProgress: raw.showProgress === undefined ? true : Boolean(raw.showProgress),
    hideWhenIdle: Boolean(raw.hideWhenIdle),
  };
}

interface NowPlayingProps {
  size: WidgetSize;
  theme: ThemeId;
  settings: NowPlayingSettings;
  pausedApp: boolean;
  onAmbientChange?: (rgb: [number, number, number] | null) => void;
  onHiddenIdleChange?: (hidden: boolean) => void;
  onAriaChange?: (aria: string) => void;
}

export function NowPlaying({
  size,
  theme,
  settings,
  pausedApp,
  onAmbientChange,
  onHiddenIdleChange,
  onAriaChange,
}: NowPlayingProps) {
  const { status, data: snapshot, showSkeleton } = useProvider(
    "media",
    settings.preferredSource === "auto" ? null : settings.preferredSource,
  );
  const [scrubMs, setScrubMs] = useState<number | null>(null);
  const livePosMs = useLiveMediaPosition(snapshot, pausedApp);

  const isPlaying = snapshot?.status === "playing" && !pausedApp;
  const isIdle = !snapshot || snapshot.status === "none" || snapshot.status === "stopped";

  useEffect(() => {
    if (!onAmbientChange) return;
    if (
      settings.ambient &&
      snapshot &&
      snapshot.status !== "none" &&
      snapshot.artUrl &&
      snapshot.artColor
    ) {
      onAmbientChange(snapshot.artColor);
    } else {
      onAmbientChange(null);
    }
  }, [settings.ambient, snapshot, onAmbientChange]);

  useEffect(() => {
    if (!onHiddenIdleChange) return;
    if (!settings.hideWhenIdle || !isIdle) {
      onHiddenIdleChange(false);
      return;
    }
    const timer = window.setTimeout(() => {
      onHiddenIdleChange(true);
    }, 10_000);
    return () => window.clearTimeout(timer);
  }, [settings.hideWhenIdle, isIdle, onHiddenIdleChange]);

  const anchorPosMs = snapshot ? Math.max(0, snapshot.positionMs) : 0;
  const anchorUpdatedAt = snapshot ? snapshot.updatedAt : undefined;
  const posMs = scrubMs ?? livePosMs;
  const durMs = snapshot?.durationMs ?? 0;

  useEffect(() => {
    if (!onAriaChange) return;
    if (!snapshot || snapshot.status === "none") {
      onAriaChange(t("media.empty.title"));
      return;
    }
    const artistPart = snapshot.artist ? ` af ${snapshot.artist}` : "";
    const statusWord = snapshot.status === "playing" ? "afspiller" : "pauset";
    onAriaChange(`Afspiller: ${snapshot.title}${artistPart}, ${statusWord}`);
  }, [snapshot, onAriaChange]);

  if (!snapshot) {
    if (!showSkeleton) return null;
    return <MediaSkeleton size={size} />;
  }

  if (status === "empty" || snapshot.status === "none") {
    return (
      <StateEmpty
        icon={<MusicNotesSimple size="calc(18 * var(--u))" />}
        title={t("media.empty.title")}
        subtitle={size === "pill" ? "Spotify, browser m.fl." : t("media.empty.body")}
        compact={size === "pill"}
      />
    );
  }

  const artSrc = snapshot.artUrl ? toAssetUrl(snapshot.artUrl) : "";
  const trackKey = `${snapshot.title}|${snapshot.artist}|${snapshot.album}`;
  const isStopped = snapshot.status === "stopped";
  const prefArg = settings.preferredSource === "auto" ? null : settings.preferredSource;
  const playLabel = isPlaying ? t("media.pause") : t("media.play");
  const restMs = Math.max(0, durMs - posMs);

  const sourceLabel = snapshot.source || "Afspiller";
  const disabledSeekHint =
    !snapshot.canSeek && durMs > 0
      ? t("media.seekUnsupported", { source: sourceLabel })
      : undefined;

  const sendControl = (action: Parameters<typeof ipc.mediaControl>[0]) => {
    void ipc.mediaControl(action, prefArg);
  };
  const pickSource = (source: string) => {
    void ipc.mediaSelectSource(source);
  };

  if (size === "pill") {
    return (
      <div className={isStopped ? "np-stopped" : undefined} style={{ height: "100%", padding: "0 calc(0 * var(--u))" }}>
        <div className="row g8 np-pill" style={{ margin: "0 calc(-4 * var(--u))" }}>
          <div
            className="np-cover"
            style={{ "--cs": 32, "--cr": theme === "laerk" ? 6 : 8 } as CSSProperties}
            data-enter="pop"
          >
            {artSrc ? (
              <img key={artSrc} src={artSrc} alt="" />
            ) : (
              <MusicNotesSimple size="calc(15 * var(--u))" className="ink3" />
            )}
          </div>
          <div key={trackKey} className="col grow" style={{ gap: "calc(1 * var(--u))", minWidth: 0 }} data-enter>
            <div className="title-s trunc">{snapshot.title || t("media.empty.title")}</div>
            {snapshot.artist && <div className="meta trunc">{snapshot.artist}</div>}
          </div>
          <div className="np-pill-action" data-enter="pop">
            {isPlaying ? (
              <>
                <div className="np-pill-eq is-active">
                  <Equalizer playing={!pausedApp} height={11} />
                </div>
                <button
                  type="button"
                  className="od-btn np-pill-pause is-active"
                  style={{ "--b": 28 } as CSSProperties}
                  aria-label={t("media.pause")}
                  onClick={() => sendControl({ type: "playPause" })}
                >
                  <Pause size="calc(13 * var(--u))" weight="fill" />
                </button>
              </>
            ) : (
              <button
                type="button"
                className="od-btn"
                style={{ "--b": 28 } as CSSProperties}
                aria-label={t("media.play")}
                onClick={() => sendControl({ type: "playPause" })}
              >
                <Play size="calc(13 * var(--u))" weight="fill" />
              </button>
            )}
          </div>
        </div>
        {settings.showProgress && durMs > 0 && (
          <div className="np-pill-prog">
            <Progress
              positionMs={posMs}
              anchorPositionMs={anchorPosMs}
              anchorUpdatedAt={anchorUpdatedAt}
              durationMs={durMs}
              playing={isPlaying}
              trackKey={trackKey}
              height={2}
            />
          </div>
        )}
      </div>
    );
  }

  if (size === "s") {
    return (
      <div className={`col grow${isStopped ? " np-stopped" : ""}`} style={{ height: "100%" }}>
        <div className="row between np-s-top">
          <div
            className="np-cover"
            style={{ "--cs": 50, "--cr": theme === "laerk" ? 10 : 12 } as CSSProperties}
            data-enter="pop"
          >
            {artSrc ? (
              <img key={artSrc} src={artSrc} alt="" />
            ) : (
              <MusicNotesSimple size="calc(20 * var(--u))" className="ink3" />
            )}
          </div>
          <div className="col between np-s-side">
            {snapshot.status === "playing" || snapshot.status === "paused" ? (
              <Equalizer playing={isPlaying} height={10} />
            ) : (
              <span />
            )}
            <button
              type="button"
              className="od-btn primary"
              style={{ "--b": 30 } as CSSProperties}
              aria-label={playLabel}
              onClick={() => sendControl({ type: "playPause" })}
            >
              {isPlaying ? (
                <Pause size="calc(13 * var(--u))" weight="fill" />
              ) : (
                <Play size="calc(13 * var(--u))" weight="fill" />
              )}
            </button>
          </div>
        </div>
        <div key={trackKey} className="col np-s-meta" data-enter>
          <div className="title-s trunc">{snapshot.title || t("media.empty.title")}</div>
          {snapshot.artist && <div className="meta trunc">{snapshot.artist}</div>}
        </div>
        <div className="grow" />
        {settings.showProgress && durMs > 0 && (
          <Progress
            positionMs={posMs}
            anchorPositionMs={anchorPosMs}
            anchorUpdatedAt={anchorUpdatedAt}
            durationMs={durMs}
            playing={isPlaying}
            trackKey={trackKey}
            height={2.5}
          />
        )}
      </div>
    );
  }

  if (size === "m") {
    return (
      <div className={`row g12 np-m-root${isStopped ? " np-stopped" : ""}`}>
        <div
          className="np-cover"
          style={{ "--cs": 104, "--cr": theme === "laerk" ? 12 : 16 } as CSSProperties}
          data-enter="pop"
        >
          {artSrc ? (
            <img key={artSrc} src={artSrc} alt="" />
          ) : (
            <MusicNotesSimple size="calc(28 * var(--u))" className="ink3" />
          )}
        </div>
        <div className="col grow np-m-info">
          <div className="row between">
            {snapshot.sessions.length > 1 ? (
              <SourceChips snapshot={snapshot} onPick={pickSource} />
            ) : (
              <span className="od-chip" data-enter>
                {sourceLabel}
              </span>
            )}
            {(snapshot.status === "playing" || snapshot.status === "paused") && (
              <Equalizer playing={isPlaying} height={11} />
            )}
          </div>
          <div key={trackKey} className="col np-m-text" data-enter>
            <div className="title trunc">{snapshot.title || t("media.empty.title")}</div>
            {snapshot.artist && <div className="body trunc">{snapshot.artist}</div>}
          </div>
          <div className="grow" />
          {settings.showProgress && durMs > 0 && (
            <div className="row g6" data-enter>
              <span className="micro">{formatTrackTime(posMs)}</span>
              <div className="grow">
                <Progress
                  positionMs={posMs}
                  anchorPositionMs={anchorPosMs}
                  anchorUpdatedAt={anchorUpdatedAt}
                  durationMs={durMs}
                  playing={isPlaying}
                  trackKey={trackKey}
                  height={2.5}
                  interactive={snapshot.canSeek}
                  disabledHint={disabledSeekHint}
                  onScrubPreview={setScrubMs}
                  onSeek={(ms) => sendControl({ type: "seek", ms })}
                />
              </div>
              <span className="micro">−{formatTrackTime(restMs)}</span>
            </div>
          )}
          <div className="row between np-m-controls">
            <div className="row g4">
              {snapshot.canPrev && (
                <button
                  type="button"
                  className="od-btn"
                  style={{ "--b": 24 } as CSSProperties}
                  aria-label={t("media.prev")}
                  onClick={() => sendControl({ type: "previous" })}
                >
                  <SkipBack size="calc(12 * var(--u))" weight="fill" />
                </button>
              )}
              <button
                type="button"
                className="od-btn primary"
                style={{ "--b": 32 } as CSSProperties}
                aria-label={playLabel}
                onClick={() => sendControl({ type: "playPause" })}
              >
                {isPlaying ? (
                  <Pause size="calc(14 * var(--u))" weight="fill" />
                ) : (
                  <Play size="calc(14 * var(--u))" weight="fill" />
                )}
              </button>
              {snapshot.canNext && (
                <button
                  type="button"
                  className="od-btn"
                  style={{ "--b": 24 } as CSSProperties}
                  aria-label={t("media.next")}
                  onClick={() => sendControl({ type: "next" })}
                >
                  <SkipForward size="calc(12 * var(--u))" weight="fill" />
                </button>
              )}
            </div>
            {(snapshot.shuffle !== null || snapshot.repeat !== null) && (
              <div className="row g2">
                {snapshot.shuffle !== null && (
                  <button
                    type="button"
                    className={`od-btn ${snapshot.shuffle ? "np-btn-active" : "np-btn-muted"}`}
                    style={{ "--b": 22 } as CSSProperties}
                    aria-label={t("media.shuffle")}
                    aria-pressed={snapshot.shuffle}
                    onClick={() => sendControl({ type: "shuffle" })}
                  >
                    <Shuffle size="calc(12 * var(--u))" />
                  </button>
                )}
                {snapshot.repeat !== null && (
                  <button
                    type="button"
                    className={`od-btn ${snapshot.repeat !== "none" ? "np-btn-active" : "np-btn-muted"}`}
                    style={{ "--b": 22 } as CSSProperties}
                    aria-label={t("media.repeat")}
                    aria-pressed={snapshot.repeat !== "none"}
                    onClick={() => sendControl({ type: "repeat" })}
                  >
                    {snapshot.repeat === "track" ? (
                      <RepeatOnce size="calc(12 * var(--u))" />
                    ) : (
                      <Repeat size="calc(12 * var(--u))" />
                    )}
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    );
  }

  // Size: "l" (276x276)
  const subline = [snapshot.artist, snapshot.album].filter(Boolean).join(" · ");
  return (
    <div className={`col grow${isStopped ? " np-stopped" : ""}`} style={{ height: "100%" }}>
      <div className="np-cover np-l-cover" data-enter="pop">
        {artSrc ? (
          <img key={artSrc} src={artSrc} alt="" />
        ) : (
          <MusicNotesSimple size="calc(36 * var(--u))" className="ink3" />
        )}
        <span className="od-chip np-l-chip">
          {snapshot.source || "Afspiller"}
        </span>
      </div>

      <div key={trackKey} className="row between np-l-head" data-enter>
        <div className="col g2 grow" style={{ minWidth: 0 }}>
          <div className="np-l-title trunc">{snapshot.title || t("media.empty.title")}</div>
          {subline && <div className="body trunc">{subline}</div>}
        </div>
        {(snapshot.status === "playing" || snapshot.status === "paused") && (
          <Equalizer playing={isPlaying} height={12} />
        )}
      </div>

      {snapshot.sessions.length > 1 && (
        <SourceChips snapshot={snapshot} onPick={pickSource} />
      )}

      <div className="grow" />

      {settings.showProgress && durMs > 0 && (
        <div data-enter>
          <Progress
            positionMs={posMs}
            anchorPositionMs={anchorPosMs}
            anchorUpdatedAt={anchorUpdatedAt}
            durationMs={durMs}
            playing={isPlaying}
            trackKey={trackKey}
            height={3.5}
            interactive={snapshot.canSeek}
            disabledHint={disabledSeekHint}
            onScrubPreview={setScrubMs}
            onSeek={(ms) => sendControl({ type: "seek", ms })}
          />
          <div className="row between np-l-times">
            <span className="micro">{formatTrackTime(posMs)}</span>
            <span className="micro">−{formatTrackTime(restMs)}</span>
          </div>
        </div>
      )}

      <div className="row np-l-controls">
        {snapshot.shuffle !== null && (
          <button
            type="button"
            className={`od-btn ${snapshot.shuffle ? "np-btn-active" : "np-btn-muted"}`}
            style={{ "--b": 28 } as CSSProperties}
            aria-label={t("media.shuffle")}
            aria-pressed={snapshot.shuffle}
            onClick={() => sendControl({ type: "shuffle" })}
          >
            <Shuffle size="calc(13 * var(--u))" />
          </button>
        )}
        {snapshot.canPrev && (
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 32 } as CSSProperties}
            aria-label={t("media.prev")}
            onClick={() => sendControl({ type: "previous" })}
          >
            <SkipBack size="calc(15 * var(--u))" weight="fill" />
          </button>
        )}
        <button
          type="button"
          className="od-btn primary"
          style={{ "--b": 38 } as CSSProperties}
          aria-label={playLabel}
          onClick={() => sendControl({ type: "playPause" })}
        >
          {isPlaying ? (
            <Pause size="calc(17 * var(--u))" weight="fill" />
          ) : (
            <Play size="calc(17 * var(--u))" weight="fill" />
          )}
        </button>
        {snapshot.canNext && (
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 32 } as CSSProperties}
            aria-label={t("media.next")}
            onClick={() => sendControl({ type: "next" })}
          >
            <SkipForward size="calc(15 * var(--u))" weight="fill" />
          </button>
        )}
        {snapshot.repeat !== null && (
          <button
            type="button"
            className={`od-btn ${snapshot.repeat !== "none" ? "np-btn-active" : "np-btn-muted"}`}
            style={{ "--b": 28 } as CSSProperties}
            aria-label={t("media.repeat")}
            aria-pressed={snapshot.repeat !== "none"}
            onClick={() => sendControl({ type: "repeat" })}
          >
            {snapshot.repeat === "track" ? (
              <RepeatOnce size="calc(13 * var(--u))" />
            ) : (
              <Repeat size="calc(13 * var(--u))" />
            )}
          </button>
        )}
      </div>
    </div>
  );
}

function MediaSkeleton({ size }: { size: WidgetSize }) {
  if (size === "pill") {
    return (
      <div className="row g8" style={{ height: "100%" }}>
        <Skeleton width={32} height={32} radius={8} />
        <div className="col grow g4">
          <Skeleton width="70%" height={11} />
          <Skeleton width="45%" height={8} />
        </div>
        <Skeleton width={24} height={24} circle />
      </div>
    );
  }
  if (size === "s") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <div className="row between">
          <Skeleton width={50} height={50} radius={12} />
          <Skeleton width={30} height={30} circle />
        </div>
        <div className="col g4">
          <Skeleton width="80%" height={11} />
          <Skeleton width="55%" height={9} />
        </div>
        <Skeleton width="100%" height={3} radius={99} />
      </div>
    );
  }
  if (size === "l") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <Skeleton width="100%" height={132} radius={14} />
        <div className="col g6">
          <Skeleton width="65%" height={16} />
          <Skeleton width="48%" height={11} />
        </div>
        <Skeleton width="100%" height={4} radius={99} />
        <div className="row" style={{ justifyContent: "center", gap: "calc(8 * var(--u))" }}>
          <Skeleton width={32} height={32} circle />
          <Skeleton width={38} height={38} circle />
          <Skeleton width={32} height={32} circle />
        </div>
      </div>
    );
  }
  return (
    <div className="row g12" style={{ height: "100%" }}>
      <Skeleton width={104} height={104} radius={16} />
      <div className="col grow g6">
        <Skeleton width="46%" height={16} radius={999} />
        <Skeleton width="80%" height={12} />
        <Skeleton width="55%" height={10} />
        <div className="grow" />
        <Skeleton width="100%" height={3} radius={999} />
        <div className="row g6">
          <Skeleton width={24} height={24} circle />
          <Skeleton width={30} height={30} circle />
          <Skeleton width={24} height={24} circle />
        </div>
      </div>
    </div>
  );
}

function SourceChips({
  snapshot,
  onPick,
}: {
  snapshot: MediaSnapshot;
  onPick: (source: string) => void;
}) {
  return (
    <div className="np-sources" role="group" aria-label={t("media.sources")} data-enter>
      {snapshot.sessions.map((session) => (
        <button
          key={session.source}
          type="button"
          className={`od-chip np-source-chip${session.source === snapshot.source ? " is-active" : ""}`}
          aria-pressed={session.source === snapshot.source}
          title={
            session.title
              ? `${session.title}${session.artist ? ` – ${session.artist}` : ""}`
              : session.source
          }
          onClick={(event) => {
            event.stopPropagation();
            onPick(session.source);
          }}
        >
          {session.source}
        </button>
      ))}
    </div>
  );
}
