import { useEffect, useState } from "react";
import type { SystemSample } from "../../../shared/bindings/SystemSample";
import { ipc, on } from "../../../shared/ipc";

const MAX_HISTORY = 60;

export function useSystemHistory(enabled = true): SystemSample[] {
  const [history, setHistory] = useState<SystemSample[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let dispose: (() => void) | undefined;

    ipc
      .systemHistory()
      .then((rows) => {
        if (active && rows.length > 0) {
          setHistory(rows.slice(-MAX_HISTORY));
        }
      })
      .catch(() => {});

    on.system((sample) => {
      if (!active) return;
      setHistory((prev) => {
        const last = prev[prev.length - 1];
        if (last && last.t === sample.t) return prev;
        return [...prev, sample].slice(-MAX_HISTORY);
      });
    })
      .then((unlisten) => {
        if (active) dispose = unlisten;
        else unlisten();
      })
      .catch(() => {});

    return () => {
      active = false;
      dispose?.();
    };
  }, [enabled]);

  return enabled ? history : [];
}
