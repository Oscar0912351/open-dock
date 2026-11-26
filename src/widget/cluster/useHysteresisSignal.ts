import { useEffect, useRef, useState } from "react";

export type ApplyUpdate = (update: () => void) => void;

interface HysteresisOptions {
  /** Rå input-tilstand, fx "mindst én agent er busy". */
  input: boolean;
  /** Forsinkelse før signalet tænder når input går højt (0 = næste macrotask). */
  enterMs?: number;
  /** Forsinkelse før signalet slukker når input går lavt (0 = næste macrotask). */
  exitMs?: number;
  /** Pakker state-skift ind (fx view-transition). Køres altid asynkront, så
      den må gerne bruge flushSync. */
  apply: ApplyUpdate;
  /** Køres inde i samme batch som aktiveringen. */
  onEnter?: () => void;
  /** Køres inde i samme batch som deaktiveringen. */
  onExit?: () => void;
}

/**
 * Hysterese over et boolsk input: tænd først efter enterMs højt input,
 * sluk først efter exitMs lavt input. Vender input tilbage til signalets
 * værdi mens et skift er afventende, annulleres det — det er effect-cleanup'en
 * der bærer den logik, ingen spejlede refs.
 *
 * Alle skift er asynkrone (også ved delay 0) fordi apply typisk er en
 * view-transition, der hverken må køre under render eller synkront i en effect.
 */
export function useHysteresisSignal({
  input,
  enterMs = 0,
  exitMs = 0,
  apply,
  onEnter,
  onExit,
}: HysteresisOptions): boolean {
  const [signal, setSignal] = useState(false);

  const applyRef = useRef(apply);
  const onEnterRef = useRef(onEnter);
  const onExitRef = useRef(onExit);
  useEffect(() => {
    applyRef.current = apply;
    onEnterRef.current = onEnter;
    onExitRef.current = onExit;
  });

  useEffect(() => {
    if (input === signal) return;
    const delay = input ? enterMs : exitMs;
    const timer = window.setTimeout(() => {
      applyRef.current(() => {
        setSignal(input);
        if (input) onEnterRef.current?.();
        else onExitRef.current?.();
      });
    }, delay);
    return () => window.clearTimeout(timer);
  }, [input, signal, enterMs, exitMs]);

  return signal;
}
