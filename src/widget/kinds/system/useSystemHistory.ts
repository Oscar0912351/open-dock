import { useEffect, useState } from "react";
import type { SystemSample } from "../../../shared/bindings/SystemSample";
import { ipc } from "../../../shared/ipc";

const MAX_HISTORY = 60;

export function useSystemHistory(
  enabled = true,
  latestSample: SystemSample | null = null,
): SystemSample[] {
  const [history, setHistory] = useState<SystemSample[]>([]);

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

  useEffect(() => {
    if (!enabled || !latestSample) return;
    setHistory((prev) => {
      const last = prev[prev.length - 1];
      if (last && last.t === latestSample.t) return prev;
      return [...prev, latestSample].slice(-MAX_HISTORY);
    });
  }, [enabled, latestSample]);

  return enabled ? history : [];
}
