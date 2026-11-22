import { describe, expect, it } from "vitest";
import type { AgentSession } from "../../shared/bindings/AgentSession";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { DiscordParticipant } from "../../shared/bindings/DiscordParticipant";
import type { DiscordSnapshot } from "../../shared/bindings/DiscordSnapshot";
import {
  AGENTS_EXPANDED_MAX_INLINE_ROWS,
  computeAgentsExpandedHeight,
} from "../kinds/agents/Agents";
import {
  computeDiscordExpandedHeight,
  DISCORD_EXPANDED_MAX_INLINE_ROWS,
} from "../kinds/discord/Discord";
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
  agentsActive: false,
  discordInCall: false,
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
      { mediaActive: true, systemHighLoad: false, agentsActive: false, discordInCall: false },
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

  it("udvider AI Agenter når agenter er aktive (ExpandWhenActive)", () => {
    const modules = [
      mod("clock", "clock", "compact", "none"),
      mod("agents", "agents", "compact", "expandWhenActive"),
    ];

    const calm = resolveModuleModes(modules, idleSignals, null, "adaptive");
    expect(calm.get("agents")).toBe("compact");

    const active = resolveModuleModes(
      modules,
      { ...idleSignals, agentsActive: true },
      null,
      "adaptive",
    );
    expect(active.get("agents")).toBe("expanded");
    expect(computeClusterTargetHeight(modules, active, null, "adaptive", 320)).toBe(191);
  });

  it("manuelt klik udvider valgt modul og komprimerer andre udvidede moduler uden at røre skjulte", () => {
    const modules = makeAllInOneModules();
    const modes = resolveModuleModes(
      modules,
      { mediaActive: true, systemHighLoad: false, agentsActive: false, discordInCall: false },
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
      { mediaActive: true, systemHighLoad: true, agentsActive: false, discordInCall: false },
      null,
      "adaptive",
    );
    expect(computeClusterTargetHeight(modules, modes, "allInOne", "adaptive", 260)).toBe(260);
    expect(computeClusterTargetHeight(modules, modes, "allInOne", "fixed", 220)).toBe(220);
  });

  it("bruger heightOverrides til dynamisk udvidet modulhøjde i vertikal klynge og pillBar", () => {
    const modules = [
      mod("clock", "clock", "compact", "none"),
      mod("agents", "agents", "compact", "expandWhenActive"),
    ];
    const modes = resolveModuleModes(
      modules,
      { ...idleSignals, agentsActive: true },
      null,
      "adaptive",
    );
    const overrides = new Map<string, number>([["agents", 148]]);

    expect(
      computeClusterTargetHeight(modules, modes, null, "adaptive", 480, overrides),
    ).toBe(207);
    expect(
      computeClusterTargetHeight(modules, modes, null, "adaptive", 180, overrides),
    ).toBe(180);
    expect(
      computeClusterTargetHeight(modules, modes, "pillBar", "adaptive", 480, overrides),
    ).toBe(203);
  });
});

function makeAgentSession(id: string, withTodo = false): AgentSession {
  return {
    id,
    agent: "devin",
    agentLabel: "Devin",
    status: "ready",
    title: "Session",
    project: "APPS",
    detail: null,
    subagentCount: 0,
    todo: withTodo ? { done: 2, total: 4, current: "Step" } : null,
    pid: null,
    processRunning: true,
    lastActiveMs: BigInt(1000),
  };
}

describe("computeAgentsExpandedHeight", () => {
  it("skalerer udvidet agent-højde efter synlige sessioner og loft ved max inline rækker", () => {
    expect(computeAgentsExpandedHeight([])).toBe(76);
    expect(computeAgentsExpandedHeight([makeAgentSession("a", false)])).toBe(76);
    expect(computeAgentsExpandedHeight([makeAgentSession("a", true)])).toBe(78);
    expect(
      computeAgentsExpandedHeight([
        makeAgentSession("a", true),
        makeAgentSession("b", false),
        makeAgentSession("c", false),
      ]),
    ).toBe(148);

    const six = Array.from({ length: AGENTS_EXPANDED_MAX_INLINE_ROWS }, (_, i) =>
      makeAgentSession(`s-${i}`, false),
    );
    const nine = Array.from({ length: AGENTS_EXPANDED_MAX_INLINE_ROWS + 3 }, (_, i) =>
      makeAgentSession(`s-${i}`, false),
    );
    expect(computeAgentsExpandedHeight(nine)).toBe(computeAgentsExpandedHeight(six));
  });
});

function makeParticipant(id: string): DiscordParticipant {
  return {
    id,
    username: `user_${id}`,
    displayName: `User ${id}`,
    avatarPath: null,
    speaking: false,
    muted: false,
    deafened: false,
    isSelf: false,
  };
}

function makeDiscordSnapshot(
  inCall: boolean,
  participants: DiscordParticipant[] = [],
  canControlVoice = true,
): DiscordSnapshot {
  return {
    running: true,
    inCall,
    mode: participants.length > 0 ? "rpc" : "audio",
    channelName: inCall ? "General" : null,
    participants,
    selfMute: false,
    selfDeaf: false,
    canControlVoice,
    callStartedAt: inCall ? BigInt(1000) : null,
    updatedAt: BigInt(2000),
  };
}

describe("Discord autoRule og dynamisk højde", () => {
  it("skjuler Discord med expandWhenInCall uden call og udvider under call", () => {
    const modules = [
      mod("clock", "clock", "compact", "none"),
      mod("discord", "discord", "hidden", "expandWhenInCall"),
    ];

    const idle = resolveModuleModes(modules, idleSignals, null, "adaptive");
    expect(idle.get("discord")).toBe("hidden");

    const inCall = resolveModuleModes(
      modules,
      { ...idleSignals, discordInCall: true },
      null,
      "adaptive",
    );
    expect(inCall.get("discord")).toBe("expanded");

    const manualOther = resolveModuleModes(
      modules,
      { ...idleSignals, discordInCall: true },
      "clock",
      "adaptive",
    );
    expect(manualOther.get("clock")).toBe("expanded");
    expect(manualOther.get("discord")).toBe("compact");
  });

  it("beregner dynamisk højde for Discord-modul efter deltagere og loft", () => {
    expect(computeDiscordExpandedHeight(null)).toBe(76);
    expect(computeDiscordExpandedHeight(makeDiscordSnapshot(false))).toBe(76);
    expect(computeDiscordExpandedHeight(makeDiscordSnapshot(true, [], false))).toBe(86);

    const two = [makeParticipant("1"), makeParticipant("2")];
    expect(computeDiscordExpandedHeight(makeDiscordSnapshot(true, two, true))).toBe(131);

    const five = Array.from({ length: DISCORD_EXPANDED_MAX_INLINE_ROWS }, (_, i) =>
      makeParticipant(String(i)),
    );
    const eight = Array.from({ length: DISCORD_EXPANDED_MAX_INLINE_ROWS + 3 }, (_, i) =>
      makeParticipant(String(i)),
    );
    expect(computeDiscordExpandedHeight(makeDiscordSnapshot(true, eight, true))).toBe(
      computeDiscordExpandedHeight(makeDiscordSnapshot(true, five, true)),
    );
  });
});
