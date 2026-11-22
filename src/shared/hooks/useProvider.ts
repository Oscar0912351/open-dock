import { useEffect, useState } from "react";
import type { AgentsSnapshot } from "../bindings/AgentsSnapshot";
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
  agents: AgentsSnapshot;
};

export interface ProviderResult<T> {
  status: ProviderStatus;
  data: T | null;
  showSkeleton: boolean;
}

const LOADING_REVEAL_MS = 300;

interface ChannelState {
  data: unknown | null;
  failed: boolean;
}

interface SharedChannel {
  kind: ProviderKind;
  key: string | null;
  state: ChannelState;
  listeners: Set<(next: ChannelState) => void>;
  unlisten?: () => void;
  subscribed: boolean;
  receivedEvent: boolean;
  disposed: boolean;
}

const channels = new Map<string, SharedChannel>();

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

function retainChannel(
  subId: string,
  kind: ProviderKind,
  normalizedKey: string | null,
  listener: (next: ChannelState) => void,
): { initial: ChannelState; release: () => void } {
  let ch = channels.get(subId);
  if (!ch) {
    ch = {
      kind,
      key: normalizedKey,
      state: { data: null, failed: false },
      listeners: new Set(),
      subscribed: false,
      receivedEvent: false,
      disposed: false,
    };
    channels.set(subId, ch);

    const channel = ch;
    const notify = (next: ChannelState) => {
      if (channel.disposed) return;
      channel.state = next;
      for (const cb of channel.listeners) {
        cb(next);
      }
    };

    void (async () => {
      try {
        const dispose = await openEventListener(kind, normalizedKey, (payload, failed = false) => {
          channel.receivedEvent = true;
          notify({ data: payload, failed });
        });
        if (channel.disposed) {
          dispose();
          return;
        }
        channel.unlisten = dispose;

        const snapshot = await ipc.providerSubscribe(kind, normalizedKey);
        channel.subscribed = true;
        if (channel.disposed) {
          void ipc.providerUnsubscribe(kind, normalizedKey).catch(() => {});
          return;
        }
        if (snapshot && !channel.receivedEvent) {
          notify({ data: snapshot, failed: false });
        }
      } catch (reason: unknown) {
        if (channel.disposed) return;
        notify({ data: null, failed: true });
        void reportError(`useProvider (${kind}): ${String(reason)}`);
      }
    })();
  }

  ch.listeners.add(listener);
  const retained = ch;
  return {
    initial: retained.state,
    release: () => {
      retained.listeners.delete(listener);
      if (retained.listeners.size === 0 && !retained.disposed) {
        retained.disposed = true;
        channels.delete(subId);
        retained.unlisten?.();
        if (retained.subscribed) {
          void ipc.providerUnsubscribe(retained.kind, retained.key).catch(() => {});
        }
      }
    },
  };
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
  }>(() => {
    const existing = subId ? channels.get(subId)?.state : undefined;
    return {
      subId,
      data: (existing?.data as ProviderDataMap[K] | null) ?? null,
      failed: existing?.failed ?? false,
      showSkeleton: false,
    };
  });

  useEffect(() => {
    if (!subId) {
      return;
    }

    let active = true;
    const skeletonTimer = window.setTimeout(() => {
      if (active) {
        setState((prev) =>
          prev.subId === subId
            ? { ...prev, showSkeleton: true }
            : { subId, data: null, failed: false, showSkeleton: true },
        );
      }
    }, LOADING_REVEAL_MS);

    const { initial, release } = retainChannel(subId, kind, normalizedKey, (next) => {
      if (!active) return;
      setState({
        subId,
        data: next.data as ProviderDataMap[K] | null,
        failed: next.failed,
        showSkeleton: false,
      });
    });

    if (initial.data !== null || initial.failed) {
      queueMicrotask(() => {
        if (!active) return;
        setState({
          subId,
          data: initial.data as ProviderDataMap[K] | null,
          failed: initial.failed,
          showSkeleton: false,
        });
      });
    }

    return () => {
      active = false;
      window.clearTimeout(skeletonTimer);
      release();
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
