import { useCallback, useEffect, useRef, useState } from "react";
import type { WidgetBoot } from "../../shared/bindings/WidgetBoot";
import { ipc } from "../../shared/ipc";

interface UseHoverRevealOptions {
  id: string;
  boot: WidgetBoot | null;
  hiddenIdle: boolean;
}

export function useHoverReveal({ id, boot, hiddenIdle }: UseHoverRevealOptions) {
  // Start som false indtil widgetReady returnerer Rusts målte starttilstand.
  // De første 4 s er widgetten alligevel synlig (bootGrace).
  const [hoverInside, setHoverInside] = useState(false);
  const [bootGrace, setBootGrace] = useState(true);
  const [pinnedVisible, setPinnedVisible] = useState(false);
  const hoverHideTimerRef = useRef<number | null>(null);

  const instance = boot?.instance;
  const kind = instance?.kind ?? "clock";
  const isEditing = boot?.editMode ?? false;

  const clearHoverHideTimer = useCallback(() => {
    if (hoverHideTimerRef.current !== null) {
      window.clearTimeout(hoverHideTimerRef.current);
      hoverHideTimerRef.current = null;
    }
  }, []);

  const setHoverInsideImmediate = useCallback(
    (inside: boolean) => {
      clearHoverHideTimer();
      setHoverInside(inside);
    },
    [clearHoverHideTimer],
  );

  const handleHoverEvent = useCallback(
    (inside: boolean) => {
      if (inside) {
        // Ind = vises med det samme — afbryd en ventende skjulning.
        clearHoverHideTimer();
        setHoverInside(true);
      } else {
        // Ud = 600 ms grace før widgetten skjules igen.
        clearHoverHideTimer();
        hoverHideTimerRef.current = window.setTimeout(() => {
          hoverHideTimerRef.current = null;
          setHoverInside(false);
        }, 600);
      }
    },
    [clearHoverHideTimer],
  );

  useEffect(() => {
    const timer = window.setTimeout(() => setBootGrace(false), 4_000);
    return () => window.clearTimeout(timer);
  }, []);

  useEffect(() => clearHoverHideTimer, [clearHoverHideTimer]);

  // "Vis kun når musen er over": widgetten er helt skjult + klik-igennem
  // indtil Rust melder markøren over vinduet. Pinnet synlig når der er en
  // finished-timer, inputtet er fokuseret, edit-mode kører, eller i de første
  // 4 s efter boot (bootGrace).
  const revealOnHover = instance?.revealOnHover ?? false;
  const peekHidden = Boolean(
    revealOnHover && !hoverInside && !pinnedVisible && !isEditing && !bootGrace,
  );
  const effectiveHiddenIdle =
    ((kind === "nowPlaying" || kind === "cluster") && hiddenIdle) || peekHidden;
  const idleHiddenActive = effectiveHiddenIdle && !isEditing;

  const prevIdleHiddenRef = useRef(false);
  useEffect(() => {
    if (!id || !boot) return;
    if (prevIdleHiddenRef.current === idleHiddenActive) return;
    prevIdleHiddenRef.current = idleHiddenActive;
    void ipc.widgetSetIdleHidden(id, idleHiddenActive).catch(() => {});
  }, [id, idleHiddenActive, boot]);

  return {
    hoverInside,
    bootGrace,
    pinnedVisible,
    setPinnedVisible,
    hoverHideTimerRef,
    revealOnHover,
    peekHidden,
    effectiveHiddenIdle,
    idleHiddenActive,
    setHoverInsideImmediate,
    handleHoverEvent,
  };
}
