// Ren tidslogik for timer-widgetten — ingen React, kan unit-testes.

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

/** Kort varighed til titler og chips: "45 s", "10 min", "2 min 30 s", "1 t 30 min", "2 t". */
export function formatDurationShort(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds} s`;
  if (totalSeconds < 3600) {
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return seconds > 0 ? `${minutes} min ${seconds} s` : `${minutes} min`;
  }
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  return minutes > 0 ? `${hours} t ${minutes} min` : `${hours} t`;
}
