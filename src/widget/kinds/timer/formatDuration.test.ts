import { describe, expect, it } from "vitest";
import {
  adjustDuration,
  clampDuration,
  durationParts,
  formatCountdown,
  formatOvertime,
  MAX_DURATION_MS,
  MIN_DURATION_MS,
} from "./formatDuration";

describe("formatCountdown", () => {
  it("formaterer minutter og sekunder", () => {
    expect(formatCountdown(25 * 60_000)).toBe("25:00");
    expect(formatCountdown(60_000)).toBe("1:00");
    expect(formatCountdown(65_000)).toBe("1:05");
  });

  it("runder op så timeren ikke viser 0 for tidligt", () => {
    expect(formatCountdown(900)).toBe("0:01");
    expect(formatCountdown(1000)).toBe("0:01");
  });

  it("viser timer over en time", () => {
    expect(formatCountdown(3600_000)).toBe("1:00:00");
    expect(formatCountdown(3600_000 + 61_000)).toBe("1:01:01");
  });

  it("klamper negative værdier", () => {
    expect(formatCountdown(-500)).toBe("0:00");
  });
});

describe("formatOvertime", () => {
  it("tæller op med plus", () => {
    expect(formatOvertime(0)).toBe("+0:00");
    expect(formatOvertime(23_000)).toBe("+0:23");
    expect(formatOvertime(65_000)).toBe("+1:05");
  });
});

describe("adjustDuration", () => {
  it("justerer minutter i trin af 1 og 10", () => {
    expect(adjustDuration(25 * 60_000, "minutes", 1, false)).toBe(26 * 60_000);
    expect(adjustDuration(25 * 60_000, "minutes", -1, false)).toBe(24 * 60_000);
    expect(adjustDuration(25 * 60_000, "minutes", 1, true)).toBe(35 * 60_000);
  });

  it("justerer sekunder i trin af 5", () => {
    expect(adjustDuration(60_000, "seconds", 1, false)).toBe(65_000);
    expect(adjustDuration(60_000, "seconds", -1, false)).toBe(55_000);
  });

  it("klamper ved minimum og maksimum", () => {
    expect(adjustDuration(MIN_DURATION_MS, "seconds", -1, false)).toBe(MIN_DURATION_MS);
    expect(adjustDuration(MAX_DURATION_MS, "minutes", 1, true)).toBe(MAX_DURATION_MS);
    expect(clampDuration(0)).toBe(MIN_DURATION_MS);
  });
});

describe("durationParts", () => {
  it("splitter korrekt", () => {
    expect(durationParts(65_000)).toEqual({ minutes: 1, seconds: 5 });
    expect(durationParts(25 * 60_000)).toEqual({ minutes: 25, seconds: 0 });
  });
});
