import { describe, expect, it } from "vitest";
import { parseTimerInput } from "./parseTimerInput";

// now = 15. januar 2025, 14:00 lokal tid (vintertid).
const NOW = new Date(2025, 0, 15, 14, 0, 0);

function parse(raw: string) {
  return parseTimerInput(raw, NOW);
}

describe("parseTimerInput — grundlæggende", () => {
  it("skiller label og minutter", () => {
    expect(parse("pasta 10")).toEqual({ label: "Pasta", durationMs: 600_000 });
    expect(parse("10 pasta")).toEqual({ label: "Pasta", durationMs: 600_000 });
    expect(parse("pasta 10 min")).toEqual({ label: "Pasta", durationMs: 600_000 });
    expect(parse("pasta 10min")).toEqual({ label: "Pasta", durationMs: 600_000 });
    expect(parse("pasta 10m")).toEqual({ label: "Pasta", durationMs: 600_000 });
  });

  it("fjerner kantord om/i/på/kl", () => {
    expect(parse("ring til mor om 25 minutter")).toEqual({
      label: "Ring til mor",
      durationMs: 1_500_000,
    });
  });

  it("forstår timer+minutter samlet og med mellemrum", () => {
    expect(parse("1t30 vasketøj")).toEqual({ label: "Vasketøj", durationMs: 5_400_000 });
    expect(parse("vasketøj 1t 30m")).toEqual({ label: "Vasketøj", durationMs: 5_400_000 });
    expect(parse("1,5 t")).toEqual({ label: "", durationMs: 5_400_000 });
  });

  it("forstår sekunder og colon-tider", () => {
    expect(parse("90s te")).toEqual({ label: "Te", durationMs: 90_000 });
    expect(parse("ovn 45 sek")).toEqual({ label: "Ovn", durationMs: 45_000 });
    expect(parse("te 2:30")).toEqual({ label: "Te", durationMs: 150_000 });
    expect(parse("1:00:00")).toEqual({ label: "", durationMs: 3_600_000 });
  });

  it("rent tal = minutter (sidste ord, ellers første)", () => {
    expect(parse("3 tæpper 5")).toEqual({ label: "3 tæpper", durationMs: 300_000 });
    expect(parse("10")).toEqual({ label: "", durationMs: 600_000 });
  });
});

describe("parseTimerInput — klokkeslæt", () => {
  it("kl 14:30 i dag (endnu ikke passeret)", () => {
    expect(parse("møde kl 14:30")).toEqual({ label: "Møde", durationMs: 1_800_000 });
  });

  it("kl 9 → i morgen når tidspunktet er passeret", () => {
    const result = parse("kl 9 vækning");
    expect(result?.label).toBe("Vækning");
    expect(result?.durationMs).toBe(19 * 3_600_000);
  });

  it("kl. 15.15 med punktum", () => {
    expect(parse("kl. 15.15 hent")).toEqual({ label: "Hent", durationMs: 4_500_000 });
  });

  it("klokkeslæt forhindrer anden tids-parsning", () => {
    // "kl 15" = 15:00 i dag → 60 min; "30" må ikke lægges til.
    expect(parse("kl 15 og 30")).toEqual({ label: "Og 30", durationMs: 3_600_000 });
  });
});

describe("parseTimerInput — ugyldig input", () => {
  it("ingen tid → null", () => {
    expect(parse("pasta")).toBeNull();
    expect(parse("")).toBeNull();
    expect(parse("   ")).toBeNull();
  });

  it("nul og for lange tider → null", () => {
    expect(parse("0")).toBeNull();
    expect(parse("25t")).toBeNull();
    expect(parse("pasta 0 min")).toBeNull();
  });

  it("ugyldigt klokkeslæt falder tilbage på almindelig parsing", () => {
    expect(parse("kl 25")).toEqual({ label: "", durationMs: 1_500_000 });
  });
});
