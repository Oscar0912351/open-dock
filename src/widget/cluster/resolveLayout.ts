import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { ResizeBehavior } from "../../shared/bindings/ResizeBehavior";
import type { WidgetKind } from "../../shared/bindings/WidgetKind";

export interface ClusterSignals {
  mediaActive: boolean;
  systemHighLoad: boolean;
}

export const DEFAULT_CLUSTER_SIGNALS: ClusterSignals = {
  mediaActive: false,
  systemHighLoad: false,
};

export type ClusterModuleKind = Exclude<WidgetKind, "cluster">;

export const MODULE_HEIGHT_U: Record<ClusterModuleKind, Record<ModuleMode, number>> = {
  clock: { hidden: 0, compact: 32, expanded: 76 },
  nowPlaying: { hidden: 0, compact: 40, expanded: 102 },
  system: { hidden: 0, compact: 32, expanded: 104 },
  weather: { hidden: 0, compact: 32, expanded: 112 },
  note: { hidden: 0, compact: 32, expanded: 104 },
  discord: { hidden: 0, compact: 32, expanded: 76 },
};

const FIXED_EXPANSION_ORDER: readonly ClusterModuleKind[] = [
  "weather",
  "system",
  "nowPlaying",
  "note",
  "discord",
  "clock",
];

export function resolveModuleModes(
  modules: readonly ClusterModule[],
  signals: ClusterSignals,
  manualExpandedId: string | null,
  resizeBehavior: ResizeBehavior,
  allowHiddenManual = false,
): Map<string, ModuleMode> {
  const modes = new Map<string, ModuleMode>();

  // 1. Udgangspunkt fra hvert moduls defaultMode
  // 2. Anvend automatiske signal-regler
  for (const mod of modules) {
    let mode: ModuleMode = mod.defaultMode;
    switch (mod.autoRule) {
      case "expandWhenPlaying":
        mode = signals.mediaActive ? "expanded" : "hidden";
        break;
      case "collapseWhenIdle":
        mode = signals.mediaActive ? "compact" : "hidden";
        break;
      case "expandWhenHighLoad":
        mode = signals.systemHighLoad ? "expanded" : "compact";
        break;
      case "none":
      case "fillRemainingSpace":
        break;
    }
    modes.set(mod.id, mode);
  }

  // 3. Manuel klik-override
  if (manualExpandedId !== null) {
    const manualMode = modes.get(manualExpandedId);
    if (manualMode !== undefined && (manualMode !== "hidden" || allowHiddenManual)) {
      modes.set(manualExpandedId, "expanded");
      for (const mod of modules) {
        if (mod.id !== manualExpandedId && modes.get(mod.id) === "expanded") {
          modes.set(mod.id, "compact");
        }
      }
    }
  }

  // 4. FillRemainingSpace og fallback i fixed-ramme
  let expandedCount = 0;
  for (const mod of modules) {
    if (modes.get(mod.id) === "expanded") {
      expandedCount += 1;
    }
  }

  if (expandedCount === 0) {
    const filler = modules.find(
      (mod) => modes.get(mod.id) !== "hidden" && mod.autoRule === "fillRemainingSpace",
    );
    if (filler) {
      modes.set(filler.id, "expanded");
    } else if (resizeBehavior === "fixed") {
      let best: ClusterModule | undefined;
      let bestRank = Number.POSITIVE_INFINITY;
      for (const mod of modules) {
        if (modes.get(mod.id) === "hidden" || mod.kind === "cluster") continue;
        const rank = FIXED_EXPANSION_ORDER.indexOf(mod.kind);
        if (rank !== -1 && rank < bestRank) {
          best = mod;
          bestRank = rank;
        }
      }
      if (best) {
        modes.set(best.id, "expanded");
      }
    }
  }

  return modes;
}

export function moduleHeightUnits(kind: WidgetKind, mode: ModuleMode): number {
  if (kind === "cluster") return 48;
  return MODULE_HEIGHT_U[kind][mode];
}

export function computeClusterTargetHeight(
  modules: readonly ClusterModule[],
  modes: ReadonlyMap<string, ModuleMode>,
  presetId: string | null,
  resizeBehavior: ResizeBehavior,
  maxHeight: number,
): number {
  const cappedMax = Math.max(48, Math.round(maxHeight));
  if (resizeBehavior === "fixed") {
    return cappedMax;
  }

  if (presetId === "pillBar") {
    const expanded = modules.find((mod) => modes.get(mod.id) === "expanded");
    if (!expanded) {
      return 48;
    }
    const expH = moduleHeightUnits(expanded.kind, "expanded");
    return Math.min(cappedMax, Math.max(48, 8 + 32 + 7 + expH + 8));
  }

  let visibleCount = 0;
  let sumHeight = 0;
  for (const mod of modules) {
    const mode = modes.get(mod.id) ?? mod.defaultMode;
    if (mode !== "hidden") {
      visibleCount += 1;
      sumHeight += moduleHeightUnits(mod.kind, mode);
    }
  }

  if (visibleCount === 0) {
    return 48;
  }

  const sepGap = Math.max(0, visibleCount - 1) * 7;
  return Math.min(cappedMax, Math.max(48, 20 + sumHeight + sepGap));
}

export function nextManualExpandedId(
  current: string | null,
  clickedId: string,
): string | null {
  return current === clickedId ? null : clickedId;
}
