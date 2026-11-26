import { describe, expect, it } from "vitest";
import {
  formatCountdown,
  formatDurationShort,
  formatOvertime,
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

describe("formatDurationShort", () => {
  it("viser sekunder under et minut", () => {
    expect(formatDurationShort(45_000)).toBe("45 s");
    expect(formatDurationShort(59_000)).toBe("59 s");
  });

  it("viser minutter og evt. sekunder under en time", () => {
    expect(formatDurationShort(600_000)).toBe("10 min");
    expect(formatDurationShort(150_000)).toBe("2 min 30 s");
  });

  it("viser timer og minutter over en time", () => {
    expect(formatDurationShort(5_400_000)).toBe("1 t 30 min");
    expect(formatDurationShort(3_600_000)).toBe("1 t");
    expect(formatDurationShort(7_200_000)).toBe("2 t");
  });
});
