export type Point = readonly [number, number];

export function mapPoints(
  values: number[],
  width: number,
  height: number,
  padTop = 6,
  padBottom = 6,
  min?: number,
  max?: number,
): Point[] {
  if (values.length === 0) return [];
  if (values.length === 1) {
    const midY = padTop + (height - padTop - padBottom) * 0.5;
    return [[0, midY], [width, midY]];
  }
  const lo = min ?? Math.min(...values);
  const hi = max ?? Math.max(...values);
  const span = hi - lo || 1;
  const innerH = Math.max(1, height - padTop - padBottom);
  return values.map((v, i) => [
    (i / (values.length - 1)) * width,
    padTop + (1 - (v - lo) / span) * innerH,
  ]);
}

/** Catmull-Rom spline converted to cubic Bezier SVG path. */
export function catmullRomPath(points: Point[]): string {
  const first = points[0];
  if (!first) return "";
  if (points.length === 1) return `M${first[0].toFixed(1)},${first[1].toFixed(1)}`;
  let d = `M${first[0].toFixed(1)},${first[1].toFixed(1)}`;
  for (let i = 0; i < points.length - 1; i++) {
    const p1 = points[i]!;
    const p2 = points[i + 1]!;
    const p0 = points[i - 1] ?? p1;
    const p3 = points[i + 2] ?? p2;
    const cp1x = p1[0] + (p2[0] - p0[0]) / 6;
    const cp1y = p1[1] + (p2[1] - p0[1]) / 6;
    const cp2x = p2[0] - (p3[0] - p1[0]) / 6;
    const cp2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C${cp1x.toFixed(1)},${cp1y.toFixed(1)} ${cp2x.toFixed(1)},${cp2y.toFixed(1)} ${p2[0].toFixed(1)},${p2[1].toFixed(1)}`;
  }
  return d;
}
