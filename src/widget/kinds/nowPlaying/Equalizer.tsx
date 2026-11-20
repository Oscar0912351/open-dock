import type { CSSProperties } from "react";

interface EqualizerProps {
  playing: boolean;
  height?: number;
  className?: string;
}

const BARS = [
  { d: "0.9s", dl: "-0.2s", h: 0.3 },
  { d: "1.1s", dl: "-0.5s", h: 0.55 },
  { d: "0.8s", dl: "-0.1s", h: 0.4 },
  { d: "1.25s", dl: "-0.7s", h: 0.25 },
] as const;

export function Equalizer({ playing, height = 13, className = "" }: EqualizerProps) {
  const wrapStyle: CSSProperties & Record<string, string | number> = { "--eqh": height };
  return (
    <div
      className={`eq${playing ? "" : " paused"}${className ? ` ${className}` : ""}`}
      style={wrapStyle}
      aria-hidden="true"
    >
      {BARS.map((bar, index) => (
        <i
          key={index}
          style={
            {
              "--d": bar.d,
              "--dl": bar.dl,
              "--h": bar.h,
            } as CSSProperties
          }
        />
      ))}
    </div>
  );
}
