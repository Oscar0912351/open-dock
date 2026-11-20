import { useState } from "react";
import { secondAnimationStyle, zonedTimeParts, type ClockTheme } from "./Clock";

interface AnalogProps {
  now: Date;
  timeZone: string | null;
  showSeconds: boolean;
  aria: string;
  theme: ClockTheme;
}

export function Analog({ now, timeZone, showSeconds, aria, theme }: AnalogProps) {
  const { hour, minute, second } = zonedTimeParts(now, timeZone ?? undefined);
  const [secondStyle] = useState(() => secondAnimationStyle(now, timeZone));
  const hourAngle = ((hour % 12) + minute / 60) * 30;
  const minuteAngle = (minute + second / 60) * 6;

  return (
    <section className="clock clock-analog" data-theme={theme} role="group" aria-label={aria}>
      <svg viewBox="0 0 108 108" data-enter aria-hidden="true">
        {Array.from({ length: 12 }, (_, index) => {
          const major = index % 3 === 0;
          return (
            <line
              className={major ? "clock-tick clock-tick-major" : "clock-tick"}
              key={index}
              x1="54"
              y1="4"
              x2="54"
              y2={major ? "11" : "8"}
              transform={`rotate(${index * 30} 54 54)`}
            />
          );
        })}
        <line className="clock-hour clock-hand" x1="54" y1="56" x2="54" y2="26" transform={`rotate(${hourAngle} 54 54)`} />
        <line className="clock-minute clock-hand" x1="54" y1="58" x2="54" y2="12" transform={`rotate(${minuteAngle} 54 54)`} />
        {showSeconds && (
          <g className="clock-second" style={secondStyle}>
            <line x1="54" y1="60" x2="54" y2="8" />
          </g>
        )}
        <circle className={showSeconds ? "clock-center is-second" : "clock-center"} cx="54" cy="54" r="2.5" />
      </svg>
    </section>
  );
}
