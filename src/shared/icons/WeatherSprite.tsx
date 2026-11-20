import type { CSSProperties } from "react";
import type { WeatherIconId } from "../wmo";

export function WeatherSprite() {
  return (
    <svg width="0" height="0" style={{ position: "absolute" }} aria-hidden="true">
      <defs>
        <linearGradient id="amber-grad" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#E7E1D3" />
          <stop offset=".55" stopColor="#F8C44C" />
          <stop offset="1" stopColor="#F6B12A" />
        </linearGradient>
      </defs>
      <symbol id="wx-sun" viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2.8v2.1M12 19.1v2.1M2.8 12h2.1M19.1 12h2.1M5.5 5.5 7 7M17 17l1.5 1.5M5.5 18.5 7 17M17 7l1.5-1.5" />
      </symbol>
      <symbol id="wx-moon" viewBox="0 0 24 24">
        <path d="M19.5 14.6A7.8 7.8 0 0 1 9.4 4.5a7.8 7.8 0 1 0 10.1 10.1z" />
      </symbol>
      <symbol id="wx-cloud" viewBox="0 0 24 24">
        <path d="M7.2 18.5h10.3a4 4 0 0 0 .4-7.98A5.5 5.5 0 0 0 7.4 9.7a4.4 4.4 0 0 0-.2 8.8z" />
      </symbol>
      <symbol id="wx-cloud-sun" viewBox="0 0 24 24">
        <path d="M5.4 11.2a3.4 3.4 0 0 1 6.3-3.6M8.5 2.6v1.5M3.2 4.8l1 1M2.4 9.9h1.5M13.8 4.8l-1 1" />
        <path d="M9.3 20.2h8.3a3.6 3.6 0 0 0 .3-7.18 5 5 0 0 0-9.4-.9 4.05 4.05 0 0 0 .8 8.08z" />
      </symbol>
      <symbol id="wx-cloud-moon" viewBox="0 0 24 24">
        <path d="M10.6 4.3a4.4 4.4 0 0 0-5.4 6.3" />
        <path d="M9.3 20.2h8.3a3.6 3.6 0 0 0 .3-7.18 5 5 0 0 0-9.4-.9 4.05 4.05 0 0 0 .8 8.08z" />
      </symbol>
      <symbol id="wx-fog" viewBox="0 0 24 24">
        <path d="M5 7.5h14M3.5 11.5h17M6 15.5h12M8.5 19.5h7" />
      </symbol>
      <symbol id="wx-drizzle" viewBox="0 0 24 24">
        <path d="M7.2 15h10.3a3.7 3.7 0 0 0 .35-7.38A5.2 5.2 0 0 0 7.8 6.8 4.1 4.1 0 0 0 7.2 15z" />
        <path d="M9 18.2v.6M13 18.2v.6M11 20.6v.6M15 20.6v.6" />
      </symbol>
      <symbol id="wx-rain" viewBox="0 0 24 24">
        <path d="M7.2 15h10.3a3.7 3.7 0 0 0 .35-7.38A5.2 5.2 0 0 0 7.8 6.8 4.1 4.1 0 0 0 7.2 15z" />
        <path d="M9.2 17.6 8.2 20.6M13.2 17.6l-1 3M17.2 17.6l-1 3" />
      </symbol>
      <symbol id="wx-sleet" viewBox="0 0 24 24">
        <path d="M7.2 15h10.3a3.7 3.7 0 0 0 .35-7.38A5.2 5.2 0 0 0 7.8 6.8 4.1 4.1 0 0 0 7.2 15z" />
        <path d="M9.2 17.6 8.2 20.6M16.2 17.6l-1 3" />
        <circle cx="12.4" cy="19.4" r=".9" fill="currentColor" />
      </symbol>
      <symbol id="wx-snow" viewBox="0 0 24 24">
        <path d="M7.2 15h10.3a3.7 3.7 0 0 0 .35-7.38A5.2 5.2 0 0 0 7.8 6.8 4.1 4.1 0 0 0 7.2 15z" />
        <path d="M9 17.6v3.2M7.6 18.4l2.8 1.6M7.6 20l2.8-1.6M15 17.6v3.2M13.6 18.4l2.8 1.6M13.6 20l2.8-1.6" />
      </symbol>
      <symbol id="wx-hail" viewBox="0 0 24 24">
        <path d="M7.2 15h10.3a3.7 3.7 0 0 0 .35-7.38A5.2 5.2 0 0 0 7.8 6.8 4.1 4.1 0 0 0 7.2 15z" />
        <circle cx="9" cy="18.8" r="1" />
        <circle cx="12.5" cy="20.6" r="1" />
        <circle cx="16" cy="18.8" r="1" />
      </symbol>
      <symbol id="wx-thunder" viewBox="0 0 24 24">
        <path d="M7.2 15h10.3a3.7 3.7 0 0 0 .35-7.38A5.2 5.2 0 0 0 7.8 6.8 4.1 4.1 0 0 0 7.2 15z" />
        <path d="m12.8 15.6-2.1 3.3h3l-2 3.3" />
      </symbol>
      <symbol id="wx-wind" viewBox="0 0 24 24">
        <path d="M3 9h10.5a2.5 2.5 0 1 0-2.5-2.5M3 13h15a2.5 2.5 0 1 1-2.5 2.5M3 17h7" />
      </symbol>
      <symbol id="wx-drop" viewBox="0 0 24 24">
        <path d="M12 3.6s6 6.5 6 10.4a6 6 0 0 1-12 0c0-3.9 6-10.4 6-10.4z" />
      </symbol>
      <symbol id="wx-gust" viewBox="0 0 24 24">
        <path d="M3 8h9a2.5 2.5 0 1 0-2.5-2.5M3 12h16M3 16h11a2.5 2.5 0 1 1-2.5 2.5" />
      </symbol>
    </svg>
  );
}

interface WeatherIconProps {
  id: WeatherIconId;
  size: number;
  className?: string;
}

export function WeatherIcon({ id, size, className = "" }: WeatherIconProps) {
  const style: CSSProperties & Record<string, string | number> = { "--s": size };
  return (
    <svg className={`wx-ic${className ? ` ${className}` : ""}`} style={style} aria-hidden="true">
      <use href={`#${id}`} />
    </svg>
  );
}
