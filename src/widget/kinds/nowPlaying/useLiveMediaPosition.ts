import { useEffect, useState } from "react";
import type { MediaSnapshot } from "../../../shared/bindings/MediaSnapshot";

export function computeLivePositionMs(
  snapshot: MediaSnapshot | null,
  nowMs: number,
): number {
  if (!snapshot) return 0;
  const base = Math.max(0, Number(snapshot.positionMs));
  const dur = Math.max(0, Number(snapshot.durationMs));
  if (snapshot.status !== "playing") {
    return dur > 0 ? Math.min(dur, base) : base;
  }
  const elapsed = Math.max(0, nowMs - Number(snapshot.updatedAt));
  const pos = base + elapsed;
  return dur > 0 ? Math.min(dur, pos) : pos;
}

export function useLiveMediaPosition(
  snapshot: MediaSnapshot | null,
  paused = false,
): number {
  const [tickNow, setTickNow] = useState(() => Date.now());
  const isPlaying = snapshot?.status === "playing" && !paused;

  useEffect(() => {
    if (!isPlaying) return;
    const id = window.setInterval(() => {
      setTickNow(Date.now());
    }, 1000);
    return () => window.clearInterval(id);
  }, [isPlaying, snapshot?.updatedAt]);

  const updatedAtMs = Number(snapshot?.updatedAt ?? 0);
  return computeLivePositionMs(snapshot, Math.max(tickNow, updatedAtMs));
}
