import { describe, expect, it } from "vitest";
import type { MediaSnapshot } from "../../../shared/bindings/MediaSnapshot";
import { formatTrackTime } from "./Progress";
import { computeLivePositionMs } from "./useLiveMediaPosition";

function makeSnapshot(overrides: Partial<MediaSnapshot> = {}): MediaSnapshot {
  return {
    status: "playing",
    source: "Spotify",
    title: "Track",
    artist: "Artist",
    album: "Album",
    artUrl: null,
    artColor: null,
    positionMs: 10_000,
    durationMs: 60_000,
    updatedAt: 1_000,
    canPlayPause: true,
    canNext: true,
    canPrev: true,
    canSeek: true,
    shuffle: false,
    repeat: "none",
    availableSources: ["Spotify"],
    sessions: [],
    ...overrides,
  };
}

describe("computeLivePositionMs", () => {
  it("interpolates elapsed time while playing", () => {
    const snap = makeSnapshot({ positionMs: 10_000, updatedAt: 1_000 });
    expect(computeLivePositionMs(snap, 6_500)).toBe(15_500);
  });

  it("clamps interpolated position to durationMs", () => {
    const snap = makeSnapshot({
      positionMs: 55_000,
      durationMs: 60_000,
      updatedAt: 1_000,
    });
    expect(computeLivePositionMs(snap, 20_000)).toBe(60_000);
  });

  it("does not advance position when paused or stopped", () => {
    const paused = makeSnapshot({ status: "paused", positionMs: 12_000, updatedAt: 1_000 });
    expect(computeLivePositionMs(paused, 9_000)).toBe(12_000);
  });

  it("handles clock skew where nowMs precedes updatedAt", () => {
    const snap = makeSnapshot({ positionMs: 10_000, updatedAt: 5_000 });
    expect(computeLivePositionMs(snap, 3_000)).toBe(10_000);
  });

  it("returns 0 when snapshot is null", () => {
    expect(computeLivePositionMs(null, 5_000)).toBe(0);
  });
});

describe("formatTrackTime", () => {
  it("formats short tracks as M:SS and hour-plus tracks as H:MM:SS", () => {
    expect(formatTrackTime(0)).toBe("0:00");
    expect(formatTrackTime(295_000)).toBe("4:55");
    expect(formatTrackTime(3_599_000)).toBe("59:59");
    expect(formatTrackTime(3_600_000)).toBe("1:00:00");
    expect(formatTrackTime(7_509_000)).toBe("2:05:09");
  });
});
