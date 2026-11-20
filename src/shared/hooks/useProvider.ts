import { useEffect, useState } from "react";
import type { DiscordSnapshot } from "../bindings/DiscordSnapshot";
import type { MediaSnapshot } from "../bindings/MediaSnapshot";
import type { ProviderKind } from "../bindings/ProviderKind";
import type { SystemSample } from "../bindings/SystemSample";
import type { WeatherSnapshot } from "../bindings/WeatherSnapshot";
import { ipc, on } from "../ipc";
import { reportError } from "../logging";

export type ProviderStatus = "loading" | "ready" | "empty" | "error" | "stale";

export type ProviderDataMap = {
  media: MediaSnapshot;
  system: SystemSample;
  weather: WeatherSnapshot;
  discord: DiscordSnapshot;
};

export interface ProviderResult<T> {
  status: ProviderStatus;
  data: T | null;
  showSkeleton: boolean;
}

const LOADING_REVEAL_MS = 300;

function deriveStatus<K extends ProviderKind>(
  kind: K,
  data: ProviderDataMap[K] | null,
  enabled: boolean,
  failed: boolean,
): ProviderStatus {
  if (!enabled) return "empty";
  if (failed && !data) return "error";
  if (!data) return "loading";

  if (kind === "media") {
    const snap = data as MediaSnapshot;
    return snap.status === "none" ? "empty" : "ready";
  }

  if (kind === "weather") {
    const snap = data as WeatherSnapshot;
    if (!snap.current) {
      return snap.error ? "error" : "empty";
    }
    return snap.stale ? "stale" : "ready";
  }

  return "ready";
}

export function useProvider<K extends ProviderKind>(
  kind: K,
  key?: string | null,
  activeSub = true,
): ProviderResult<ProviderDataMap[K]> {
  const normalizedKey = key?.trim() ? key.trim() : null;
  const enabled = activeSub && (kind !== "weather" || normalizedKey !== null);
  const subId = enabled ? `${kind}:${normalizedKey ?? ""}` : null;

  const [state, setState] = useState<{
    subId: string | null;
    data: ProviderDataMap[K] | null;
    failed: boolean;
    showSkeleton: boolean;
  }>({ subId: null, data: null, failed: false, showSkeleton: false });

  useEffect(() => {
    if (!subId) {
      return;
    }

    let active = true;
    let unlisten: (() => void) | undefined;
    let subscribed = false;
    let receivedEvent = false;
    const skeletonTimer = window.setTimeout(() => {
      if (active) {
        setState((prev) =>
          prev.subId === subId
            ? { ...prev, showSkeleton: true }
            : { subId, data: null, failed: false, showSkeleton: true },
        );
      }
    }, LOADING_REVEAL_MS);

    const applyData = (payload: ProviderDataMap[K], failed = false) => {
      if (!active) return;
      receivedEvent = true;
      setState({ subId, data: payload, failed, showSkeleton: false });
    };

    const subscribeEvent = () => {
      if (kind === "media") {
        return on.media((payload) => {
          applyData(payload as ProviderDataMap[K]);
        });
      }
      if (kind === "system") {
        return on.system((payload) => {
          applyData(payload as ProviderDataMap[K]);
        });
      }
      if (kind === "weather") {
        return on.weather((payload) => {
          if (normalizedKey && payload.locationId !== normalizedKey) return;
          applyData(
            payload as ProviderDataMap[K],
            Boolean(payload.error && !payload.current),
          );
        });
      }
      return on.discord((payload) => {
        applyData(payload as ProviderDataMap[K]);
      });
    };

    void (async () => {
      try {
        const dispose = await subscribeEvent();
        if (!active) {
          dispose();
          return;
        }
        unlisten = dispose;

        const snapshot = await ipc.providerSubscribe(kind, normalizedKey);
        subscribed = true;
        if (!active) {
          void ipc.providerUnsubscribe(kind, normalizedKey).catch(() => {});
          return;
        }
        if (snapshot && !receivedEvent) {
          setState({
            subId,
            data: snapshot as ProviderDataMap[K],
            failed: false,
            showSkeleton: false,
          });
        }
      } catch (reason: unknown) {
        if (!active) return;
        setState({ subId, data: null, failed: true, showSkeleton: false });
        void reportError(`useProvider (${kind}): ${String(reason)}`);
      }
    })();

    return () => {
      active = false;
      window.clearTimeout(skeletonTimer);
      unlisten?.();
      if (subscribed) {
        void ipc.providerUnsubscribe(kind, normalizedKey).catch(() => {});
      }
    };
  }, [kind, normalizedKey, subId]);

  const matched = subId !== null && state.subId === subId;
  const data = matched ? state.data : null;
  const failed = matched ? state.failed : false;
  const showSkeleton = matched ? state.showSkeleton : false;

  const status = deriveStatus(kind, data, enabled, failed);
  return {
    status,
    data,
    showSkeleton: status === "loading" && showSkeleton,
  };
}
