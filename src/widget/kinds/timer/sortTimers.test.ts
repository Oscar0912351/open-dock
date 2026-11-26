import { describe, expect, it } from "vitest";
import type { TimerItem } from "../../../shared/bindings/TimerItem";
import { sortTimers } from "./sortTimers";

let seq = 0;
function item(partial: Partial<TimerItem> & { status: TimerItem["status"] }): TimerItem {
  seq += 1;
  return {
    id: `t${seq}`,
    label: `timer ${seq}`,
    endsAt: null,
    remainingMs: 0,
    totalMs: 60_000,
    initialMs: 60_000,
    createdAt: 0,
    finishedAt: null,
    ...partial,
  };
}

describe("sortTimers", () => {
  it("lægger finished først, så running, så paused", () => {
    const paused = item({ status: "paused", remainingMs: 30_000 });
    const running = item({ status: "running", endsAt: 5_000 });
    const finished = item({ status: "finished", finishedAt: 1_000 });
    expect(sortTimers([paused, running, finished]).map((i) => i.status)).toEqual([
      "finished",
      "running",
      "paused",
    ]);
  });

  it("finished: ældst færdig først", () => {
    const newer = item({ status: "finished", finishedAt: 2_000 });
    const older = item({ status: "finished", finishedAt: 1_000 });
    expect(sortTimers([newer, older])[0]).toBe(older);
  });

  it("running: snarligst udløbende først", () => {
    const later = item({ status: "running", endsAt: 9_000 });
    const sooner = item({ status: "running", endsAt: 3_000 });
    expect(sortTimers([later, sooner])[0]).toBe(sooner);
  });

  it("paused: mindst resterende tid først", () => {
    const more = item({ status: "paused", remainingMs: 50_000 });
    const less = item({ status: "paused", remainingMs: 10_000 });
    expect(sortTimers([more, less])[0]).toBe(less);
  });

  it("ændrer ikke input-rækkefølgen", () => {
    const items = [item({ status: "paused" }), item({ status: "running", endsAt: 1 })];
    const copy = [...items];
    sortTimers(items);
    expect(items).toEqual(copy);
  });
});
