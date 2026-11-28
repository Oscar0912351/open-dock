import { useEffect, useState } from "react";
import type { SystemSample } from "../../../shared/bindings/SystemSample";
import { ipc } from "../../../shared/ipc";
import { reportFailure } from "../../../shared/logging";

const MAX_HISTORY = 60;

export function useSystemHistory(
  enabled = true,
  latestSample: SystemSample | null = null,
): SystemSample[] {
  const [history, setHistory] = useState<SystemSample[]>([]);
  const [lastAppendedT, setLastAppendedT] = useState<number | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let active = true;

    ipc
      .systemHistory()
      .then((rows) => {
        if (!active || rows.length === 0) return;
        setHistory((prev) => {
          const firstLive = prev[0];
          const older = firstLive ? rows.filter((r) => r.t < firstLive.t) : rows;
          if (older.length === 0) return prev;
          return [...older, ...prev].slice(-MAX_HISTORY);
        });
      })
      .catch((reason: unknown) => reportFailure("system_history", reason));

    return () => {
      active = false;
    };
  }, [enabled]);

  if (enabled && latestSample && latestSample.t !== lastAppendedT) {
    setLastAppendedT(latestSample.t);
    setHistory((prev) => {
      const last = prev[prev.length - 1];
      if (last && last.t >= latestSample.t) return prev;
      return [...prev, latestSample].slice(-MAX_HISTORY);
    });
  }

  return enabled ? history : [];
}
