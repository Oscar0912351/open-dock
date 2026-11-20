import type { CSSProperties, ReactNode } from "react";

export type MetricTone = "normal" | "warn" | "crit";

interface RingProps {
  size: number;
  stroke: number;
  pct: number;
  tone?: MetricTone;
  children?: ReactNode;
  animateDurationMs?: number;
  animateRunning?: boolean;
  className?: string;
}

export function Ring({
  size,
  stroke,
  pct,
  tone = "normal",
  children,
  animateDurationMs,
  animateRunning = false,
  className = "",
}: RingProps) {
  const clamped = Math.min(100, Math.max(0, pct));
  const sw = (stroke / size) * 100;
  const r = 50 - sw / 2;
  const angle = (clamped / 100) * Math.PI * 2;
  const kx = 50 + r * Math.cos(angle);
  const ky = 50 + r * Math.sin(angle);

  const ringStyle: CSSProperties & Record<string, string | number> = {
    "--s": size,
  };

  const isCountdown = animateDurationMs !== undefined;
  const fillStyle: CSSProperties & Record<string, string | number> = isCountdown
    ? {
        strokeDasharray: "100 100",
        strokeDashoffset: 100 - clamped,
        ...(animateRunning && animateDurationMs > 0
          ? {
              "--from-offset": 100 - clamped,
              animation: `od-focus-drain ${animateDurationMs}ms linear forwards`,
            }
          : {}),
      }
    : {
        strokeDasharray: `${clamped} 100`,
      };

  const strokeColor =
    tone === "crit" ? "var(--crit)" : tone === "warn" ? "var(--warn)" : undefined;
  if (strokeColor) {
    fillStyle.stroke = strokeColor;
  }

  return (
    <div className={`ring${className ? ` ${className}` : ""}`} style={ringStyle}>
      <svg viewBox="0 0 100 100" aria-hidden="true">
        <circle className="r-track" cx="50" cy="50" r={r} strokeWidth={sw} />
        <circle
          className={`r-fill${isCountdown ? " r-countdown" : ""}`}
          cx="50"
          cy="50"
          r={r}
          strokeWidth={sw}
          pathLength={100}
          style={fillStyle}
        />
        {!isCountdown && clamped > 0 && (
          <circle
            className="r-knob"
            cx={kx.toFixed(2)}
            cy={ky.toFixed(2)}
            r={(sw * 0.62).toFixed(2)}
          />
        )}
      </svg>
      {children && <div className="ring-in">{children}</div>}
    </div>
  );
}
