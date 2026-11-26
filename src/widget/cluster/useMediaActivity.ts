import { useEffect, useRef, useState } from "react";
import type { ApplyUpdate } from "./useHysteresisSignal";

const PAUSE_GRACE_MS = 10_000;

interface MediaActivityOptions {
  /** Om klyngen overhovedet har et nowPlaying-modul. */
  enabled: boolean;
  /** Media-providerens afspilningsstatus. */
  status: string | undefined;
  apply: ApplyUpdate;
  /** Køres inde i samme batch som aktiveringen. */
  onEnter?: () => void;
}

/**
 * Afspiller-signalet passer ikke i en ren enter/exit-hysterese: en pause
 * aktiverer modulet med det samme (så brugeren når at se hvad der står stille),
 * men slukker det igen efter 10 s pause — også selvom inputtet aldrig forlader
 * "paused". Derfor sin egen maskine:
 *
 * - playing: aktiv med det samme, pause-udløber annulleres
 * - pause-start: aktiv med det samme + udløber om 10 s
 * - pause fortsat: intet nyt — udløberen fra pause-starten består
 * - stopped/none/slået fra: inaktiv med det samme
 *
 * Alle skift er asynkrone fordi apply typisk er en view-transition.
 */
export function useMediaActivitySignal({
  enabled,
  status,
  apply,
  onEnter,
}: MediaActivityOptions): boolean {
  const [active, setActive] = useState(false);

  const activeRef = useRef(active);
  const applyRef = useRef(apply);
  const onEnterRef = useRef(onEnter);
  const prevStatusRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    activeRef.current = active;
    applyRef.current = apply;
    onEnterRef.current = onEnter;
  });

  useEffect(() => {
    const current = enabled ? (status ?? "none") : "none";
    const enteredPause = current === "paused" && prevStatusRef.current !== "paused";
    prevStatusRef.current = current;

    if (current === "playing" || enteredPause) {
      const timers: number[] = [];
      if (!activeRef.current) {
        timers.push(
          window.setTimeout(() => {
            applyRef.current(() => {
              setActive(true);
              onEnterRef.current?.();
            });
          }, 0),
        );
      }
      if (enteredPause) {
        timers.push(
          window.setTimeout(() => {
            applyRef.current(() => setActive(false));
          }, PAUSE_GRACE_MS),
        );
      }
      return () => {
        for (const timer of timers) window.clearTimeout(timer);
      };
    }

    if (current !== "paused" && activeRef.current) {
      const timer = window.setTimeout(() => {
        applyRef.current(() => setActive(false));
      }, 0);
      return () => window.clearTimeout(timer);
    }
    // Fortsat pause: ingen ny effekt — udløberen fra pause-starten lever videre,
    // fordi active ikke er en dependency og effekten derfor ikke køres igen.
  }, [enabled, status]);

  return active;
}
