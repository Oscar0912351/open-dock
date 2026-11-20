import { useEffect, useState } from "react";
import { on } from "../ipc";
import { reportError } from "../logging";

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
      void reportError(`app://paused: ${String(reason)}`);
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  return { paused, setPaused };
}
