import { useEffect, useState } from "react";

export function useClock(showSeconds: boolean): Date {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    let timer: number;
    const schedule = () => {
      const interval = showSeconds ? 1000 : 60_000;
      const delay = interval - (Date.now() % interval) + 20;
      timer = window.setTimeout(() => {
        setNow(new Date());
        schedule();
      }, delay);
    };
    schedule();
    return () => window.clearTimeout(timer);
  }, [showSeconds]);

  return now;
}
