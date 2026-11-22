import { useEffect, useState } from "react";

export function useClock(showSeconds: boolean, enabled = true): Date {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    if (!enabled) return;
    let timer: number;
    const schedule = () => {
      const interval = showSeconds ? 1000 : 60_000;
      const delay = interval - (Date.now() % interval) + 20;
      timer = window.setTimeout(() => {
        if (!document.hidden) {
          setNow(new Date());
        }
        schedule();
      }, delay);
    };
    const onVis = () => {
      if (!document.hidden) {
        setNow(new Date());
      }
    };
    schedule();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [showSeconds, enabled]);

  return now;
}
