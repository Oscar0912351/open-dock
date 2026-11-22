import type { CSSProperties } from "react";
import type { GlassEngine } from "../../shared/bindings/GlassEngine";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import { t } from "../../shared/i18n";

export function glassToPct(gs: number): number {
  return Math.round(Math.min(100, Math.max(0, ((gs - 0.6) / 0.8) * 100)));
}

export function pctToGlass(pct: number): number {
  return Number((0.6 + (Math.min(100, Math.max(0, pct)) / 100) * 0.8).toFixed(3));
}

export function glassStrengthWord(pct: number): string {
  if (pct <= 18) return t("themes.glassClear");
  if (pct <= 38) return t("themes.glassLight");
  if (pct <= 62) return t("themes.glassNormal");
  if (pct <= 82) return t("themes.glassFrosted");
  return t("themes.glassDense");
}

function scrubGlassStyle(theme: ThemeId, engine: GlassEngine, pct: number): CSSProperties {
  const gs = pctToGlass(pct);
  const blur = Math.max(2, (gs - 0.44) * 11).toFixed(1);
  const tNorm = Math.min(1, Math.max(0, pct / 100));

  if (engine === "acrylic") {
    if (theme === "laerk") {
      return {
        background: `rgba(244,240,233,${(0.36 + tNorm * 0.56).toFixed(2)})`,
        backdropFilter: `blur(${blur}px)`,
        WebkitBackdropFilter: `blur(${blur}px)`,
      };
    }
    const alpha = (theme === "grafit" ? 0.28 : 0.22) + tNorm * 0.56;
    return {
      background: `rgba(18,20,24,${alpha.toFixed(2)})`,
      backdropFilter: `blur(${blur}px) saturate(1.15)`,
      WebkitBackdropFilter: `blur(${blur}px) saturate(1.15)`,
    };
  }

  if (theme === "laerk") {
    return {
      background: `rgba(244,240,233,${(0.38 + tNorm * 0.58).toFixed(2)})`,
      backdropFilter: `blur(${blur}px)`,
      WebkitBackdropFilter: `blur(${blur}px)`,
    };
  }
  if (theme === "grafit") {
    return {
      background: `linear-gradient(180deg, rgba(22,24,28,${(0.24 + tNorm * 0.56).toFixed(2)}), rgba(18,20,24,${(0.32 + tNorm * 0.58).toFixed(2)}))`,
      backdropFilter: `blur(${blur}px) saturate(1.35)`,
      WebkitBackdropFilter: `blur(${blur}px) saturate(1.35)`,
    };
  }
  return {
    background: `linear-gradient(180deg, rgba(255,255,255,${(0.10 + tNorm * 0.34).toFixed(2)}), rgba(255,255,255,${(0.08 + tNorm * 0.28).toFixed(2)}))`,
    backdropFilter: `blur(${blur}px) saturate(1.25)`,
    WebkitBackdropFilter: `blur(${blur}px) saturate(1.25)`,
  };
}

const STOPS: ReadonlyArray<{ pct: number; labelKey: string; min: number; max: number }> = [
  { pct: 0, labelKey: "themes.glassClear", min: 0, max: 15 },
  { pct: 50, labelKey: "themes.glassNormal", min: 40, max: 60 },
  { pct: 100, labelKey: "themes.glassDense", min: 85, max: 100 },
];

export function GlassStrength({
  valuePct,
  theme,
  engine,
  plateUrl,
  disabled = false,
  onChange,
}: {
  valuePct: number;
  theme: ThemeId;
  engine: GlassEngine;
  plateUrl: string;
  disabled?: boolean;
  onChange: (pct: number) => void;
}) {
  const clamped = Math.min(100, Math.max(0, valuePct));
  const fillStyle = scrubGlassStyle(theme, engine, clamped);

  return (
    <div className={`cc-gs${disabled ? " is-disabled" : ""}`}>
      <div
        className="cc-gs-track"
        style={{
          ...(plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined),
          "--pct": `${clamped}%`,
        } as CSSProperties}
      >
        <span className="cc-gs-veil" aria-hidden="true" />
        <span
          className="cc-gs-fill"
          aria-hidden="true"
          style={{ width: `${clamped}%` }}
        >
          <span className="cc-gs-fill-glass" style={fillStyle} />
        </span>
        <span className="cc-gs-mid" aria-hidden="true" />
        <span className="cc-gs-handle" aria-hidden="true" />
        <input
          type="range"
          className="cc-gs-input"
          min={0}
          max={100}
          step={5}
          value={clamped}
          disabled={disabled}
          aria-label={t("themes.glassStrength")}
          aria-valuetext={`${glassStrengthWord(clamped)}, ${clamped} %`}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      </div>

      <div className="cc-gs-stops">
        {STOPS.map((stop) => {
          const active = !disabled && clamped >= stop.min && clamped <= stop.max;
          return (
            <button
              key={stop.pct}
              type="button"
              className={`cc-gs-stop${active ? " is-active" : ""}`}
              disabled={disabled}
              onClick={() => onChange(stop.pct)}
            >
              {t(stop.labelKey)}
            </button>
          );
        })}
      </div>
    </div>
  );
}
