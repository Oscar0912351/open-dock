import {
  Asterisk,
  Atom,
  CodeSimple,
  CursorClick,
  DiamondsFour,
  GoogleLogo,
  Hexagon,
  MagicWand,
  OpenAiLogo,
  Robot,
  TerminalWindow,
  Waves,
} from "@phosphor-icons/react";
import type { ComponentType } from "react";

type IconComponent = ComponentType<{ size?: number | string }>;

interface AgentStyle {
  Icon: IconComponent;
  /** Farve brugt til glyph og status-prik. */
  tint: string;
}

/** Brand-nære ikon + nuance pr. understøttet agent. Fallback: neutral robot. */
const AGENT_STYLES: Record<string, AgentStyle> = {
  "claude-code": { Icon: Asterisk, tint: "#E0824D" },
  codex: { Icon: OpenAiLogo, tint: "#E7E9EE" },
  gemini: { Icon: GoogleLogo, tint: "#7EA6F5" },
  opencode: { Icon: TerminalWindow, tint: "#34D399" },
  cline: { Icon: Robot, tint: "#4ADE80" },
  kilo: { Icon: Hexagon, tint: "#A78BFA" },
  cursor: { Icon: CursorClick, tint: "#D4D4D8" },
  windsurf: { Icon: Waves, tint: "#22D3EE" },
  qoder: { Icon: DiamondsFour, tint: "#FBBF24" },
  antigravity: { Icon: Atom, tint: "#C084FC" },
  aider: { Icon: CodeSimple, tint: "#F87171" },
  freebuff: { Icon: MagicWand, tint: "#F472B6" },
};

const FALLBACK: AgentStyle = { Icon: Robot, tint: "#94A3B8" };

export function agentTint(agent: string): string {
  return (AGENT_STYLES[agent] ?? FALLBACK).tint;
}

export function AgentIcon({
  agent,
  size = 15,
}: {
  agent: string;
  size?: number;
}) {
  const { Icon, tint } = AGENT_STYLES[agent] ?? FALLBACK;
  return (
    <span
      className="ag-icon"
      style={{ background: `${tint}26`, color: tint }}
    >
      <Icon size={size} />
    </span>
  );
}
