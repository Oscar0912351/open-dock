import {
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
} from "react";
import { t } from "../../../shared/i18n";
import "./nowPlaying.css";

interface ProgressProps {
  positionMs: number;
  anchorPositionMs?: number;
  anchorUpdatedAt?: number;
  durationMs: number;
  playing: boolean;
  trackKey: string;
  height?: number;
  interactive?: boolean;
  disabledHint?: string;
  onSeek?: (ms: number) => void;
  onScrubPreview?: (ms: number | null) => void;
}

export function formatTrackTime(ms: number): string {
  const totalSec = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSec / 3600);
  const min = Math.floor((totalSec % 3600) / 60);
  const sec = totalSec % 60;
  if (hours > 0) {
    return `${hours}:${min.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}`;
  }
  return `${min}:${sec.toString().padStart(2, "0")}`;
}

function readPctFromClientX(el: HTMLElement, clientX: number): number {
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0) return 0;
  return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
}

export function Progress({
  positionMs,
  anchorPositionMs,
  anchorUpdatedAt,
  durationMs,
  playing,
  trackKey: _trackKey,
  height = 2.5,
  interactive = false,
  disabledHint,
  onSeek,
  onScrubPreview,
}: ProgressProps) {
  const [mountNow] = useState(() => Date.now());
  const [hoverPct, setHoverPct] = useState<number | null>(null);
  const [dragPct, setDragPct] = useState<number | null>(null);
  const activePointerIdRef = useRef<number | null>(null);

  const canSeek = interactive && durationMs > 0 && Boolean(onSeek);
  const safeDur = Math.max(1, durationMs);

  const basePos = Math.max(0, anchorPositionMs ?? positionMs);
  const baseAt = anchorUpdatedAt ?? mountNow;
  const startAt = Math.max(mountNow, baseAt);
  const rawStartPos = playing ? basePos + Math.max(0, startAt - baseAt) : basePos;
  const startPos = durationMs > 0 ? Math.min(safeDur, rawStartPos) : 0;

  const p0Ratio = durationMs > 0 ? Math.min(1, Math.max(0, startPos / safeDur)) : 0;
  const remaining = durationMs > 0 ? Math.max(1, safeDur - startPos) : 1;

  const displayMs =
    dragPct !== null
      ? Math.round(dragPct * durationMs)
      : Math.min(safeDur, Math.max(0, positionMs));
  const liveRatio =
    dragPct !== null
      ? dragPct
      : durationMs > 0
        ? Math.min(1, Math.max(0, positionMs / safeDur))
        : 0;

  const activeHeight =
    canSeek && (hoverPct !== null || dragPct !== null)
      ? Math.max(4, height + 1)
      : height;

  const barStyle: CSSProperties & Record<string, string | number> = {
    height: `calc(${activeHeight} * var(--u))`,
    "--p0": p0Ratio.toFixed(5),
    "--p-live": liveRatio.toFixed(5),
    "--rest": `${remaining}ms`,
  };

  const isAnimating = playing && dragPct === null;

  const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!canSeek || event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    activePointerIdRef.current = event.pointerId;
    try {
      event.currentTarget.setPointerCapture(event.pointerId);
    } catch {
      // Ignore synthetic environments without pointer capture
    }
    const pct = readPctFromClientX(event.currentTarget, event.clientX);
    setDragPct(pct);
    setHoverPct(pct);
    onScrubPreview?.(Math.round(pct * durationMs));
  };

  const handlePointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (dragPct !== null && activePointerIdRef.current === event.pointerId) {
      const pct = readPctFromClientX(event.currentTarget, event.clientX);
      setDragPct(pct);
      setHoverPct(pct);
      onScrubPreview?.(Math.round(pct * durationMs));
      return;
    }
    if (!canSeek && !disabledHint) return;
    setHoverPct(readPctFromClientX(event.currentTarget, event.clientX));
  };

  const finishDrag = (
    el: HTMLDivElement,
    pointerId: number,
    clientX: number | null,
  ) => {
    if (activePointerIdRef.current !== pointerId) return;
    activePointerIdRef.current = null;
    try {
      if (el.hasPointerCapture(pointerId)) {
        el.releasePointerCapture(pointerId);
      }
    } catch {
      // Ignore synthetic environments without pointer capture
    }
    if (clientX !== null && canSeek && onSeek) {
      const pct = readPctFromClientX(el, clientX);
      onSeek(Math.round(pct * durationMs));
    }
    setDragPct(null);
    onScrubPreview?.(null);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!canSeek || !onSeek) return;
    let nextMs: number | null = null;
    if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
      nextMs = Math.max(0, displayMs - 5000);
    } else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
      nextMs = Math.min(durationMs, displayMs + 5000);
    } else if (event.key === "Home") {
      nextMs = 0;
    } else if (event.key === "End") {
      nextMs = durationMs;
    }
    if (nextMs !== null) {
      event.preventDefault();
      event.stopPropagation();
      onSeek(nextMs);
    }
  };

  const barEl = (
    <div
      className={`prog${canSeek ? " prog-seekable" : ""}${isAnimating ? "" : " is-paused-prog"}`}
      style={barStyle}
    >
      {durationMs > 0 && (
        <i className={isAnimating ? "is-playing" : undefined} />
      )}
      {canSeek && durationMs > 0 && (
        <span
          className="prog-thumb"
          aria-hidden="true"
          style={{ left: `${liveRatio * 100}%` }}
        />
      )}
    </div>
  );

  if (!canSeek && !disabledHint) {
    return barEl;
  }

  const tooltipPct = dragPct ?? hoverPct;

  return (
    <div
      className={`prog-hit${!canSeek && disabledHint ? " is-unsupported" : ""}${dragPct !== null ? " is-dragging" : ""}`}
      data-no-expand
      role={canSeek ? "slider" : undefined}
      tabIndex={canSeek ? 0 : undefined}
      aria-label={canSeek ? t("media.seek") : undefined}
      aria-valuemin={canSeek ? 0 : undefined}
      aria-valuemax={canSeek ? durationMs : undefined}
      aria-valuenow={canSeek ? Math.round(displayMs) : undefined}
      aria-valuetext={canSeek ? formatTrackTime(displayMs) : undefined}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={(e) => finishDrag(e.currentTarget, e.pointerId, e.clientX)}
      onPointerCancel={(e) => finishDrag(e.currentTarget, e.pointerId, null)}
      onLostPointerCapture={(e) => finishDrag(e.currentTarget, e.pointerId, null)}
      onPointerLeave={() => {
        if (dragPct === null) setHoverPct(null);
      }}
      onClick={(e) => {
        if (canSeek || disabledHint) e.stopPropagation();
      }}
      onKeyDown={handleKeyDown}
    >
      {canSeek && tooltipPct !== null && durationMs > 0 && (
        <span
          className="prog-tooltip micro"
          style={{ left: `${Math.min(92, Math.max(8, tooltipPct * 100))}%` }}
        >
          {formatTrackTime(Math.round(tooltipPct * durationMs))}
        </span>
      )}
      {!canSeek && disabledHint && hoverPct !== null && (
        <span className="prog-tooltip is-hint" style={{ left: "50%" }}>
          {disabledHint}
        </span>
      )}
      {barEl}
    </div>
  );
}
