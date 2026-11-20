import type { CSSProperties } from "react";
import {
  formatDateAccessible,
  formatDateLong,
  formatDateShort,
  formatTime,
  isoWeek,
  timeZoneCity,
  timeZoneDeltaHours,
} from "../../../shared/format";
import { t } from "../../../shared/i18n";
import { Analog } from "./Analog";
import { RollingNumber } from "./RollingNumber";
import "./clock.css";

export type ClockSize = "pill" | "s" | "m";
export type ClockTheme = "aurora" | "laerk" | "grafit";

export interface ClockSettings {
  style: "digital" | "analog";
  showSeconds: boolean;
  showDate: boolean;
  timezone: string | null;
  label: string;
  extraZones: string[];
}

export const defaultClockSettings: ClockSettings = {
  style: "digital",
  showSeconds: false,
  showDate: true,
  timezone: null,
  label: "",
  extraZones: [],
};

export function resolveClockSettings(raw: Record<string, unknown> | undefined): ClockSettings {
  if (!raw) return defaultClockSettings;
  return {
    style: raw.style === "analog" ? "analog" : "digital",
    showSeconds: Boolean(raw.showSeconds),
    showDate: raw.showDate === undefined ? true : Boolean(raw.showDate),
    timezone: typeof raw.timezone === "string" ? raw.timezone : null,
    label: typeof raw.label === "string" ? raw.label : "",
    extraZones: Array.isArray(raw.extraZones)
      ? raw.extraZones.filter((item): item is string => typeof item === "string")
      : [],
  };
}

interface ClockProps {
  now: Date;
  settings: ClockSettings;
  size: ClockSize;
  theme: ClockTheme;
}

export function Clock({ now, settings, size, theme }: ClockProps) {
  const timeZone = validTimeZone(settings.timezone);
  const city = settings.label || timeZoneCity(timeZone);
  const time = formatTime(now, timeZone ?? undefined);
  const aria = t("clock.aria", {
    time: formatTime(now, timeZone ?? undefined, settings.showSeconds),
    date: formatDateAccessible(now, timeZone ?? undefined),
  });

  if (size === "s" && settings.style === "analog") {
    return <Analog now={now} timeZone={timeZone} showSeconds={settings.showSeconds} aria={aria} theme={theme} />;
  }

  if (size === "pill") {
    return (
      <section className="clock clock-pill" data-theme={theme} role="group" aria-label={aria}>
        <div className="clock-pill-time display-s" data-enter>
          <RollingNumber value={time} />
          {settings.showSeconds && (
            <span className="clock-seconds value">
              {formatTime(now, timeZone ?? undefined, true).slice(-2)}
            </span>
          )}
        </div>
        <div className="clock-pill-meta col" data-enter>
          <span className="title-s">
            {settings.showDate ? formatDateShort(now, timeZone ?? undefined) : (city ?? t("clock.local"))}
          </span>
          <span className="micro">{city ?? t("clock.week", { n: isoWeek(now) })}</span>
        </div>
      </section>
    );
  }

  if (size === "s") {
    return (
      <section className="clock clock-s" data-theme={theme} role="group" aria-label={aria}>
        <span className="label" data-enter>{city ?? t("clock.today")}</span>
        <div className="grow" />
        <div className="clock-s-time display-l" data-enter><RollingNumber value={time} /></div>
        {settings.showSeconds && (
          <div className="micro clock-inline-seconds" data-enter>{formatTime(now, timeZone ?? undefined, true)}</div>
        )}
        {settings.showDate && <div className="meta clock-date" data-enter>{formatDateLong(now, timeZone ?? undefined)}</div>}
      </section>
    );
  }

  const extraZones = settings.extraZones.filter((zone): zone is string => validTimeZone(zone) !== null).slice(0, 3);
  return (
    <section className="clock clock-m" data-theme={theme} role="group" aria-label={aria}>
      <div className="clock-m-primary col">
        <span className="label" data-enter>{city ?? t("clock.today")}</span>
        <div className="grow" />
        <div className="clock-m-time display-l" data-enter><RollingNumber value={time} /></div>
        {settings.showSeconds && (
          <div className="micro clock-inline-seconds" data-enter>{formatTime(now, timeZone ?? undefined, true)}</div>
        )}
        {settings.showDate && <div className="meta clock-date" data-enter>{formatDateLong(now, timeZone ?? undefined)}</div>}
      </div>
      {extraZones.length > 0 ? (
        <div className="clock-zones col grow">
          {extraZones.map((zone) => (
            <div className="clock-zone row between" data-enter key={zone}>
              <div className="col">
                <span className="title-s">{timeZoneCity(zone)}</span>
                <span className="micro">{formatDelta(timeZoneDeltaHours(now, zone))}</span>
              </div>
              <span className="value">{formatTime(now, zone, settings.showSeconds)}</span>
            </div>
          ))}
        </div>
      ) : (
        <WeekGrid now={now} timeZone={timeZone} />
      )}
    </section>
  );
}

function WeekGrid({ now, timeZone }: { now: Date; timeZone: string | null }) {
  const today = weekdayIndex(now, timeZone ?? undefined);
  return (
    <div className="clock-week" data-enter aria-label={t("clock.week", { n: isoWeek(now) })}>
      {t("clock.weekDays").split(" ").map((day, index) => (
        <span className={`micro clock-week-day ${index === today ? "is-today" : ""}`} key={`${day}-${index}`}>
          {day}
        </span>
      ))}
    </div>
  );
}

export function validTimeZone(zone: string | null): string | null {
  if (!zone) return null;
  try {
    new Intl.DateTimeFormat("da-DK", { timeZone: zone }).format();
    return zone;
  } catch {
    return null;
  }
}

function weekdayIndex(date: Date, timeZone?: string): number {
  const name = new Intl.DateTimeFormat("en-US", { weekday: "short", timeZone }).format(date);
  return ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].indexOf(name);
}

export function formatDelta(hours: number): string {
  const formatted = new Intl.NumberFormat("da-DK", {
    signDisplay: "always",
    maximumFractionDigits: 1,
  }).format(hours);
  return `${formatted.replace("-", "−")} t`;
}

export function secondAnimationStyle(now: Date, timeZone: string | null): CSSProperties {
  const parts = zonedTimeParts(now, timeZone ?? undefined);
  return { "--second-angle": `${parts.second * 6}deg` } as CSSProperties;
}

export function zonedTimeParts(now: Date, timeZone?: string) {
  const parts = new Intl.DateTimeFormat("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
    timeZone,
  }).formatToParts(now);
  const value = (type: Intl.DateTimeFormatPartTypes) =>
    Number(parts.find((part) => part.type === type)?.value ?? 0);
  return { hour: value("hour"), minute: value("minute"), second: value("second") };
}
