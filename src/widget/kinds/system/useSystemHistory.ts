import { useEffect, useState } from "react";
import type { SystemSample } from "../../../shared/bindings/SystemSample";
import { ipc } from "../../../shared/ipc";

const MAX_HISTORY = 60;

export function useSystemHistory(
  enabled = true,
  latestSample: SystemSample | null = null,
): SystemSample[] {
  const [history, setHistory] = useState<SystemSample[]>([]);
  const [lastAppendedT, setLastAppendedT] = useState<bigint | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let active = true;

    ipc
      .systemHistory()
      .then((rows) => {
        if (active && rows.length > 0) {
          setHistory(rows.slice(-MAX_HISTORY));
        }
      })
      .catch(() => {});

    return () => {
      active = false;
    };
  }, [enabled]);

  if (enabled && latestSample && latestSample.t !== lastAppendedT) {
    setLastAppendedT(latestSample.t);
    setHistory((prev) => {
      const last = prev[prev.length - 1];
      if (last && last.t === latestSample.t) return prev;
      return [...prev, latestSample].slice(-MAX_HISTORY);
    });
  }

  return enabled ? history : [];
}
