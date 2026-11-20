import { useId, type CSSProperties } from "react";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import { catmullRomPath, mapPoints, type Point } from "../../../shared/sparkline";

interface SparklineProps {
  values: number[];
  maxPoints?: number;
  width: number;
  height: number;
  theme: ThemeId;
  min?: number;
  max?: number;
}

export function Sparkline({
  values,
  maxPoints = 60,
  width,
  height,
  theme,
  min = 0,
  max = 100,
}: SparklineProps) {
  const id = useId().replaceAll(":", "");

  let points: Point[] = [];
  if (values.length >= 2) {
    if (values.length >= maxPoints) {
      points = mapPoints(values.slice(-maxPoints), width, height, 8, 6, min, max);
    } else {
      const span = max - min || 1;
      const innerH = Math.max(1, height - 14);
      const offset = maxPoints - values.length;
      points = values.map((v, i) => [
        ((offset + i) / Math.max(1, maxPoints - 1)) * width,
        8 + (1 - Math.min(1, Math.max(0, (v - min) / span))) * innerH,
      ]);
    }
  }

  const style: CSSProperties & Record<string, string | number> = {
    width: `calc(${width} * var(--u))`,
    height: `calc(${height} * var(--u))`,
  };

  const first = points[0];
  const last = points[points.length - 1];
  if (!first || !last) {
    return <svg className="curve" viewBox={`0 0 ${width} ${height}`} style={style} aria-hidden="true" />;
  }

  const d = catmullRomPath(points);
  const startX = first[0].toFixed(1);
  const area = `${d} L${last[0].toFixed(1)},${height} L${startX},${height} Z`;

  return (
    <svg className="curve" viewBox={`0 0 ${width} ${height}`} style={style} aria-hidden="true">
      <defs>
        <linearGradient id={`${id}-h`} x1="0" x2="1">
          <stop
            offset="0"
            style={{ stopColor: "var(--curve-ink)", stopOpacity: theme === "aurora" ? 0.35 : 1 }}
          />
          <stop offset="1" style={{ stopColor: "var(--curve-ink)", stopOpacity: 1 }} />
        </linearGradient>
        <linearGradient id={`${id}-v`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" style={{ stopColor: "var(--curve-ink)", stopOpacity: "var(--curve-fill-a)" }} />
          <stop offset="1" style={{ stopColor: "var(--curve-ink)", stopOpacity: 0 }} />
        </linearGradient>
        <clipPath id={`${id}-w`}>
          <rect className="wipe" x="0" y="0" width={width} height={height} />
        </clipPath>
      </defs>
      <path d={area} fill={`url(#${id}-v)`} clipPath={`url(#${id}-w)`} stroke="none" />
      <path className="l1 draw" d={d} pathLength={1} stroke={`url(#${id}-h)`} />
      <path className="l2 draw" d={d} pathLength={1} stroke={`url(#${id}-h)`} />
      <path className="l3 draw" d={d} pathLength={1} stroke={`url(#${id}-h)`} />
      {theme === "laerk" ? (
        <>
          <circle
            className="pt"
            cx={last[0].toFixed(1)}
            cy={last[1].toFixed(1)}
            r="5"
            style={{ fill: "var(--accent)" }}
          />
          <circle
            className="pt pt-ring"
            cx={last[0].toFixed(1)}
            cy={last[1].toFixed(1)}
            r="6"
          />
        </>
      ) : (
        <circle className="pt" cx={last[0].toFixed(1)} cy={last[1].toFixed(1)} r="3" />
      )}
    </svg>
  );
}
