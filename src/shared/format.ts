const NUMBER = new Intl.NumberFormat("da-DK", { maximumFractionDigits: 1 });
const INTEGER = new Intl.NumberFormat("da-DK", { maximumFractionDigits: 0 });

export function formatPercent(ratio: number): string {
  return `${INTEGER.format(ratio * 100)}\u202F%`;
}

export function formatBytes(bytes: number): string {
  const units = ["B", "kB", "MB", "GB", "TB"] as const;
  const absolute = Math.abs(bytes);
  const index = Math.min(
    Math.floor(Math.log(Math.max(absolute, 1)) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / 1024 ** Math.max(0, index);
  return `${NUMBER.format(Number(value.toFixed(1)))}\u202F${units[index]}`;
}

export function formatSpeed(bytesPerSecond: number, bits = false): string {
  if (bytesPerSecond === 0) return bits ? "0\u202Fkbit/s" : "0\u202FkB/s";
  if (!bits) return `${formatBytes(bytesPerSecond)}/s`;

  const units = ["bit", "kbit", "Mbit", "Gbit"] as const;
  const absolute = Math.abs(bytesPerSecond * 8);
  const index = Math.min(
    Math.floor(Math.log(Math.max(absolute, 1)) / Math.log(1000)),
    units.length - 1,
  );
  const value = (bytesPerSecond * 8) / 1000 ** Math.max(0, index);
  return `${NUMBER.format(Number(value.toFixed(1)))}\u202F${units[index]}/s`;
}

export function formatTemperature(celsius: number): string {
  const rounded = Math.round(celsius);
  return `${rounded < 0 ? "−" : ""}${INTEGER.format(Math.abs(rounded))}°`;
}

export function formatFrequencyGhz(mhz: number): string {
  return `${NUMBER.format(Number((mhz / 1000).toFixed(1)))}\u202FGHz`;
}

export function formatTime(date: Date, timeZone?: string, showSeconds = false): string {
  return new Intl.DateTimeFormat("da-DK", {
    hour: "2-digit",
    minute: "2-digit",
    second: showSeconds ? "2-digit" : undefined,
    hourCycle: "h23",
    timeZone,
  }).format(date).replaceAll(".", ":");
}

export function formatDateShort(date: Date, timeZone?: string): string {
  return new Intl.DateTimeFormat("da-DK", {
    weekday: "short",
    day: "numeric",
    month: "short",
    timeZone,
  })
    .format(date)
    .toLocaleLowerCase("da-DK");
}

export function formatDateLong(date: Date, timeZone?: string): string {
  const formatted = new Intl.DateTimeFormat("da-DK", {
    weekday: "long",
    day: "numeric",
    month: "short",
    timeZone,
  }).format(date);
  return formatted.charAt(0).toLocaleUpperCase("da-DK") + formatted.slice(1);
}

export function formatDateAccessible(date: Date, timeZone?: string): string {
  return new Intl.DateTimeFormat("da-DK", {
    weekday: "long",
    day: "numeric",
    month: "long",
    timeZone,
  }).format(date);
}

export function formatDuration(milliseconds: number): string {
  const totalSeconds = Math.max(0, Math.round(milliseconds / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  if (hours > 0) return `${hours} t ${minutes} m`;
  if (minutes > 0) return `${minutes} m`;
  return `${totalSeconds} s`;
}

export function formatCallTimer(milliseconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const mm = String(minutes).padStart(2, "0");
  const ss = String(seconds).padStart(2, "0");
  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function isoWeek(date: Date): number {
  const utc = new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()));
  const day = utc.getUTCDay() || 7;
  utc.setUTCDate(utc.getUTCDate() + 4 - day);
  const yearStart = new Date(Date.UTC(utc.getUTCFullYear(), 0, 1));
  return Math.ceil(((utc.getTime() - yearStart.getTime()) / 86_400_000 + 1) / 7);
}

export function timeZoneCity(timeZone: string | null): string | null {
  if (!timeZone) return null;
  const city = timeZone.split("/").at(-1);
  return city?.replaceAll("_", " ") ?? null;
}

export function timeZoneDeltaHours(date: Date, timeZone: string): number {
  const zoneOffset = zonedTimeAsUtc(date, timeZone) - wallTimeAsUtc(date);
  return Math.round(zoneOffset / 3_600_000) / 1;
}

function wallTimeAsUtc(date: Date): number {
  return Date.UTC(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
    date.getHours(),
    date.getMinutes(),
    date.getSeconds(),
  );
}

function zonedTimeAsUtc(date: Date, timeZone: string): number {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(date);
  const values = new Map(parts.map((part) => [part.type, part.value]));
  return Date.UTC(
    Number(values.get("year")),
    Number(values.get("month")) - 1,
    Number(values.get("day")),
    Number(values.get("hour")),
    Number(values.get("minute")),
    Number(values.get("second")),
  );
}
