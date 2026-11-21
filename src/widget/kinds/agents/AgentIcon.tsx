import { OpenAiLogo, Robot } from "@phosphor-icons/react";
import type { ComponentType } from "react";
import aiderLogo from "./logos/aider.png";
import antigravityLogo from "./logos/antigravity.png";
import claudeLogo from "./logos/claude.svg";
import clineLogo from "./logos/cline.svg";
import cursorLogo from "./logos/cursor.svg";
import freebuffLogo from "./logos/freebuff.png";
import geminiLogo from "./logos/googlegemini.svg";
import kiloLogo from "./logos/kilo.png";
import opencodeLogo from "./logos/opencode.svg";
import qoderLogo from "./logos/qoder.png";
import windsurfLogo from "./logos/windsurf.svg";

type Glyph = { kind: "img"; src: string } | { kind: "icon"; Icon: ComponentType<{ size?: number | string }> };

interface AgentStyle {
  glyph: Glyph;
  /** Farve brugt til tile og status-prik. */
  tint: string;
}

/** Rigtige app-logoer pr. understøttet agent. Fallback: neutral robot. */
const AGENT_STYLES: Record<string, AgentStyle> = {
  "claude-code": { glyph: { kind: "img", src: claudeLogo }, tint: "#E0824D" },
  codex: { glyph: { kind: "icon", Icon: OpenAiLogo }, tint: "#E7E9EE" },
  gemini: { glyph: { kind: "img", src: geminiLogo }, tint: "#8E75B2" },
  opencode: { glyph: { kind: "img", src: opencodeLogo }, tint: "#D4D4D8" },
  cline: { glyph: { kind: "img", src: clineLogo }, tint: "#4ADE80" },
  kilo: { glyph: { kind: "img", src: kiloLogo }, tint: "#A78BFA" },
  cursor: { glyph: { kind: "img", src: cursorLogo }, tint: "#D4D4D8" },
  windsurf: { glyph: { kind: "img", src: windsurfLogo }, tint: "#22D3EE" },
  qoder: { glyph: { kind: "img", src: qoderLogo }, tint: "#FBBF24" },
  antigravity: { glyph: { kind: "img", src: antigravityLogo }, tint: "#C084FC" },
  aider: { glyph: { kind: "img", src: aiderLogo }, tint: "#4ADE80" },
  freebuff: { glyph: { kind: "img", src: freebuffLogo }, tint: "#F472B6" },
};

const FALLBACK: AgentStyle = { glyph: { kind: "icon", Icon: Robot }, tint: "#94A3B8" };

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
  const { glyph, tint } = AGENT_STYLES[agent] ?? FALLBACK;
  return (
    <span className="ag-icon" style={{ background: `${tint}26`, color: tint }}>
      {glyph.kind === "img" ? (
        <img className="ag-logo" src={glyph.src} width={size} height={size} alt="" />
      ) : (
        <glyph.Icon size={size} />
      )}
    </span>
  );
}
