export type WeatherIconId =
  | "wx-sun"
  | "wx-moon"
  | "wx-cloud"
  | "wx-cloud-sun"
  | "wx-cloud-moon"
  | "wx-fog"
  | "wx-drizzle"
  | "wx-rain"
  | "wx-sleet"
  | "wx-snow"
  | "wx-hail"
  | "wx-thunder"
  | "wx-wind"
  | "wx-drop"
  | "wx-gust";

export interface WmoEntry {
  label: string;
  dayIcon: WeatherIconId;
  nightIcon: WeatherIconId;
}

const WMO_TABLE: Record<number, WmoEntry> = {
  0: { label: "Klart", dayIcon: "wx-sun", nightIcon: "wx-moon" },
  1: { label: "Mest klart", dayIcon: "wx-cloud-sun", nightIcon: "wx-cloud-moon" },
  2: { label: "Delvist skyet", dayIcon: "wx-cloud-sun", nightIcon: "wx-cloud-moon" },
  3: { label: "Overskyet", dayIcon: "wx-cloud", nightIcon: "wx-cloud" },
  45: { label: "Tåge", dayIcon: "wx-fog", nightIcon: "wx-fog" },
  48: { label: "Tåge", dayIcon: "wx-fog", nightIcon: "wx-fog" },
  51: { label: "Støvregn", dayIcon: "wx-drizzle", nightIcon: "wx-drizzle" },
  53: { label: "Støvregn", dayIcon: "wx-drizzle", nightIcon: "wx-drizzle" },
  55: { label: "Støvregn", dayIcon: "wx-drizzle", nightIcon: "wx-drizzle" },
  56: { label: "Isslag", dayIcon: "wx-sleet", nightIcon: "wx-sleet" },
  57: { label: "Isslag", dayIcon: "wx-sleet", nightIcon: "wx-sleet" },
  61: { label: "Regn", dayIcon: "wx-rain", nightIcon: "wx-rain" },
  63: { label: "Regn", dayIcon: "wx-rain", nightIcon: "wx-rain" },
  65: { label: "Kraftig regn", dayIcon: "wx-rain", nightIcon: "wx-rain" },
  66: { label: "Isregn", dayIcon: "wx-sleet", nightIcon: "wx-sleet" },
  67: { label: "Isregn", dayIcon: "wx-sleet", nightIcon: "wx-sleet" },
  71: { label: "Sne", dayIcon: "wx-snow", nightIcon: "wx-snow" },
  73: { label: "Sne", dayIcon: "wx-snow", nightIcon: "wx-snow" },
  75: { label: "Sne", dayIcon: "wx-snow", nightIcon: "wx-snow" },
  77: { label: "Sne", dayIcon: "wx-snow", nightIcon: "wx-snow" },
  80: { label: "Byger", dayIcon: "wx-rain", nightIcon: "wx-rain" },
  81: { label: "Byger", dayIcon: "wx-rain", nightIcon: "wx-rain" },
  82: { label: "Kraftige byger", dayIcon: "wx-rain", nightIcon: "wx-rain" },
  85: { label: "Snebyger", dayIcon: "wx-snow", nightIcon: "wx-snow" },
  86: { label: "Snebyger", dayIcon: "wx-snow", nightIcon: "wx-snow" },
  95: { label: "Torden", dayIcon: "wx-thunder", nightIcon: "wx-thunder" },
  96: { label: "Torden med hagl", dayIcon: "wx-hail", nightIcon: "wx-hail" },
  99: { label: "Torden med hagl", dayIcon: "wx-hail", nightIcon: "wx-hail" },
};

const FALLBACK_WMO: WmoEntry = { label: "Overskyet", dayIcon: "wx-cloud", nightIcon: "wx-cloud" };

export function resolveWmo(code: number, isDay = true): { label: string; icon: WeatherIconId } {
  const entry = WMO_TABLE[code] ?? FALLBACK_WMO;
  return {
    label: entry.label,
    icon: isDay ? entry.dayIcon : entry.nightIcon,
  };
}
