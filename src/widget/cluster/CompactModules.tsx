import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import { formatPercent, formatTemperature, formatTime } from "../../shared/format";
import { t } from "../../shared/i18n";
import { WeatherIcon } from "../../shared/icons/WeatherSprite";
import { resolveWmo } from "../../shared/wmo";
import { resolveWeatherSettings } from "../kinds/weather/Weather";
import { WIDGET_REGISTRY, type FreeWidgetKind } from "../registry";
import {
  type ClusterModuleRenderProps,
  type ClusterModuleSharedData,
} from "./clusterModuleShared";

export type { ClusterModuleSharedData } from "./clusterModuleShared";

interface ModuleCardProps extends Omit<ClusterModuleRenderProps, "mode"> {
  mode: ModuleMode;
}

export function ClusterModuleView({ mod, mode, shared, onToggleExpand }: ModuleCardProps) {
  if (mode === "hidden") return null;
  return (
    WIDGET_REGISTRY[mod.kind as FreeWidgetKind]?.renderCluster({
      mod,
      mode,
      shared,
      onToggleExpand,
    }) ?? null
  );
}

export function ClusterPillBarHeader({
  modules,
  expandedId,
  shared,
  onToggleChip,
}: {
  modules: readonly ClusterModule[];
  expandedId: string | null;
  shared: ClusterModuleSharedData;
  onToggleChip: (moduleId: string) => void;
}) {
  const chips = modules.filter(
    (m) => m.kind === "clock" || m.kind === "weather" || m.kind === "system",
  );
  const cpuPct = shared.system.data ? Math.round(shared.system.data.cpu.usage) : 0;

  return (
    <div className="c-pillbar">
      {chips.map((mod, idx) => {
        const active = expandedId === mod.id;
        return (
          <div className="row g4" key={mod.id} style={{ minWidth: 0 }}>
            {idx > 0 && <span className="c-pill-divider" />}
            <button
              type="button"
              className={`c-pill-chip${active ? " is-active" : ""}`}
              onClick={() => onToggleChip(mod.id)}
            >
              {mod.kind === "clock" && (
                <span className="value tabular">{formatTime(shared.now)}</span>
              )}
              {mod.kind === "weather" && (() => {
                const ws = resolveWeatherSettings(mod.settings);
                const locId = ws.locationId ?? shared.locations[0]?.id ?? "";
                const snap = locId ? shared.weatherByLoc.get(locId)?.data : null;
                const cur = snap?.current;
                const wmo = cur ? resolveWmo(cur.code, cur.isDay) : null;
                const tempVal = cur
                  ? formatTemperature(ws.unit === "f" ? (cur.tempC * 9) / 5 + 32 : cur.tempC)
                  : t("state.dash");
                return (
                  <>
                    <WeatherIcon id={wmo?.icon ?? "wx-cloud-sun"} size={13} />
                    <span className="value tabular">{tempVal}</span>
                  </>
                );
              })()}
              {mod.kind === "system" && (
                <span className="micro tabular">
                  CPU{" "}
                  <b style={{ color: "var(--ink-1)" }}>
                    {shared.system.data ? formatPercent(cpuPct / 100) : t("state.dash")}
                  </b>
                </span>
              )}
            </button>
          </div>
        );
      })}
    </div>
  );
}
