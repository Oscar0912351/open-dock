import { describe, expect, it } from "vitest";
import { catmullRomPath, mapPoints } from "./sparkline";

describe("sparkline", () => {
  it("returnerer tom liste og tom sti ved 0 punkter", () => {
    expect(mapPoints([], 200, 40)).toEqual([]);
    expect(catmullRomPath([])).toBe("");
  });

  it("centrerer et enkelt datapunkt vandret fra venstre til højre", () => {
    const pts = mapPoints([42], 200, 40, 5, 5);
    expect(pts).toEqual([
      [0, 20],
      [200, 20],
    ]);
  });

  it("undgår NaN når alle værdier er ens", () => {
    const pts = mapPoints([50, 50, 50], 100, 40, 4, 4);
    expect(pts).toHaveLength(3);
    for (const [x, y] of pts) {
      expect(Number.isFinite(x)).toBe(true);
      expect(Number.isFinite(y)).toBe(true);
    }
    expect(pts[0]).toEqual([0, 36]);
    expect(pts[2]).toEqual([100, 36]);
  });

  it("skalerer punkter efter eksplicit eller udledt min/max", () => {
    const autoPts = mapPoints([10, 20, 30], 100, 50, 5, 5);
    expect(autoPts).toEqual([
      [0, 45],
      [50, 25],
      [100, 5],
    ]);

    const boundedPts = mapPoints([0, 50, 100], 200, 60, 10, 10, 0, 100);
    expect(boundedPts).toEqual([
      [0, 50],
      [100, 30],
      [200, 10],
    ]);
  });

  it("genererer gyldig SVG-sti med kubiske Bezier-kurver", () => {
    expect(catmullRomPath([[12.34, 56.78]])).toBe("M12.3,56.8");

    const d = catmullRomPath([
      [0, 40],
      [50, 10],
      [100, 30],
    ]);
    expect(d.startsWith("M0.0,40.0 C")).toBe(true);
    const segments = d.match(/C[^C]+/g) ?? [];
    expect(segments).toHaveLength(2);
  });
});
