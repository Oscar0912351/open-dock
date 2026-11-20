import { describe, expect, it } from "vitest";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import {
  computeClusterTargetHeight,
  nextManualExpandedId,
  resolveModuleModes,
  type ClusterSignals,
} from "./resolveLayout";

function mod(
  id: string,
  kind: ClusterModule["kind"],
  defaultMode: ClusterModule["defaultMode"],
  autoRule: ClusterModule["autoRule"],
): ClusterModule {
  return { id, kind, defaultMode, autoRule, settings: {} };
}

const idleSignals: ClusterSignals = {
  mediaActive: false,
  systemHighLoad: false,
};

function makeAllInOneModules(): ClusterModule[] {
  return [
    mod("clock", "clock", "compact", "none"),
    mod("weather", "weather", "compact", "fillRemainingSpace"),
    mod("system", "system", "compact", "expandWhenHighLoad"),
    mod("media", "nowPlaying", "hidden", "expandWhenPlaying"),
  ];
}

describe("resolveModuleModes", () => {
  it("udvider FillRemainingSpace-modulet når alle auto-moduler er idle", () => {
    const modules = makeAllInOneModules();
    const modes = resolveModuleModes(modules, idleSignals, null, "adaptive");

    expect(modes.get("clock")).toBe("compact");
    expect(modes.get("weather")).toBe("expanded");
    expect(modes.get("system")).toBe("compact");
    expect(modes.get("media")).toBe("hidden");
  });

  it("folder ExpandWhenPlaying ud og trækker FillRemainingSpace sammen når musik starter", () => {
    const modules = makeAllInOneModules();
    const modes = resolveModuleModes(
      modules,
      { mediaActive: true, systemHighLoad: false },
      null,
      "adaptive",
    );

    expect(modes.get("media")).toBe("expanded");
    expect(modes.get("weather")).toBe("compact");
    expect(modes.get("clock")).toBe("compact");
  });

  it("håndterer CollapseWhenIdle til compact når aktiv og hidden når idle", () => {
    const modules = [
      mod("media", "nowPlaying", "compact", "collapseWhenIdle"),
      mod("weather", "weather", "compact", "fillRemainingSpace"),
    ];

    const idle = resolveModuleModes(modules, idleSignals, null, "adaptive");
    expect(idle.get("media")).toBe("hidden");
    expect(idle.get("weather")).toBe("expanded");

    const playing = resolveModuleModes(
      modules,
      { ...idleSignals, mediaActive: true },
      null,
      "adaptive",
    );
    expect(playing.get("media")).toBe("compact");
    expect(playing.get("weather")).toBe("expanded");
  });


  it("udvider System ved høj belastning (ExpandWhenHighLoad)", () => {
    const modules = [
      mod("clock", "clock", "compact", "none"),
      mod("system", "system", "compact", "expandWhenHighLoad"),
    ];

    const normal = resolveModuleModes(modules, idleSignals, null, "adaptive");
    expect(normal.get("system")).toBe("compact");

    const hot = resolveModuleModes(
      modules,
      { ...idleSignals, systemHighLoad: true },
      null,
      "adaptive",
    );
    expect(hot.get("system")).toBe("expanded");
  });

  it("manuelt klik udvider valgt modul og komprimerer andre udvidede moduler uden at røre skjulte", () => {
    const modules = makeAllInOneModules();
    const modes = resolveModuleModes(
      modules,
      { mediaActive: true, systemHighLoad: false },
      "system",
      "adaptive",
    );

    expect(modes.get("system")).toBe("expanded");
    expect(modes.get("media")).toBe("compact");
    expect(modes.get("weather")).toBe("compact");
    expect(modes.get("clock")).toBe("compact");
  });

  it("manuel toggle på samme modul vender tilbage til null (auto)", () => {
    expect(nextManualExpandedId(null, "system")).toBe("system");
    expect(nextManualExpandedId("system", "system")).toBeNull();
    expect(nextManualExpandedId("system", "weather")).toBe("weather");
  });

  it("manualExpandedId vælger ikke et modul der er hidden, medmindre allowHiddenManual er sand", () => {
    const modules = makeAllInOneModules();
    const blocked = resolveModuleModes(modules, idleSignals, "media", "adaptive");
    expect(blocked.get("media")).toBe("hidden");
    expect(blocked.get("weather")).toBe("expanded");

    const allowed = resolveModuleModes(modules, idleSignals, "media", "adaptive", true);
    expect(allowed.get("media")).toBe("expanded");
    expect(allowed.get("weather")).toBe("compact");
  });

  it("i fixed-ramme promoveres et synligt modul til expanded når intet modul ellers er udvidet", () => {
    const modules = [
      mod("clock", "clock", "compact", "none"),
      mod("weather", "weather", "compact", "none"),
    ];

    const adaptive = resolveModuleModes(modules, idleSignals, null, "adaptive");
    expect(adaptive.get("clock")).toBe("compact");
    expect(adaptive.get("weather")).toBe("compact");

    const fixed = resolveModuleModes(modules, idleSignals, null, "fixed");
    expect(fixed.get("clock")).toBe("compact");
    expect(fixed.get("weather")).toBe("expanded");
  });
});

describe("computeClusterTargetHeight", () => {
  it("returnerer 48u for pillBar i hvile og vokser med udvidet modul", () => {
    const pillMods = [
      mod("clock", "clock", "compact", "none"),
      mod("weather", "weather", "compact", "none"),
      mod("system", "system", "compact", "none"),
      mod("media", "nowPlaying", "hidden", "expandWhenPlaying"),
    ];

    const idleModes = resolveModuleModes(pillMods, idleSignals, null, "adaptive");
    expect(computeClusterTargetHeight(pillMods, idleModes, "pillBar", "adaptive", 240)).toBe(48);

    const playModes = resolveModuleModes(
      pillMods,
      { ...idleSignals, mediaActive: true },
      null,
      "adaptive",
    );
    expect(computeClusterTargetHeight(pillMods, playModes, "pillBar", "adaptive", 240)).toBe(157);
  });

  it("respekterer maxHeight i fixed- og adaptive-tilstand", () => {
    const modules = makeAllInOneModules();
    const modes = resolveModuleModes(
      modules,
      { mediaActive: true, systemHighLoad: true },
      null,
      "adaptive",
    );
    expect(computeClusterTargetHeight(modules, modes, "allInOne", "adaptive", 260)).toBe(260);
    expect(computeClusterTargetHeight(modules, modes, "allInOne", "fixed", 220)).toBe(220);
  });
});
