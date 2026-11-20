import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatDateLong,
  formatDateShort,
  formatDuration,
  formatFrequencyGhz,
  formatPercent,
  formatSpeed,
  formatTemperature,
  formatTime,
  isoWeek,
} from "./format";

const date = new Date(2026, 10, 16, 14, 32, 0);

describe("format", () => {
  it("formaterer procent med dansk procentmellemrum", () => {
    expect(formatPercent(0.37)).toBe("37\u202F%");
  });

  it("formaterer bytes binært med dansk decimal", () => {
    expect(formatBytes(11.8 * 1024 ** 3)).toBe("11,8\u202FGB");
    expect(formatBytes(512 * 1024 ** 2)).toBe("512\u202FMB");
  });

  it("formaterer hastigheder i bytes og bits", () => {
    expect(formatSpeed(4.2 * 1024 ** 2)).toBe("4,2\u202FMB/s");
    expect(formatSpeed(860 * 1024)).toBe("860\u202FkB/s");
    expect(formatSpeed(0)).toBe("0\u202FkB/s");
    expect(formatSpeed(4.25 * 1_000_000, true)).toBe("34\u202FMbit/s");
  });

  it("bruger ægte minustegn ved negative temperaturer", () => {
    expect(formatTemperature(-3.2)).toBe("−3°");
    expect(formatTemperature(7.6)).toBe("8°");
  });

  it("formaterer tid og danske datoer med Intl", () => {
    expect(formatTime(date)).toBe("14:32");
    expect(formatDateShort(date)).toBe("man. 16. nov.");
    expect(formatDateLong(date)).toBe("Mandag 16. nov.");
  });

  it("formaterer timer og minutter", () => {
    expect(formatDuration((3 * 60 + 12) * 60 * 1000)).toBe("3 t 12 m");
  });

  it("beregner ISO-ugenummer", () => {
    expect(isoWeek(date)).toBe(47);
  });

  it("formaterer CPU-frekvens i GHz med dansk decimal", () => {
    expect(formatFrequencyGhz(4250)).toBe("4,3\u202FGHz");
    expect(formatFrequencyGhz(3600)).toBe("3,6\u202FGHz");
  });
});
