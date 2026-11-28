import { useEffect, useState } from "react";
import { on } from "../ipc";
import { reportFailure } from "../logging";

export function usePaused() {
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    on.paused((state) => {
      if (active) setPaused(state.paused);
    }).then((dispose) => {
      if (active) unlisten = dispose;
      else dispose();
    }).catch((reason: unknown) => {
      reportFailure("app://paused", reason);
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return { paused, setPaused };
}
