import { Check, Info } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { GlassEngine } from "../../shared/bindings/GlassEngine";
import type { MotionPreset } from "../../shared/bindings/MotionPreset";
import type { RadiusPreset } from "../../shared/bindings/RadiusPreset";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { Card, SettingRow } from "../components/Card";
import { ChoiceGrid } from "../components/ChoiceGrid";
import { accentLabel, ColorSwatches, THEME_DEFAULT_ACCENT } from "../components/ColorSwatches";
import {
  GlassStrength,
  glassStrengthWord,
  glassToPct,
  pctToGlass,
} from "../components/GlassStrength";
import { Segmented } from "../components/Segmented";
import { Switch } from "../components/Switch";
import { WidgetPreview } from "../components/WidgetPreview";
import { useControlStore } from "../store";

const THEME_CARDS: ReadonlyArray<{ id: ThemeId; descKey: string }> = [
  { id: "aurora", descKey: "theme.auroraDesc" },
  { id: "laerk", descKey: "theme.laerkDesc" },
  { id: "grafit", descKey: "theme.grafitDesc" },
];

const TINT_BG: Record<ThemeId, string> = {
  aurora: "linear-gradient(180deg, rgba(24,24,28,.92), rgba(16,16,20,.96))",
  laerk: "#F4F0E9",
  grafit: "linear-gradient(180deg, #23262B, #1B1D21)",
};

const RADIUS_TILE: Record<RadiusPreset, number> = {
  soft: 3,
  round: 7,
  extra: 12,
};

const RADIUS_SAMPLE: Record<RadiusPreset, number> = {
  soft: 6,
  round: 11,
  extra: 17,
};

// Mini-glasflader i motor- og hjørne-fliserne afspejler både tema, motor og glas-styrke.
function glassChipStyle(
  theme: ThemeId,
  engine: GlassEngine,
  gs: number,
  extraShadow = "",
): CSSProperties {
  const shadow = (base: string) => (extraShadow ? `${base}, ${extraShadow}` : base);

  if (engine === "tint") {
    return {
      background: TINT_BG[theme],
      boxShadow: shadow(
        theme === "laerk"
          ? "inset 0 0 0 1px rgba(46,45,42,.12), inset 0 1px 0 rgba(255,255,255,.70)"
          : "inset 0 0 0 1px rgba(255,255,255,.16), inset 0 1px 0 rgba(255,255,255,.18)",
      ),
    };
  }

  const tNorm = Math.min(1, Math.max(0, (gs - 0.6) / 0.8));
  const blur = Math.max(1.5, (gs - 0.44) * (engine === "acrylic" ? 6.5 : 8.5)).toFixed(1);

  if (engine === "acrylic") {
    if (theme === "laerk") {
      return {
        background: `rgba(244,240,233,${(0.42 + tNorm * 0.50).toFixed(2)})`,
        boxShadow: shadow("inset 0 0 0 1px rgba(46,45,42,.09)"),
        backdropFilter: `blur(${blur}px)`,
        WebkitBackdropFilter: `blur(${blur}px)`,
      };
    }
    const alpha = (theme === "grafit" ? 0.30 : 0.24) + tNorm * 0.54;
    return {
      background: `rgba(18,20,24,${alpha.toFixed(2)})`,
      boxShadow: shadow("inset 0 0 0 1px rgba(255,255,255,.12)"),
      backdropFilter: `blur(${blur}px) saturate(1.15)`,
      WebkitBackdropFilter: `blur(${blur}px) saturate(1.15)`,
    };
  }

  if (theme === "laerk") {
    return {
      background: `rgba(244,240,233,${(0.44 + tNorm * 0.52).toFixed(2)})`,
      boxShadow: shadow("inset 0 0 0 1px rgba(46,45,42,.08), inset 0 1px 0 rgba(255,255,255,.70)"),
      backdropFilter: `blur(${blur}px)`,
      WebkitBackdropFilter: `blur(${blur}px)`,
    };
  }
  if (theme === "grafit") {
    return {
      background: `linear-gradient(180deg, rgba(22,24,28,${(0.24 + tNorm * 0.56).toFixed(2)}), rgba(18,20,24,${(0.32 + tNorm * 0.58).toFixed(2)}))`,
      boxShadow: shadow("inset 0 0 0 1px rgba(255,255,255,.10), inset 0 1px 0 rgba(255,255,255,.14)"),
      backdropFilter: `blur(${blur}px) saturate(1.4)`,
      WebkitBackdropFilter: `blur(${blur}px) saturate(1.4)`,
    };
  }
  return {
    background: `linear-gradient(180deg, rgba(255,255,255,${(0.10 + tNorm * 0.34).toFixed(2)}), rgba(255,255,255,${(0.08 + tNorm * 0.28).toFixed(2)}))`,
    boxShadow: shadow("inset 0 0 0 1px rgba(255,255,255,.22), inset 0 1px 0 rgba(255,255,255,.30)"),
    backdropFilter: `blur(${blur}px) saturate(1.25)`,
    WebkitBackdropFilter: `blur(${blur}px) saturate(1.25)`,
  };
}

function GlassChipPreview({
  engine,
  theme,
  radius,
  glassStrength,
  plateUrl,
}: {
  engine: GlassEngine;
  theme: ThemeId;
  radius: RadiusPreset;
  glassStrength: number;
  plateUrl: string;
}) {
  const { boxShadow, ...glass } = glassChipStyle(theme, engine, glassStrength);
  return (
    <span
      className="cc-eng-prev"
      style={plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined}
    >
      <span
        className="cc-eng-glass"
        style={{ boxShadow, borderRadius: RADIUS_TILE[radius] }}
      >
        <span className="cc-eng-glass-blur" style={glass} />
      </span>
    </span>
  );
}

export function Themes({ embedded = false }: { embedded?: boolean } = {}) {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const isWindows11 = useControlStore((s) => s.isWindows11);

  const baseGlass = config?.appearance.glassStrength ?? 1;
  const [glassOverride, setGlassOverride] = useState<{ base: number; pct: number } | null>(null);
  const glassPct =
    glassOverride && glassOverride.base === baseGlass
      ? glassOverride.pct
      : glassToPct(baseGlass);

  const baseAccent = config?.appearance.accent ?? null;
  const [accentOverride, setAccentOverride] = useState<{
    base: string | null;
    value: string | null;
  } | null>(null);
  const accentDraft =
    accentOverride && accentOverride.base === baseAccent
      ? accentOverride.value
      : baseAccent;

  const [osReducedMotion, setOsReducedMotion] = useState(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
      : false,
  );
  const glassTimer = useRef<number | null>(null);
  const accentTimer = useRef<number | null>(null);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const sync = () => setOsReducedMotion(mq.matches);
    sync();
    mq.addEventListener("change", sync);
    return () => mq.removeEventListener("change", sync);
  }, []);

  useEffect(() => {
    return () => {
      if (glassTimer.current !== null) window.clearTimeout(glassTimer.current);
      if (accentTimer.current !== null) window.clearTimeout(accentTimer.current);
    };
  }, []);

  if (!config) return null;

  const { appearance } = config;
  const primaryMonitor = monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null;
  const plateUrl = toAssetUrl(primaryMonitor?.platePath ?? "");

  const handleSelectTheme = (themeId: ThemeId) => {
    const upper = accentDraft?.toUpperCase();
    const clash =
      (themeId === "laerk" && upper === "#FFFFFF") ||
      (themeId !== "laerk" && upper === "#2E2D2A");
    if (clash) setAccentOverride(null);
    void ipc.appearancePatch(clash ? { themeId, accent: null } : { themeId });
  };

  const handleAccent = (next: string | null) => {
    setAccentOverride({ base: baseAccent, value: next });
    if (accentTimer.current !== null) window.clearTimeout(accentTimer.current);
    accentTimer.current = window.setTimeout(() => {
      accentTimer.current = null;
      void ipc.appearancePatch({ accent: next });
    }, 45);
  };

  const handleGlassStrength = (pct: number) => {
    setGlassOverride({ base: baseGlass, pct });
    if (glassTimer.current !== null) window.clearTimeout(glassTimer.current);
    glassTimer.current = window.setTimeout(() => {
      glassTimer.current = null;
      void ipc.appearancePatch({ glassStrength: pctToGlass(pct) });
    }, 55);
  };

  const isTint = appearance.glassEngine === "tint";
  const isAcrylic = appearance.glassEngine === "acrylic";
  const gs = pctToGlass(glassPct);

  const engineHelp =
    appearance.glassEngine === "acrylic"
      ? `${t("engine.acrylicHelp")} ${t("themes.enginePillNote")}`
      : appearance.glassEngine === "tint"
        ? t("engine.tintHelp")
        : t("engine.wallpaperHelp");

  const activeAppearance = {
    ...appearance,
    glassStrength: gs,
    accent: accentDraft,
  };

  const { boxShadow: recipeShadow, ...recipeGlass } = glassChipStyle(
    appearance.themeId,
    appearance.glassEngine,
    gs,
    "0 1px 5px rgba(0,0,0,.25)",
  );
  const effectiveAccent =
    accentDraft ?? THEME_DEFAULT_ACCENT[appearance.themeId];

  const content = (
    <>
      {!embedded && (
        <div className="cc-page-header">
          <div className="cc-page-header-copy">
            <h1 className="cc-page-title">{t("themes.title")}</h1>
            <p className="cc-page-subtitle">{t("themes.subtitle")}</p>
          </div>
        </div>
      )}

      {/* Live Hero Wallpaper Stage — always visible across all window widths */}
      <div className="cc-themes-hero">
        {plateUrl && (
          <div
            className="cc-preview-bg"
            aria-hidden="true"
            style={{ backgroundImage: `url("${plateUrl}")` }}
          />
        )}
        <div className="cc-stage-scrim" aria-hidden="true" />

        <div className="cc-themes-recipe">
          <span
            className="cc-recipe-sample"
            aria-hidden="true"
            style={{
              boxShadow: recipeShadow,
              borderRadius: RADIUS_SAMPLE[appearance.radius],
            }}
          >
            <span className="cc-recipe-sample-blur" style={recipeGlass} />
            <i
              className="cc-recipe-dot"
              style={{ background: effectiveAccent }}
            />
          </span>
          <span className="cc-recipe-copy">
            <span className="cc-recipe-name">
              {t(`theme.${appearance.themeId}`)}
            </span>
            <span className="cc-recipe-sub">
              {isTint
                ? t(`engine.${appearance.glassEngine}`)
                : `${t(`engine.${appearance.glassEngine}`)} · ${glassPct} %`}
            </span>
          </span>
        </div>

        <div className="cc-themes-hero-widgets">
          <div className="cc-themes-hero-stack">
            <WidgetPreview
              id="hero-np"
              kind="nowPlaying"
              size="pill"
              appearance={activeAppearance}
              monitors={monitors}
              locations={config.weather.locations}
              maxWidth={204}
              maxHeight={48}
              retriggerKey={appearance.motion}
            />
            <WidgetPreview
              id="hero-sys"
              kind="system"
              size="pill"
              appearance={activeAppearance}
              monitors={monitors}
              locations={config.weather.locations}
              maxWidth={204}
              maxHeight={48}
              retriggerKey={appearance.motion}
            />
          </div>

          <div className="cc-themes-hero-row">
            <WidgetPreview
              id="hero-clk"
              kind="clock"
              size="s"
              appearance={activeAppearance}
              monitors={monitors}
              locations={config.weather.locations}
              maxWidth={118}
              maxHeight={118}
              retriggerKey={appearance.motion}
            />
            <WidgetPreview
              id="hero-wx"
              kind="weather"
              size="s"
              appearance={activeAppearance}
              monitors={monitors}
              locations={config.weather.locations}
              maxWidth={118}
              maxHeight={118}
              retriggerKey={appearance.motion}
            />
          </div>
        </div>
      </div>

      {/* 3 Theme Selector Cards */}
      <div className="cc-theme-cards">
        {THEME_CARDS.map((card) => {
          const active = appearance.themeId === card.id;
          return (
            <button
              key={card.id}
              type="button"
              className={`cc-theme-card${active ? " is-selected" : ""}`}
              onClick={() => handleSelectTheme(card.id)}
            >
              <div
                className="cc-theme-plate"
                style={plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined}
              >
                <div className="cc-stage-scrim" aria-hidden="true" />
                <div className="cc-theme-plate-stack">
                  <WidgetPreview
                    id={`theme-np-${card.id}`}
                    kind="nowPlaying"
                    size="pill"
                    themeOverride={card.id}
                    appearance={{ ...activeAppearance, themeId: card.id }}
                    monitors={monitors}
                    locations={config.weather.locations}
                    maxWidth={148}
                    maxHeight={36}
                    retriggerKey={appearance.motion}
                  />
                  <WidgetPreview
                    id={`theme-wx-${card.id}`}
                    kind="weather"
                    size="s"
                    themeOverride={card.id}
                    appearance={{ ...activeAppearance, themeId: card.id }}
                    monitors={monitors}
                    locations={config.weather.locations}
                    maxWidth={82}
                    maxHeight={82}
                    retriggerKey={appearance.motion}
                  />
                </div>
                {active && (
                  <span className="cc-theme-check" aria-hidden="true">
                    <Check size={13} weight="bold" />
                  </span>
                )}
              </div>

              <div className="cc-theme-meta">
                <span className="cc-theme-name">{t(`theme.${card.id}`)}</span>
                <span className="cc-theme-desc">{t(card.descKey)}</span>
              </div>
            </button>
          );
        })}
      </div>

      {/* 2-Column Precision Controls */}
      <div className="cc-themes-controls-grid">
        <section className="cc-section">
          <span className="cc-section-label">{t("themes.groupSurface")}</span>
          <Card compact>
            <SettingRow
              title={t("themes.accent")}
              meta={
                <span className="cc-accent-readout">
                  <span
                    className="cc-accent-readout-dot"
                    aria-hidden="true"
                    style={{ backgroundColor: effectiveAccent }}
                  />
                  <span>{accentLabel(accentDraft)}</span>
                </span>
              }
              stacked
            >
              <ColorSwatches
                value={accentDraft}
                themeId={appearance.themeId}
                onChange={handleAccent}
              />
            </SettingRow>

            <SettingRow
              title={t("themes.glassStrength")}
              meta={isTint ? undefined : `${glassStrengthWord(glassPct)} · ${glassPct} %`}
              help={isTint ? t("themes.tintNoEffect") : undefined}
              stacked
            >
              <GlassStrength
                valuePct={glassPct}
                theme={appearance.themeId}
                engine={appearance.glassEngine}
                plateUrl={plateUrl}
                disabled={isTint}
                onChange={handleGlassStrength}
              />
            </SettingRow>

            <SettingRow
              title={t("themes.corners")}
              help={isAcrylic ? t("themes.cornersAcrylic") : undefined}
              stacked
            >
              <ChoiceGrid<RadiusPreset>
                value={appearance.radius}
                ariaLabel={t("themes.corners")}
                options={(
                  [
                    { value: "soft", labelKey: "radius.soft" },
                    { value: "round", labelKey: "radius.round" },
                    { value: "extra", labelKey: "radius.extra" },
                  ] as const
                ).map((opt) => ({
                  value: opt.value,
                  label: t(opt.labelKey),
                  preview: (
                    <GlassChipPreview
                      radius={opt.value}
                      theme={appearance.themeId}
                      engine={appearance.glassEngine}
                      glassStrength={gs}
                      plateUrl={plateUrl}
                    />
                  ),
                }))}
                onChange={(radius) => void ipc.appearancePatch({ radius })}
              />
            </SettingRow>
          </Card>
        </section>

        <section className="cc-section">
          <span className="cc-section-label">{t("themes.groupBehavior")}</span>
          <Card compact>
            <SettingRow title={t("themes.glassEngine")} help={engineHelp} stacked>
              <ChoiceGrid<GlassEngine>
                value={appearance.glassEngine}
                ariaLabel={t("themes.glassEngine")}
                options={[
                  {
                    value: "wallpaper",
                    label: t("engine.wallpaper"),
                    badge: t("themes.badgeRec"),
                    preview: (
                      <GlassChipPreview
                        engine="wallpaper"
                        theme={appearance.themeId}
                        radius={appearance.radius}
                        glassStrength={gs}
                        plateUrl={plateUrl}
                      />
                    ),
                  },
                  {
                    value: "acrylic",
                    label: t("engine.acrylic"),
                    disabled: !isWindows11,
                    title: !isWindows11 ? t("engine.acrylicWin10") : undefined,
                    preview: (
                      <GlassChipPreview
                        engine="acrylic"
                        theme={appearance.themeId}
                        radius={appearance.radius}
                        glassStrength={gs}
                        plateUrl={plateUrl}
                      />
                    ),
                  },
                  {
                    value: "tint",
                    label: t("engine.tint"),
                    preview: (
                      <GlassChipPreview
                        engine="tint"
                        theme={appearance.themeId}
                        radius={appearance.radius}
                        glassStrength={gs}
                        plateUrl={plateUrl}
                      />
                    ),
                  },
                ]}
                onChange={(glassEngine) => void ipc.appearancePatch({ glassEngine })}
              />
            </SettingRow>

            <SettingRow title={t("themes.motion")} stacked>
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                <Segmented<MotionPreset>
                  fullWidth
                  value={appearance.motion}
                  ariaLabel={t("themes.motion")}
                  options={[
                    { value: "full", label: t("motion.full") },
                    { value: "calm", label: t("motion.calm") },
                    { value: "off", label: t("motion.off") },
                  ]}
                  onChange={(motion) => void ipc.appearancePatch({ motion })}
                />
                {osReducedMotion && (
                  <div className="cc-info-banner">
                    <Info size={14} aria-hidden="true" />
                    <span>{t("motion.windowsReduced")}</span>
                  </div>
                )}
              </div>
            </SettingRow>

            <SettingRow
              title={t("themes.autoContrast")}
              help={
                isTint ? t("themes.tintNoEffect") : t("themes.autoContrastHelp")
              }
            >
              <Switch
                checked={appearance.autoContrast}
                ariaLabel={t("themes.autoContrast")}
                disabled={isTint}
                onChange={(autoContrast) => void ipc.appearancePatch({ autoContrast })}
              />
            </SettingRow>
          </Card>
        </section>
      </div>
    </>
  );

  if (embedded) return content;
  return <div className="cc-route">{content}</div>;
}
