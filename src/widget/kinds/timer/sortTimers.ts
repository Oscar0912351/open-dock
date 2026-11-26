import type { TimerItem } from "../../../shared/bindings/TimerItem";

/**
 * Sorteringsrækkefølge for timer-rækker: Finished først (ældst færdig
 * først), så Running (snarligst udløbende først), så Paused (mindst
 * resterende tid først). Ren funktion — input ændres ikke.
 */
export function sortTimers(items: readonly TimerItem[]): TimerItem[] {
  const rank = (item: TimerItem): number =>
    item.status === "finished" ? 0 : item.status === "running" ? 1 : 2;
  return [...items].sort((a, b) => {
    const byStatus = rank(a) - rank(b);
    if (byStatus !== 0) return byStatus;
    if (a.status === "finished" && b.status === "finished") {
      return (a.finishedAt ?? 0) - (b.finishedAt ?? 0);
    }
    if (a.status === "running" && b.status === "running") {
      return (a.endsAt ?? Number.MAX_SAFE_INTEGER) - (b.endsAt ?? Number.MAX_SAFE_INTEGER);
    }
    if (a.status === "paused" && b.status === "paused") {
      return a.remainingMs - b.remainingMs;
    }
    return 0;
  });
}
