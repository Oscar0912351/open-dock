import { Globe, MapPin } from "@phosphor-icons/react";
import { useEffect } from "react";
import type { Location } from "../../../shared/bindings/Location";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import { formatTemperature, formatTime, getDateTimeFormatter } from "../../../shared/format";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { WeatherIcon } from "../../../shared/icons/WeatherSprite";
import { resolveWmo } from "../../../shared/wmo";
import { Skeleton, StaleIndicator, StateEmpty, StateError } from "../../components/States";
import { WaveChart } from "./WaveChart";
import "./weather.css";

export interface WeatherSettings {
  locationId: string | null;
  unit: "c" | "f";
  chart: "hourly" | "daily";
}

export const defaultWeatherSettings: WeatherSettings = {
  locationId: null,
  unit: "c",
  chart: "hourly",
};

export function resolveWeatherSettings(raw: Record<string, unknown> | undefined): WeatherSettings {
  if (!raw) return defaultWeatherSettings;
  return {
    locationId:
      typeof raw.locationId === "string" && raw.locationId.trim() ? raw.locationId.trim() : null,
    unit: raw.unit === "f" ? "f" : "c",
    chart: raw.chart === "daily" ? "daily" : "hourly",
  };
}

function convertTemp(celsius: number, unit: "c" | "f"): number {
  return unit === "f" ? (celsius * 9) / 5 + 32 : celsius;
}

function fmtTemp(celsius: number, unit: "c" | "f"): string {
  return formatTemperature(convertTemp(celsius, unit));
}

function formatHourLabel(epochMs: number, timeZone?: string): string {
  const tz = timeZone && timeZone !== "auto" ? timeZone : undefined;
  return getDateTimeFormatter("da-DK", `wx-h:${tz ?? ""}`, {
    hour: "2-digit",
    hourCycle: "h23",
    timeZone: tz,
  }).format(new Date(epochMs));
}

function formatDayShort(dateStr: string): string {
  const date = new Date(`${dateStr}T12:00:00`);
  if (Number.isNaN(date.getTime())) return dateStr;
  return getDateTimeFormatter("da-DK", "wx-d", { weekday: "short" })
    .format(date)
    .replace(".", "")
    .slice(0, 3);
}

interface WeatherProps {
  size: WidgetSize;
  theme: ThemeId;
  settings: WeatherSettings;
  locations: Location[];
  onAriaChange?: (aria: string) => void;
}

export function Weather({ size, theme, settings, locations, onAriaChange }: WeatherProps) {
  const effectiveLocation =
    (settings.locationId
      ? locations.find((loc) => loc.id === settings.locationId)
      : undefined) ??
    locations[0] ??
    null;

  const { status, data: snapshot, showSkeleton } = useProvider(
    "weather",
    effectiveLocation?.id ?? null,
  );

  const cityName = effectiveLocation?.name ?? "";
  const countryName = effectiveLocation?.country ?? "";
  const tz = effectiveLocation?.tz;

  useEffect(() => {
    if (!onAriaChange) return;
    if (!effectiveLocation) {
      onAriaChange(t("weather.pickPlace"));
      return;
    }
    if (!snapshot?.current) {
      onAriaChange(cityName);
      return;
    }
    const wmo = resolveWmo(snapshot.current.code, snapshot.current.isDay);
    const curTemp = Math.round(convertTemp(snapshot.current.tempC, settings.unit));
    const today = snapshot.daily[0];
    const hlPart = today
      ? `, højest ${Math.round(convertTemp(today.maxC, settings.unit))}, lavest ${Math.round(convertTemp(today.minC, settings.unit))}`
      : "";
    onAriaChange(`Vejr i ${cityName}: ${curTemp} grader, ${wmo.label.toLowerCase()}${hlPart}`);
  }, [effectiveLocation, cityName, snapshot, settings.unit, onAriaChange]);

  if (!effectiveLocation || status === "empty") {
    return (
      <StateEmpty
        icon={<Globe size="calc(20 * var(--u))" />}
        title={t("weather.pickPlace")}
        subtitle={t("weather.pickPlaceHint")}
      />
    );
  }

  if (status === "error" || (snapshot && !snapshot.current && snapshot.error)) {
    return (
      <StateError
        header={
          <span className="row g4 title-s">
            <MapPin size="calc(11 * var(--u))" />
            <span className="trunc">{cityName}</span>
          </span>
        }
      />
    );
  }

  if (!snapshot?.current) {
    if (!showSkeleton) return null;
    return <WeatherSkeleton size={size} />;
  }

  const current = snapshot.current;
  const wmo = resolveWmo(current.code, current.isDay);
  const today = snapshot.daily[0];
  const isStale = status === "stale" || snapshot.stale;
  const updatedTime =
    Number(snapshot.fetchedAt) > 0
      ? formatTime(new Date(Number(snapshot.fetchedAt)), tz && tz !== "auto" ? tz : undefined)
      : "";
  const safeWrapClass = isStale ? "is-stale-content" : undefined;

  if (size === "s") {
    return (
      <>
        <div className={safeWrapClass} style={{ display: "contents" }}>
          <div className="row between" data-enter>
            <span className="row g4 title-s grow" style={{ minWidth: 0 }}>
              <MapPin size="calc(11 * var(--u))" style={{ flex: "none" }} />
              <span className="trunc">{cityName}</span>
            </span>
            <WeatherIcon id={wmo.icon} size={22} />
          </div>
          <div className="grow" />
          <div className="display-l" data-enter>
            {fmtTemp(current.tempC, settings.unit)}
          </div>
          <div className="meta ink2 wx-s-label trunc" data-enter>
            {wmo.label}
          </div>
        </div>
        {isStale && updatedTime ? (
          <div className="wx-stale-foot">
            <StaleIndicator label={t("weather.updated", { time: updatedTime })} />
          </div>
        ) : (
          today && (
            <div className="micro wx-s-hl" data-enter>
              H {fmtTemp(today.maxC, settings.unit)} · L {fmtTemp(today.minC, settings.unit)}
            </div>
          )
        )}
      </>
    );
  }

  if (size === "m") {
    const nextFiveHours = snapshot.hourly.slice(0, 5);
    return (
      <div className="row g10 wx-m-root">
        <div className={`col wx-m-left${isStale ? " is-stale-content" : ""}`}>
          <span className="row g4 title-s" data-enter>
            <MapPin size="calc(11 * var(--u))" style={{ flex: "none" }} />
            <span className="trunc">{cityName}</span>
          </span>
          <div className="grow" />
          <div className="display-l" data-enter>
            {fmtTemp(current.tempC, settings.unit)}
          </div>
          {isStale && updatedTime ? (
            <div className="wx-stale-foot">
              <StaleIndicator label={t("weather.updated", { time: updatedTime })} />
            </div>
          ) : (
            <div className="meta ink2 wx-s-label trunc" data-enter>
              {wmo.label}
            </div>
          )}
        </div>
        <div className={`row grow between wx-m-hours${isStale ? " is-stale-content" : ""}`}>
          {nextFiveHours.map((hour, idx) => {
            const hWmo = resolveWmo(hour.code, hour.isDay);
            const label = idx === 0 ? t("weather.now") : formatHourLabel(Number(hour.t), tz);
            return (
              <div
                key={String(hour.t)}
                className="col wx-m-col"
                style={idx === 0 ? { color: "var(--ink-1)" } : undefined}
                data-enter
              >
                <span className="micro" style={idx === 0 ? { color: "var(--ink-1)" } : undefined}>
                  {label}
                </span>
                <WeatherIcon id={hWmo.icon} size={17} />
                <span className="value wx-m-temp">{fmtTemp(hour.tempC, settings.unit)}</span>
              </div>
            );
          })}
        </div>
      </div>
    );
  }

  // Size: "l" (276x276)
  const useDaily = settings.chart === "daily" && snapshot.daily.length >= 2;
  const hourlySlots = snapshot.hourly.filter((_, index) => index % 4 === 0).slice(0, 6);
  const precipProb = snapshot.hourly[0]?.precipProb ?? 0;

  const chartValues = useDaily
    ? snapshot.daily.map((d) => convertTemp(d.maxC, settings.unit))
    : snapshot.hourly.map((h) => convertTemp(h.tempC, settings.unit));

  const slotTops = useDaily
    ? snapshot.daily.map((d) => ({
        temp: fmtTemp(d.maxC, settings.unit),
        icon: resolveWmo(d.code, true).icon,
      }))
    : hourlySlots.map((h) => ({
        temp: fmtTemp(h.tempC, settings.unit),
        icon: resolveWmo(h.code, h.isDay).icon,
      }));

  const slotBottoms = useDaily
    ? snapshot.daily.map((d) => formatDayShort(d.date))
    : hourlySlots.map((h, i) => (i === 0 ? t("weather.now") : formatHourLabel(Number(h.t), tz)));

  return (
    <>
      <div className="row between wx-l-head">
        <div className="col g2 grow" style={{ minWidth: 0 }} data-enter>
          <span className="row g4 title">
            <MapPin size="calc(13 * var(--u))" style={{ flex: "none" }} />
            <span className="trunc">{cityName}</span>
          </span>
          {isStale && updatedTime ? (
            <StaleIndicator label={t("weather.updated", { time: updatedTime })} />
          ) : (
            <span className="meta trunc">
              {countryName}
              {updatedTime ? ` · ${t("weather.updated", { time: updatedTime }).toLowerCase()}` : ""}
              {snapshot.source ? ` · ${snapshot.source}` : ""}
            </span>
          )}
        </div>
        <span data-enter="pop">
          <WeatherIcon id={wmo.icon} size={34} />
        </span>
      </div>

      <div className={`row g10 wx-l-temp-row${isStale ? " is-stale-content" : ""}`} data-enter>
        <div className="display-xl">{fmtTemp(current.tempC, settings.unit)}</div>
        <div className="col g2 wx-l-feels">
          <span className="body trunc">{wmo.label}</span>
          <span className="meta">
            {t("weather.feels", { t: fmtTemp(current.feelsC, settings.unit) })}
          </span>
        </div>
      </div>

      <div className={`row g12 wx-l-metrics${isStale ? " is-stale-content" : ""}`} data-enter>
        <span className="row g4">
          <span className="ink2">
            <WeatherIcon id="wx-wind" size={14} />
          </span>
          <span className="value wx-l-metric-val">{Math.round(current.windKmh)} km/t</span>
        </span>
        <span className="row g4">
          <span className="ink2">
            <WeatherIcon id="wx-drop" size={14} />
          </span>
          <span className="value wx-l-metric-val">{Math.round(precipProb)}\u202F%</span>
        </span>
        <span className="row g4">
          <span className="ink2">
            <WeatherIcon id="wx-gust" size={14} />
          </span>
          <span className="value wx-l-metric-val">{Math.round(current.gustKmh)} km/t</span>
        </span>
      </div>

      <div className="grow" />

      <div className={`wx-l-chart-wrap${isStale ? " is-stale-content" : ""}`}>
        <div className="row between wx-l-slots-top">
          {slotTops.map((slot, idx) => (
            <span key={idx} className="col wx-l-slot" data-enter>
              <span className="micro ink2">{slot.temp}</span>
              <span className="ink2">
                <WeatherIcon id={slot.icon} size={14} />
              </span>
            </span>
          ))}
        </div>
        <WaveChart
          values={chartValues}
          width={248}
          height={48}
          theme={theme}
          dotsEvery={useDaily ? 1 : 4}
          split={useDaily ? 0.16 : 0.26}
        />
        <div className="row between wx-l-slots-bot">
          {slotBottoms.map((label, idx) => (
            <span
              key={idx}
              className={`micro wx-l-time${idx === 0 ? " is-now" : ""}`}
              data-enter
            >
              {label}
            </span>
          ))}
        </div>
      </div>
    </>
  );
}

function WeatherSkeleton({ size }: { size: WidgetSize }) {
  if (size === "s") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <div className="row between">
          <Skeleton width={56} height={12} />
          <Skeleton width={22} height={22} circle />
        </div>
        <Skeleton width={52} height={38} />
        <div className="col g4">
          <Skeleton width="72%" height={10} />
          <Skeleton width="55%" height={9} />
        </div>
      </div>
    );
  }
  if (size === "m") {
    return (
      <div className="row g12" style={{ height: "100%" }}>
        <div className="col between" style={{ width: "calc(100 * var(--u))" }}>
          <Skeleton width={68} height={12} />
          <Skeleton width={54} height={36} />
          <Skeleton width={76} height={10} />
        </div>
        <div className="row grow between" style={{ alignItems: "center" }}>
          {[0, 1, 2, 3, 4].map((i) => (
            <div key={i} className="col g8" style={{ alignItems: "center" }}>
              <Skeleton width={16} height={9} />
              <Skeleton width={17} height={17} circle />
              <Skeleton width={18} height={11} />
            </div>
          ))}
        </div>
      </div>
    );
  }
  return (
    <div className="col between" style={{ height: "100%" }}>
      <div className="row between">
        <div className="col g6">
          <Skeleton width={88} height={14} />
          <Skeleton width={116} height={10} />
        </div>
        <Skeleton width={34} height={34} circle />
      </div>
      <Skeleton width={90} height={50} />
      <Skeleton width="78%" height={12} />
      <Skeleton width="100%" height={78} radius={10} />
    </div>
  );
}
