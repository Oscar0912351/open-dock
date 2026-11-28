import { CaretDown } from "@phosphor-icons/react";
import type { CSSProperties, MouseEvent } from "react";
import type { AgentsSnapshot } from "../../shared/bindings/AgentsSnapshot";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { DiscordSnapshot } from "../../shared/bindings/DiscordSnapshot";
import type { Location } from "../../shared/bindings/Location";
import type { MediaSnapshot } from "../../shared/bindings/MediaSnapshot";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { SystemCaps } from "../../shared/bindings/SystemCaps";
import type { SystemSample } from "../../shared/bindings/SystemSample";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { TimerListSnapshot } from "../../shared/bindings/TimerListSnapshot";
import type { WeatherSnapshot } from "../../shared/bindings/WeatherSnapshot";

export interface ClusterModuleSharedData {
  widgetId: string;
  theme: ThemeId;
  editMode: boolean;
  pausedApp: boolean;
  preview?: boolean;
  locations: Location[];
  media: { status: string; data: MediaSnapshot | null };
  system: { status: string; data: SystemSample | null };
  systemHistory: SystemSample[];
  systemCaps: SystemCaps | null;
  weatherByLoc: ReadonlyMap<string, { status: string; data: WeatherSnapshot | null }>;
  discord: { status: string; data: DiscordSnapshot | null };
  agents: { status: string; data: AgentsSnapshot | null };
  agentsNowMs: number;
  /** Timer-snapshots pr. timer-modul-id — hentes i shell så de er levende
      selv når modulet er hidden (pin-regler og auto-udvidelse ved ring). */
  timerSnapshots?: ReadonlyMap<string, TimerListSnapshot>;
  /** Moduler med aktivt input rapporterer fokus hit, så et showOnHover-modul
      ikke kollapser mens brugeren skriver. */
  onModuleFocus?: (moduleId: string, focused: boolean) => void;
}

export interface ClusterModuleRenderProps {
  mod: ClusterModule;
  mode: Exclude<ModuleMode, "hidden">;
  shared: ClusterModuleSharedData;
  onToggleExpand: (id: string) => void;
}

export function shouldIgnoreExpand(event: MouseEvent<HTMLElement>): boolean {
  const target = event.target as HTMLElement | null;
  return Boolean(target?.closest("button, a, input, textarea, [data-no-expand]"));
}

export function viewTransitionStyle(id: string, preview?: boolean): CSSProperties | undefined {
  if (preview) return undefined;
  return { viewTransitionName: `odmod-${id.replace(/[^a-zA-Z0-9_-]/g, "")}` };
}

export function HintBadge({ label }: { label: string }) {
  return (
    <span className="micro c-hint">
      {label}
      <CaretDown size="calc(10 * var(--u))" weight="bold" />
    </span>
  );
}
