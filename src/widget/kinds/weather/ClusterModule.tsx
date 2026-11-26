import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { MapPin } from "@phosphor-icons/react";
import { memo, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import type { Location } from "../../../shared/bindings/Location";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WeatherSnapshot } from "../../../shared/bindings/WeatherSnapshot";
import { formatTemperature, formatTime } from "../../../shared/format";
import { t, useLocale } from "../../../shared/i18n";
import { WeatherIcon } from "../../../shared/icons/WeatherSprite";
import { resolveWmo } from "../../../shared/wmo";
import { WaveChart } from "./WaveChart";
import { resolveWeatherSettings } from "./Weather";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";

const WeatherClusterMod = memo(function WeatherClusterMod({
  mod,
  mode,
  theme,
  location,
  snapshot,
  status,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  theme: ThemeId;
  location: Location | null;
  snapshot: WeatherSnapshot | null;
  status: string;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveWeatherSettings(mod.settings);
  const toUnit = (c: number) => (settings.unit === "f" ? (c * 9) / 5 + 32 : c);
  const fmtT = (c: number) => formatTemperature(toUnit(c));

  const city = location?.name || t("weather.pickPlace");
  const tz = location?.tz && location.tz !== "auto" ? location.tz : undefined;
  const current = snapshot?.current;
  const wmo = current ? resolveWmo(current.code, current.isDay) : null;

  if (mode === "expanded") {
    const hours = snapshot?.hourly.slice(0, 12) ?? [];
    const temps = hours.map((h) => toUnit(h.tempC));
    const labels = [0, 3, 6, 9, 11]
      .filter((idx) => hours[idx] !== undefined)
      .map((idx, i) => {
        if (i === 0) return t("weather.now");
        const item = hours[idx];
        return item ? formatTime(new Date(item.t), tz).slice(0, 2) : "";
      });

    return (
      <div
        className="c-mod is-exp c-mod-wx-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <div className="col" style={{ minWidth: 0 }}>
            <div className="row g4 title-s">
              <MapPin size="calc(11 * var(--u))" />
              <span className="trunc">{city}</span>
            </div>
            <div className="row g8" style={{ alignItems: "baseline", marginTop: "calc(2 * var(--u))" }}>
              <span className="display-m" style={{ fontSize: "calc(30 * var(--u))" }}>
                {current ? fmtT(current.tempC) : t("state.dash")}
              </span>
              <span className="meta trunc">
                {wmo ? wmo.label : status === "error" ? t("state.error") : t("weather.pickPlace")}
                {current ? ` · ${Math.round(current.windKmh)} km/t` : ""}
              </span>
            </div>
          </div>
          <WeatherIcon id={wmo?.icon ?? "wx-cloud-sun"} size={28} />
        </div>
        {temps.length >= 2 && (
          <>
            <div className="c-wave" style={{ marginTop: "calc(4 * var(--u))" }}>
              <WaveChart values={temps} width={200} height={34} theme={theme} />
            </div>
            <div className="row between" style={{ marginTop: "calc(2 * var(--u))" }}>
              {labels.map((lbl, idx) => (
                <span
                  key={`${lbl}-${idx}`}
                  className="micro"
                  style={idx === 0 ? { color: "var(--ink-1)" } : undefined}
                >
                  {lbl}
                </span>
              ))}
            </div>
          </>
        )}
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-wx-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between g6">
        <div className="row g6 grow">
          <WeatherIcon id={wmo?.icon ?? "wx-cloud-sun"} size={15} />
          <span className="value" style={{ fontSize: "calc(13 * var(--u))" }}>
            {current ? fmtT(current.tempC) : t("state.dash")}
          </span>
          <span className="meta trunc">
            {location ? `${city}${wmo ? ` · ${wmo.label}` : ""}` : t("weather.pickPlace")}
          </span>
        </div>
        <HintBadge label={t("cluster.forecast")} />
      </div>
    </div>
  );
});

export function WeatherClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  const settings = resolveWeatherSettings(mod.settings);
  const locId = settings.locationId ?? shared.locations[0]?.id ?? "";
  const wx = locId ? shared.weatherByLoc.get(locId) : undefined;
  return (
    <WeatherClusterMod
      mod={mod}
      mode={mode}
      theme={shared.theme}
      location={shared.locations.find((l) => l.id === locId) ?? shared.locations[0] ?? null}
      snapshot={wx?.data ?? null}
      status={wx?.status ?? (shared.locations.length === 0 ? "empty" : "loading")}
      preview={shared.preview}
      onToggleExpand={onToggleExpand}
    />
  );
}
