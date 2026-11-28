import { useEffect, useState } from "react";
import type { AgentsSnapshot } from "../bindings/AgentsSnapshot";
import type { DiscordSnapshot } from "../bindings/DiscordSnapshot";
import type { MediaSnapshot } from "../bindings/MediaSnapshot";
import type { ProviderKind } from "../bindings/ProviderKind";
import type { SystemSample } from "../bindings/SystemSample";
import type { WeatherSnapshot } from "../bindings/WeatherSnapshot";
import { ipc, on } from "../ipc";
import { fireIpc, reportFailure } from "../logging";

export type ProviderStatus = "loading" | "ready" | "empty" | "error" | "stale";

export type ProviderDataMap = {
  media: MediaSnapshot;
  system: SystemSample;
  weather: WeatherSnapshot;
  discord: DiscordSnapshot;
  agents: AgentsSnapshot;
};

export interface ProviderResult<T> {
  status: ProviderStatus;
  data: T | null;
  showSkeleton: boolean;
}

const LOADING_REVEAL_MS = 300;

function openEventListener(
  kind: ProviderKind,
  normalizedKey: string | null,
  onPayload: (payload: unknown, failed?: boolean) => void,
): Promise<() => void> {
  if (kind === "media") {
    return on.media((payload) => onPayload(payload));
  }
  if (kind === "system") {
    return on.system((payload) => onPayload(payload));
  }
  if (kind === "weather") {
    return on.weather((payload) => {
      if (normalizedKey && payload.locationId !== normalizedKey) return;
      onPayload(payload, Boolean(payload.error && !payload.current));
    });
  }
  if (kind === "discord") {
    return on.discord((payload) => onPayload(payload));
  }
  return on.agents((payload) => onPayload(payload));
}

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

/**
 * Tynd abonnements-hook: lytter på provider-events og parrer ét
 * subscribe/unsubscribe pr. effekt. Refcounting og deduplikering af selve
 * provideren ejes af Rust-scheduleren (pr. window-label, emit deduplikeres pr.
 * vindue) — der er bevidst ingen frontend-cache oveni, så der kun er én
 * sandhed om hvem der abonnerer.
 */
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
    let unlisten: (() => void) | null = null;
    // Om subscribe-invoke'et har fuldført — kun da må der unsubscribe'es,
    // ellers ville vi spise en anden komponents refcount i samme vindue.
    let subscribed = false;

    const skeletonTimer = window.setTimeout(() => {
      if (active) {
        setState((prev) =>
          prev.subId === subId ? { ...prev, showSkeleton: true } : { subId, data: null, failed: false, showSkeleton: true },
        );
      }
    }, LOADING_REVEAL_MS);

    void (async () => {
      let dispose: (() => void) | null = null;
      try {
        dispose = await openEventListener(kind, normalizedKey, (payload, failed = false) => {
          if (!active) return;
          setState({
            subId,
            data: payload as ProviderDataMap[K],
            failed,
            showSkeleton: false,
          });
        });
      } catch (reason: unknown) {
        if (active) {
          setState({ subId, data: null, failed: true, showSkeleton: false });
          reportFailure(`useProvider (${kind})`, reason);
        }
        return;
      }
      if (!active) {
        dispose();
        return;
      }
      unlisten = dispose;
    })();

    void ipc
      .providerSubscribe(kind, normalizedKey)
      .then((snapshot) => {
        subscribed = true;
        if (!active) {
          // Cleanup nåede før svaret — par subscribe/unsubscribe med det samme,
          // så unsubscribe altid sendes efter subscribe på Rust-siden.
          fireIpc(ipc.providerUnsubscribe(kind, normalizedKey), `provider_unsubscribe (${kind})`);
          return;
        }
        if (snapshot) {
          setState({
            subId,
            data: snapshot as ProviderDataMap[K],
            failed: false,
            showSkeleton: false,
          });
        }
      })
      .catch((reason: unknown) => {
        if (!active) return;
        setState({ subId, data: null, failed: true, showSkeleton: false });
        reportFailure(`useProvider (${kind})`, reason);
      });

    return () => {
      active = false;
      window.clearTimeout(skeletonTimer);
      unlisten?.();
      if (subscribed) {
        subscribed = false;
        fireIpc(ipc.providerUnsubscribe(kind, normalizedKey), `provider_unsubscribe (${kind})`);
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
