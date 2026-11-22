import { OpenAiLogo, Robot } from "@phosphor-icons/react";
import type { ComponentType, CSSProperties } from "react";
import aiderLogo from "./logos/aider.png";
import antigravityLogo from "./logos/antigravity.png";
import claudeLogo from "./logos/claude.svg";
import clineLogo from "./logos/cline.svg";
import cursorLogo from "./logos/cursor.svg";
import devinLogo from "./logos/devin.png";
import freebuffLogo from "./logos/freebuff.png";
import geminiLogo from "./logos/googlegemini.svg";
import grokbotLogo from "./logos/grokbot.png";
import hermesLogo from "./logos/hermes.png";
import kiloLogo from "./logos/kilo.png";
import opencodeLogo from "./logos/opencode.svg";
import qoderLogo from "./logos/qoder.png";
import windsurfLogo from "./logos/windsurf.svg";

type Glyph =
  | { kind: "img"; src: string }
  | { kind: "icon"; Icon: ComponentType<{ size?: number | string }> };

interface AgentStyle {
  glyph: Glyph;
  /** Hvid monokrom grafik inverteres automatisk på lyse temaer som Creme/Lærk. */
  mono?: boolean;
}

const AGENT_STYLES: Record<string, AgentStyle> = {
  antigravity: { glyph: { kind: "img", src: antigravityLogo } },
  devin: { glyph: { kind: "img", src: devinLogo }, mono: true },
  "claude-code": { glyph: { kind: "img", src: claudeLogo } },
  codex: { glyph: { kind: "icon", Icon: OpenAiLogo } },
  gemini: { glyph: { kind: "img", src: geminiLogo } },
  opencode: { glyph: { kind: "img", src: opencodeLogo }, mono: true },
  cline: { glyph: { kind: "img", src: clineLogo }, mono: true },
  kilo: { glyph: { kind: "img", src: kiloLogo } },
  cursor: { glyph: { kind: "img", src: cursorLogo }, mono: true },
  windsurf: { glyph: { kind: "img", src: windsurfLogo }, mono: true },
  qoder: { glyph: { kind: "img", src: qoderLogo } },
  aider: { glyph: { kind: "img", src: aiderLogo } },
  freebuff: { glyph: { kind: "img", src: freebuffLogo } },
  "grok-bot": { glyph: { kind: "img", src: grokbotLogo } },
  hermes: { glyph: { kind: "img", src: hermesLogo } },
};

const FALLBACK: AgentStyle = { glyph: { kind: "icon", Icon: Robot } };

export function AgentIcon({
  agent,
  size,
  variant = "default",
}: {
  agent: string;
  size?: number;
  variant?: "default" | "compact";
}) {
  const { glyph, mono } = AGENT_STYLES[agent] ?? FALLBACK;
  const resolvedSize = size ?? (variant === "compact" ? 11 : 13);
  const classes = [
    "ag-icon",
    variant === "compact" ? "ag-icon-sm" : "",
    mono ? "is-mono" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <span className={classes} style={{ "--ag-logo": resolvedSize } as CSSProperties}>
      {glyph.kind === "img" ? (
        <img className="ag-logo" src={glyph.src} alt="" />
      ) : (
        <glyph.Icon size={`calc(${resolvedSize} * var(--u))`} />
      )}
    </span>
  );
}
