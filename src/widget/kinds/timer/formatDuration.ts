// Ren tidslogik for timer-widgetten — ingen React, kan unit-testes.

/** Kortest mulige nedtælling — 5 s, så hurtige test-timere (og theæg) kan laves. */
export const MIN_DURATION_MS = 5_000;
export const MAX_DURATION_MS = 12 * 60 * 60 * 1000;

/** Resterende tid som "m:ss" eller "t:mm:ss" over en time. `ms` er allerede afrundet op. */
export function formatCountdown(ms: number): string {
  const totalSeconds = Math.max(0, Math.ceil(ms / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const ss = String(seconds).padStart(2, "0");
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${ss}`;
  }
  return `${minutes}:${ss}`;
}

/** Overtid efter finished: "+m:ss". */
export function formatOvertime(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `+${minutes}:${String(seconds).padStart(2, "0")}`;
}

/** Justér varighed med scroll: minutter-delen ±1 min (±10 med shift), sekunder ±5 s. */
export function adjustDuration(
  currentMs: number,
  part: "minutes" | "seconds",
  direction: 1 | -1,
  bigStep: boolean,
): number {
  const minuteStep = bigStep ? 10 * 60_000 : 60_000;
  const delta = part === "minutes" ? minuteStep : 5_000;
  const next = currentMs + direction * delta;
  return clampDuration(next);
}

export function clampDuration(ms: number): number {
  return Math.min(MAX_DURATION_MS, Math.max(MIN_DURATION_MS, ms));
}

/** Minut- og sekunddelene af en varighed til split-scroll-zoner. */
export function durationParts(ms: number): { minutes: number; seconds: number } {
  const totalSeconds = Math.round(ms / 1000);
  return {
    minutes: Math.floor(totalSeconds / 60),
    seconds: totalSeconds % 60,
  };
}
