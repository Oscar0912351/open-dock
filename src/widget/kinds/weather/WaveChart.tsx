import { useId, type CSSProperties } from "react";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import { catmullRomPath, mapPoints } from "../../../shared/sparkline";

interface WaveChartProps {
  values: number[];
  width?: number;
  height?: number;
  theme: ThemeId;
  dotsEvery?: number;
  split?: number;
}

export function WaveChart({
  values,
  width = 248,
  height = 48,
  theme,
  dotsEvery = 4,
  split = 0.26,
}: WaveChartProps) {
  const id = useId().replaceAll(":", "");
  const style: CSSProperties & Record<string, string | number> = {
    width: `calc(${width} * var(--u))`,
    height: `calc(${height} * var(--u))`,
  };

  if (values.length < 2) {
    return <svg className="curve" viewBox={`0 0 ${width} ${height}`} style={style} aria-hidden="true" />;
  }

  const points = mapPoints(values, width, height, 8, 6);
  const anchor = points[0];
  if (!anchor) {
    return <svg className="curve" viewBox={`0 0 ${width} ${height}`} style={style} aria-hidden="true" />;
  }
  const d = catmullRomPath(points);
  const area = `${d} L${width},${height} L0,${height} Z`;
  const splitX = split * width;

  return (
    <svg className="curve" viewBox={`0 0 ${width} ${height}`} style={style} aria-hidden="true">
      <defs>
        <linearGradient id={`${id}-h`} x1="0" x2="1">
          <stop offset="0" style={{ stopColor: "var(--curve-ink)", stopOpacity: 1 }} />
          <stop
            offset="1"
            style={{ stopColor: "var(--curve-ink)", stopOpacity: theme === "aurora" ? 0.35 : 1 }}
          />
        </linearGradient>
        <linearGradient id={`${id}-v`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" style={{ stopColor: "var(--curve-ink)", stopOpacity: "var(--curve-fill-a)" }} />
          <stop offset="1" style={{ stopColor: "var(--curve-ink)", stopOpacity: 0 }} />
        </linearGradient>
        <clipPath id={`${id}-w`}>
          <rect className="wipe" x="0" y="0" width={width} height={height} />
        </clipPath>
        <clipPath id={`${id}-s`}>
          <rect x="-4" y="-10" width={splitX + 4} height={height + 20} />
        </clipPath>
        <clipPath id={`${id}-r`}>
          <rect x={splitX} y="-10" width={width - splitX + 6} height={height + 20} />
        </clipPath>
      </defs>

      {theme === "laerk" ? (
        <>
          <path className="dotted" d={d} pathLength={100} clipPath={`url(#${id}-r)`} />
          <path
            className="l3 draw"
            d={d}
            pathLength={1}
            stroke="var(--curve-ink)"
            clipPath={`url(#${id}-s)`}
            style={{ strokeWidth: 3.2 }}
          />
          {dotsEvery > 0 &&
            points
              .filter((pt, index) => index % dotsEvery === 0 && pt[0] > splitX)
              .map((pt, idx) => (
                <circle
                  key={idx}
                  className="pt"
                  cx={pt[0].toFixed(1)}
                  cy={pt[1].toFixed(1)}
                  r="2.6"
                />
              ))}
          <circle
            className="pt"
            cx={anchor[0].toFixed(1)}
            cy={anchor[1].toFixed(1)}
            r="5"
            style={{ fill: "var(--accent)" }}
          />
          <circle
            className="pt pt-ring"
            cx={anchor[0].toFixed(1)}
            cy={anchor[1].toFixed(1)}
            r="6"
          />
        </>
      ) : (
        <>
          <path d={area} fill={`url(#${id}-v)`} clipPath={`url(#${id}-w)`} stroke="none" />
          <path className="l1 draw" d={d} pathLength={1} stroke={`url(#${id}-h)`} />
          <path className="l2 draw" d={d} pathLength={1} stroke={`url(#${id}-h)`} />
          <path className="l3 draw" d={d} pathLength={1} stroke={`url(#${id}-h)`} />
          <circle className="pt" cx={anchor[0].toFixed(1)} cy={anchor[1].toFixed(1)} r="3" />
        </>
      )}
    </svg>
  );
}
