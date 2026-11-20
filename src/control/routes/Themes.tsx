import { Check, Info } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import type { GlassEngine } from "../../shared/bindings/GlassEngine";
import type { MotionPreset } from "../../shared/bindings/MotionPreset";
import type { RadiusPreset } from "../../shared/bindings/RadiusPreset";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { Card, SettingRow } from "../components/Card";
import { ColorSwatches } from "../components/ColorSwatches";
import { Segmented } from "../components/Segmented";
import { Slider } from "../components/Slider";
import { Switch } from "../components/Switch";
import { WidgetPreview } from "../components/WidgetPreview";
import { useControlStore } from "../store";

const THEME_CARDS: ReadonlyArray<{ id: ThemeId; descKey: string }> = [
  { id: "aurora", descKey: "theme.auroraDesc" },
  { id: "laerk", descKey: "theme.laerkDesc" },
  { id: "grafit", descKey: "theme.grafitDesc" },
];

function RadiusGlyph({ r }: { r: number }) {
  return (
    <svg width={12} height={12} viewBox="0 0 12 12" fill="none" aria-hidden="true">
      <path
        d={`M2 10 V${2 + r} Q2 2 ${2 + r} 2 H10`}
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

export function Themes() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const isWindows11 = useControlStore((s) => s.isWindows11);

  const baseGlass = config?.appearance.glassStrength ?? 1;
  const [glassOverride, setGlassOverride] = useState<{ base: number; pct: number } | null>(null);
  const glassDraft =
    glassOverride && glassOverride.base === baseGlass
      ? glassOverride.pct
      : Math.round(baseGlass * 100);

  const [osReducedMotion, setOsReducedMotion] = useState(() =>
    typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)").matches
      : false,
  );
  const glassTimer = useRef<number | null>(null);

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
    };
  }, []);

  if (!config) return null;

  const { appearance } = config;
  const primaryMonitor = monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null;
  const plateUrl = toAssetUrl(primaryMonitor?.platePath ?? "");

  const handleSelectTheme = (themeId: ThemeId) => {
    const clearWhite =
      themeId === "laerk" && appearance.accent?.toUpperCase() === "#FFFFFF";
    void ipc.appearancePatch(clearWhite ? { themeId, accent: null } : { themeId });
  };

  const handleGlassStrength = (pct: number) => {
    setGlassOverride({ base: baseGlass, pct });
    if (glassTimer.current !== null) window.clearTimeout(glassTimer.current);
    glassTimer.current = window.setTimeout(() => {
      glassTimer.current = null;
      void ipc.appearancePatch({ glassStrength: pct / 100 });
    }, 60);
  };

  const engineHelp =
    appearance.glassEngine === "acrylic"
      ? t("engine.acrylicHelp")
      : appearance.glassEngine === "tint"
        ? t("engine.tintHelp")
        : t("engine.wallpaperHelp");

  const activeAppearance = { ...appearance, glassStrength: glassDraft / 100 };

  return (
    <div className="cc-route">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("themes.title")}</h1>
          <p className="cc-page-subtitle">{t("themes.subtitle")}</p>
        </div>
      </div>

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

        <div className="cc-themes-recipe-pill">
          <span>{t(`theme.${appearance.themeId}`)}</span>
          <span>·</span>
          <span>{t(`engine.${appearance.glassEngine}`)}</span>
          <span>·</span>
          <span>{glassDraft} %</span>
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
                    appearance={{ ...appearance, themeId: card.id }}
                    monitors={monitors}
                    locations={config.weather.locations}
                    maxWidth={148}
                    maxHeight={36}
                  />
                  <WidgetPreview
                    id={`theme-wx-${card.id}`}
                    kind="weather"
                    size="s"
                    themeOverride={card.id}
                    appearance={{ ...appearance, themeId: card.id }}
                    monitors={monitors}
                    locations={config.weather.locations}
                    maxWidth={82}
                    maxHeight={82}
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
            <SettingRow title={t("themes.accent")} stacked>
              <ColorSwatches
                value={appearance.accent}
                themeId={appearance.themeId}
                onChange={(accent) => void ipc.appearancePatch({ accent })}
              />
            </SettingRow>

            <SettingRow title={t("themes.glassStrength")}>
              <div style={{ width: 196 }}>
                <Slider
                  value={glassDraft}
                  min={60}
                  max={140}
                  step={5}
                  ariaLabel={t("themes.glassStrength")}
                  formatValue={(v) => `${v} %`}
                  onChange={handleGlassStrength}
                />
              </div>
            </SettingRow>

            <SettingRow title={t("themes.corners")} stacked>
              <Segmented<RadiusPreset>
                fullWidth
                value={appearance.radius}
                ariaLabel={t("themes.corners")}
                options={[
                  { value: "soft", label: t("radius.soft"), icon: <RadiusGlyph r={2} /> },
                  { value: "round", label: t("radius.round"), icon: <RadiusGlyph r={4} /> },
                  { value: "extra", label: t("radius.extra"), icon: <RadiusGlyph r={7} /> },
                ]}
                onChange={(radius) => void ipc.appearancePatch({ radius })}
              />
            </SettingRow>
          </Card>
        </section>

        <section className="cc-section">
          <span className="cc-section-label">{t("themes.groupBehavior")}</span>
          <Card compact>
            <SettingRow title={t("themes.glassEngine")} help={engineHelp} stacked>
              <Segmented<GlassEngine>
                fullWidth
                value={appearance.glassEngine}
                ariaLabel={t("themes.glassEngine")}
                options={[
                  { value: "wallpaper", label: t("themes.glassEngineRec") },
                  {
                    value: "acrylic",
                    label: t("engine.acrylic"),
                    disabled: !isWindows11,
                    title: !isWindows11 ? t("engine.acrylicWin10") : undefined,
                  },
                  { value: "tint", label: t("engine.tint") },
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
              help={t("themes.autoContrastHelp")}
            >
              <Switch
                checked={appearance.autoContrast}
                ariaLabel={t("themes.autoContrast")}
                onChange={(autoContrast) => void ipc.appearancePatch({ autoContrast })}
              />
            </SettingRow>
          </Card>
        </section>
      </div>
    </div>
  );
}
