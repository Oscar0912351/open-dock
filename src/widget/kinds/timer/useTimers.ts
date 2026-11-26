import { useEffect, useRef, useState } from "react";
import type { TimerListSnapshot } from "../../../shared/bindings/TimerListSnapshot";
import { ipc, on } from "../../../shared/ipc";

export interface TimerListData {
  snapshot: TimerListSnapshot | null;
  /** Opdateres cirka hvert sekund mens en timer kører eller er færdig — bruges til lokal nedtælling. */
  nowMs: number;
}

/**
 * Data-hook til timer-widgetten (og klynge-timer-modulet). Rust ejer
 * sandheden: vi henter ét snapshot ved mount og lytter på `timer://state`.
 * Sekunderne tælles lokalt ud fra `endsAt`/`finishedAt`, så der ikke går
 * tick-trafik over IPC. `id` er widget-id eller klynge-modul-id;
 * `enabled=false` (preview) springer al IPC over.
 */
export function useTimers(id: string | null, enabled = true): TimerListData {
  const [snapshot, setSnapshot] = useState<TimerListSnapshot | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (!id || !enabled) return;
    let active = true;
    let dispose: (() => void) | null = null;

    ipc
      .timerGet(id)
      .then((initial) => {
        if (active) setSnapshot(initial);
      })
      .catch(() => {});

    on.timerState((next) => {
      if (!active || next.widgetId !== id) return;
      setSnapshot(next);
      // Synkronisér klokken med det samme ved tilstandsskift, så fx pause
      // eller et nyligt ring ikke viser et forældet sekund.
      setNowMs(Date.now());
    })
      .then((un) => {
        if (active) dispose = un;
        else un();
      })
      .catch(() => {});

    return () => {
      active = false;
      dispose?.();
    };
  }, [id, enabled]);

  const ticking =
    snapshot !== null &&
    snapshot.items.some((item) => item.status === "running" || item.status === "finished");

  const tickingRef = useRef(ticking);
  tickingRef.current = ticking;

  useEffect(() => {
    if (!ticking) return;
    let timer = 0;
    const schedule = () => {
      // Sov til næste hele sekund (+ lille margin) — samme mønster som useClock.
      const delay = 1000 - (Date.now() % 1000) + 30;
      timer = window.setTimeout(() => {
        if (!document.hidden) {
          setNowMs(Date.now());
        }
        schedule();
      }, delay);
    };
    const onVis = () => {
      if (!document.hidden) setNowMs(Date.now());
    };
    schedule();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [ticking]);

  return { snapshot, nowMs };
}
