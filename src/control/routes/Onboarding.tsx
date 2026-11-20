import { Check, Plus } from "@phosphor-icons/react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { GeoResult } from "../../shared/bindings/GeoResult";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import { t } from "../../shared/i18n";
import { DockLogo } from "../../shared/icons/DockLogo";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Drawer } from "../components/Drawer";
import { Input } from "../components/Input";
import { WidgetPreview } from "../components/WidgetPreview";
import { useControlStore } from "../store";
import { buildPresetClusters } from "./Gallery";

type StarterId = "allInOne" | "miniTrio" | "musicCalm" | "gamer";

const THEME_ORDER: ReadonlyArray<{ id: ThemeId; descKey: string }> = [
  { id: "aurora", descKey: "theme.auroraDesc" },
  { id: "laerk", descKey: "theme.laerkDesc" },
  { id: "grafit", descKey: "theme.grafitDesc" },
];

function recommendedThemeForLuma(luma: number): ThemeId {
  if (luma < 0.45) return "aurora";
  if (luma <= 0.65) return "grafit";
  return "laerk";
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

export function Onboarding() {
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const setRoute = useControlStore((s) => s.setRoute);
  const pushToast = useControlStore((s) => s.pushToast);

  const primaryMonitor = useMemo(
    () => monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null,
    [monitors],
  );
  const recommendedTheme = useMemo(
    () => recommendedThemeForLuma(primaryMonitor?.avgLuma ?? 0.25),
    [primaryMonitor?.avgLuma],
  );

  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [pickedTheme, setPickedTheme] = useState<ThemeId | null>(null);
  const selectedTheme = pickedTheme ?? recommendedTheme;
  const [selectedStarter, setSelectedStarter] = useState<StarterId>("allInOne");
  const [busy, setBusy] = useState(false);
  const [whereDrawerOpen, setWhereDrawerOpen] = useState(false);
  const [cityQuery, setCityQuery] = useState("");
  const [cityResults, setCityResults] = useState<GeoResult[]>([]);
  const [citySearching, setCitySearching] = useState(false);

  const handleCityQueryChange = (next: string) => {
    setCityQuery(next);
    if (next.trim().length < 2) {
      setCityResults([]);
      setCitySearching(false);
    } else {
      setCitySearching(true);
    }
  };

  const closeWhereDrawer = () => {
    setWhereDrawerOpen(false);
    setCityQuery("");
    setCityResults([]);
    setCitySearching(false);
  };

  useEffect(() => {
    if (!whereDrawerOpen) return;
    const trimmed = cityQuery.trim();
    if (trimmed.length < 2) return;
    let active = true;
    const timer = window.setTimeout(() => {
      void ipc
        .weatherSearch(trimmed)
        .then((rows) => {
          if (active) setCityResults(rows);
        })
        .catch(() => {
          if (active) setCityResults([]);
        })
        .finally(() => {
          if (active) setCitySearching(false);
        });
    }, 300);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [whereDrawerOpen, cityQuery]);

  const finishOnboarding = useCallback(
    async (addedWidgets: boolean) => {
      setBusy(true);
      try {
        await ipc.onboardingComplete();
        setRoute(addedWidgets ? "desktop" : "gallery");
        pushToast({ message: t("toast.onboardingDone"), durationMs: 5000 });
      } catch (err: unknown) {
        pushToast({ message: String(err) });
      } finally {
        setBusy(false);
      }
    },
    [pushToast, setRoute],
  );

  const persistSelectedTheme = useCallback(async () => {
    if (!config) return;
    const clearWhite =
      selectedTheme === "laerk" && config.appearance.accent?.toUpperCase() === "#FFFFFF";
    if (config.appearance.themeId !== selectedTheme || clearWhite) {
      await ipc.appearancePatch(
        clearWhite ? { themeId: selectedTheme, accent: null } : { themeId: selectedTheme },
      );
    }
  }, [config, selectedTheme]);

  const allInOneBoxes = useMemo(() => buildPresetClusters("allInOne"), []);
  const miniTrioBoxes = useMemo(() => buildPresetClusters("miniTrio"), []);

  if (!config) return null;

  const plateUrl = toAssetUrl(primaryMonitor?.platePath ?? "");
  const previewAppearance = {
    ...config.appearance,
    themeId: selectedTheme,
    accent:
      selectedTheme === "laerk" && config.appearance.accent?.toUpperCase() === "#FFFFFF"
        ? null
        : config.appearance.accent,
  };

  const handleSelectTheme = (themeId: ThemeId) => {
    setPickedTheme(themeId);
    const clearWhite =
      themeId === "laerk" && config.appearance.accent?.toUpperCase() === "#FFFFFF";
    void ipc.appearancePatch(clearWhite ? { themeId, accent: null } : { themeId });
  };

  const handleAdvanceFromStep2 = () => {
    void persistSelectedTheme();
    setStep(3);
  };

  const handlePlaceStarter = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await persistSelectedTheme();

      if (selectedStarter === "allInOne") {
        await ipc.clusterAddPreset("allInOne", null);
      } else if (selectedStarter === "miniTrio") {
        await ipc.clusterAddPreset("miniTrio", null);
      } else if (selectedStarter === "musicCalm") {
        await ipc.widgetAdd("nowPlaying", "pill", null);
        await sleep(120);
        await ipc.widgetAdd("clock", "s", null);
        await sleep(120);
        await ipc.widgetAdd("weather", "s", null);
      } else if (selectedStarter === "gamer") {
        await ipc.widgetAdd("system", "pill", null);
        await sleep(120);
        await ipc.widgetAdd("nowPlaying", "pill", null);
        await sleep(120);
        await ipc.widgetAdd("clock", "pill", null);
      }

      const includesWeather = selectedStarter !== "gamer";
      if (includesWeather && config.weather.locations.length === 0) {
        setBusy(false);
        setWhereDrawerOpen(true);
        return;
      }

      await finishOnboarding(true);
    } catch (err: unknown) {
      setBusy(false);
      pushToast({ message: String(err) });
    }
  };

  const handleStartEmpty = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await persistSelectedTheme();
      await finishOnboarding(false);
    } catch (err: unknown) {
      setBusy(false);
      pushToast({ message: String(err) });
    }
  };

  const handlePickCity = async (geo: GeoResult) => {
    if (busy) return;
    setBusy(true);
    try {
      await ipc.weatherLocationAdd(geo);
      closeWhereDrawer();
      await finishOnboarding(true);
    } catch (err: unknown) {
      setBusy(false);
      pushToast({ message: String(err) });
    }
  };

  const handleSkipWhere = () => {
    closeWhereDrawer();
    void finishOnboarding(true);
  };

  return (
    <div className="cc-onboard" role="dialog" aria-modal="true" aria-label={t("onboarding.step1Title")}>
      <div className="cc-onboard-top">
        <button
          type="button"
          className="cc-link-btn cc-onboard-skip"
          disabled={busy}
          onClick={() => void finishOnboarding(config.widgets.length > 0)}
        >
          {t("onboarding.skip")}
        </button>
      </div>

      <div className="cc-onboard-body">
        {step === 1 && (
          <div className="cc-onboard-step cc-onboard-welcome">
            <div className="cc-onboard-logo">
              <DockLogo size={88} animated />
            </div>
            <h1 className="cc-onboard-hero-title">{t("onboarding.step1Title")}</h1>
            <p className="cc-onboard-hero-sub">{t("onboarding.step1Body")}</p>
            <div className="cc-onboard-welcome-cta">
              <Button variant="primary" onClick={() => setStep(2)}>
                {t("onboarding.step1Cta")}
              </Button>
            </div>
          </div>
        )}

        {step === 2 && (
          <div className="cc-onboard-step">
            <div className="cc-onboard-head">
              <h2>{t("onboarding.step2Title")}</h2>
              <p>{t("onboarding.step2Body")}</p>
            </div>

            <div className="cc-theme-cards">
              {THEME_ORDER.map((card) => {
                const active = selectedTheme === card.id;
                const isRec = recommendedTheme === card.id;
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
                      {isRec && (
                        <span className="cc-theme-rec-badge">{t("theme.recommended")}</span>
                      )}
                      <div className="cc-theme-plate-stack">
                        <WidgetPreview
                          id={`onb-np-${card.id}`}
                          kind="nowPlaying"
                          size="pill"
                          themeOverride={card.id}
                          appearance={{
                            ...config.appearance,
                            themeId: card.id,
                            accent:
                              card.id === "laerk" &&
                              config.appearance.accent?.toUpperCase() === "#FFFFFF"
                                ? null
                                : config.appearance.accent,
                          }}
                          monitors={monitors}
                          locations={config.weather.locations}
                          maxWidth={148}
                          maxHeight={36}
                        />
                        <WidgetPreview
                          id={`onb-wx-${card.id}`}
                          kind="weather"
                          size="s"
                          themeOverride={card.id}
                          appearance={{
                            ...config.appearance,
                            themeId: card.id,
                            accent:
                              card.id === "laerk" &&
                              config.appearance.accent?.toUpperCase() === "#FFFFFF"
                                ? null
                                : config.appearance.accent,
                          }}
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

            <div className="cc-onboard-actions">
              <Button variant="secondary" onClick={() => setStep(1)}>
                {t("onboarding.back")}
              </Button>
              <Button variant="primary" onClick={handleAdvanceFromStep2}>
                {t("onboarding.next")}
              </Button>
            </div>
          </div>
        )}

        {step === 3 && (
          <div className="cc-onboard-step">
            <div className="cc-onboard-head">
              <h2>{t("onboarding.step3Title")}</h2>
              <p>{t("onboarding.step3Body")}</p>
            </div>

            <div className="cc-starter-grid">
              {/* 1. Alt-i-én Boks */}
              <button
                type="button"
                className={`cc-starter-card${selectedStarter === "allInOne" ? " is-selected" : ""}`}
                onClick={() => setSelectedStarter("allInOne")}
              >
                <div
                  className="cc-starter-screen"
                  style={plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined}
                >
                  <div className="cc-stage-scrim" aria-hidden="true" />
                  <span className="cc-starter-badge">{t("onboarding.badgeCompact")}</span>
                  <div className="cc-starter-preview-row">
                    {allInOneBoxes[0] && (
                      <WidgetPreview
                        id="onb-aio"
                        kind="cluster"
                        size="m"
                        cluster={allInOneBoxes[0]}
                        appearance={previewAppearance}
                        monitors={monitors}
                        locations={config.weather.locations}
                        maxWidth={138}
                        maxHeight={108}
                      />
                    )}
                  </div>
                  {selectedStarter === "allInOne" && (
                    <span className="cc-theme-check" aria-hidden="true">
                      <Check size={13} weight="bold" />
                    </span>
                  )}
                </div>
                <div className="cc-starter-meta">
                  <span className="cc-starter-title">{t("onboarding.sets.allInOneTitle")}</span>
                  <span className="cc-starter-desc">{t("onboarding.sets.allInOneDesc")}</span>
                </div>
              </button>

              {/* 2. Mini Trio */}
              <button
                type="button"
                className={`cc-starter-card${selectedStarter === "miniTrio" ? " is-selected" : ""}`}
                onClick={() => setSelectedStarter("miniTrio")}
              >
                <div
                  className="cc-starter-screen"
                  style={plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined}
                >
                  <div className="cc-stage-scrim" aria-hidden="true" />
                  <div className="cc-starter-trio-col">
                    {miniTrioBoxes.map((box, idx) => (
                      <WidgetPreview
                        key={`onb-trio-${idx}`}
                        id={`onb-trio-${idx}`}
                        kind="cluster"
                        size="m"
                        cluster={box}
                        appearance={previewAppearance}
                        monitors={monitors}
                        locations={config.weather.locations}
                        maxWidth={132}
                        maxHeight={34}
                      />
                    ))}
                  </div>
                  {selectedStarter === "miniTrio" && (
                    <span className="cc-theme-check" aria-hidden="true">
                      <Check size={13} weight="bold" />
                    </span>
                  )}
                </div>
                <div className="cc-starter-meta">
                  <span className="cc-starter-title">{t("onboarding.sets.miniTrioTitle")}</span>
                  <span className="cc-starter-desc">{t("onboarding.sets.miniTrioDesc")}</span>
                </div>
              </button>

              {/* 3. Musik og ro */}
              <button
                type="button"
                className={`cc-starter-card${selectedStarter === "musicCalm" ? " is-selected" : ""}`}
                onClick={() => setSelectedStarter("musicCalm")}
              >
                <div
                  className="cc-starter-screen"
                  style={plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined}
                >
                  <div className="cc-stage-scrim" aria-hidden="true" />
                  <span className="cc-starter-badge is-subtle">
                    {t("onboarding.badgeFreePills")}
                  </span>
                  <div className="cc-starter-free-col">
                    <WidgetPreview
                      id="onb-mc-np"
                      kind="nowPlaying"
                      size="pill"
                      appearance={previewAppearance}
                      monitors={monitors}
                      locations={config.weather.locations}
                      maxWidth={138}
                      maxHeight={32}
                    />
                    <div className="cc-starter-free-row">
                      <WidgetPreview
                        id="onb-mc-clk"
                        kind="clock"
                        size="s"
                        appearance={previewAppearance}
                        monitors={monitors}
                        locations={config.weather.locations}
                        maxWidth={68}
                        maxHeight={68}
                      />
                      <WidgetPreview
                        id="onb-mc-wx"
                        kind="weather"
                        size="s"
                        appearance={previewAppearance}
                        monitors={monitors}
                        locations={config.weather.locations}
                        maxWidth={68}
                        maxHeight={68}
                      />
                    </div>
                  </div>
                  {selectedStarter === "musicCalm" && (
                    <span className="cc-theme-check" aria-hidden="true">
                      <Check size={13} weight="bold" />
                    </span>
                  )}
                </div>
                <div className="cc-starter-meta">
                  <span className="cc-starter-title">{t("onboarding.sets.musicCalmTitle")}</span>
                  <span className="cc-starter-desc">{t("onboarding.sets.musicCalmDesc")}</span>
                </div>
              </button>

              {/* 4. Gamer */}
              <button
                type="button"
                className={`cc-starter-card${selectedStarter === "gamer" ? " is-selected" : ""}`}
                onClick={() => setSelectedStarter("gamer")}
              >
                <div
                  className="cc-starter-screen"
                  style={plateUrl ? { backgroundImage: `url("${plateUrl}")` } : undefined}
                >
                  <div className="cc-stage-scrim" aria-hidden="true" />
                  <span className="cc-starter-badge is-subtle">
                    {t("onboarding.badgeFreePills")}
                  </span>
                  <div className="cc-starter-free-col">
                    <WidgetPreview
                      id="onb-gm-sys"
                      kind="system"
                      size="pill"
                      appearance={previewAppearance}
                      monitors={monitors}
                      locations={config.weather.locations}
                      maxWidth={142}
                      maxHeight={32}
                    />
                    <WidgetPreview
                      id="onb-gm-np"
                      kind="nowPlaying"
                      size="pill"
                      appearance={previewAppearance}
                      monitors={monitors}
                      locations={config.weather.locations}
                      maxWidth={142}
                      maxHeight={32}
                    />
                    <WidgetPreview
                      id="onb-gm-clk"
                      kind="clock"
                      size="pill"
                      appearance={previewAppearance}
                      monitors={monitors}
                      locations={config.weather.locations}
                      maxWidth={142}
                      maxHeight={32}
                    />
                  </div>
                  {selectedStarter === "gamer" && (
                    <span className="cc-theme-check" aria-hidden="true">
                      <Check size={13} weight="bold" />
                    </span>
                  )}
                </div>
                <div className="cc-starter-meta">
                  <span className="cc-starter-title">{t("onboarding.sets.gamerTitle")}</span>
                  <span className="cc-starter-desc">{t("onboarding.sets.gamerDesc")}</span>
                </div>
              </button>
            </div>

            <div className="cc-onboard-actions">
              <Button variant="secondary" disabled={busy} onClick={() => setStep(2)}>
                {t("onboarding.back")}
              </Button>
              <div className="cc-onboard-actions-right">
                <button
                  type="button"
                  className="cc-link-btn"
                  disabled={busy}
                  onClick={() => void handleStartEmpty()}
                >
                  {t("onboarding.startEmpty")}
                </button>
                <Button
                  variant="primary"
                  disabled={busy}
                  onClick={() => void handlePlaceStarter()}
                >
                  {busy ? t("onboarding.placing") : t("onboarding.placeCta")}
                </Button>
              </div>
            </div>
          </div>
        )}
      </div>

      <div className="cc-onboard-dots" aria-hidden="true">
        {[1, 2, 3].map((idx) => (
          <span key={idx} className={`cc-onboard-dot${step === idx ? " is-active" : ""}`} />
        ))}
      </div>

      <Drawer
        open={whereDrawerOpen}
        title={t("onboarding.whereTitle")}
        onClose={handleSkipWhere}
      >
        <p className="cc-field-help">{t("onboarding.whereBody")}</p>

        <Input
          autoFocus
          type="text"
          value={cityQuery}
          placeholder={t("weather.searchPh")}
          onChange={(e) => handleCityQueryChange(e.target.value)}
        />

        {cityQuery.trim().length < 2 ? (
          <span className="cc-field-help">{t("form.searchPlacesHint")}</span>
        ) : citySearching ? (
          <span className="cc-field-help">{t("form.searching")}</span>
        ) : cityResults.length === 0 ? (
          <span className="cc-field-help">{t("form.noSearchResults")}</span>
        ) : (
          <div className="cc-sort-list">
            {cityResults.map((geo) => {
              const sub = [geo.admin1, geo.country].filter(Boolean).join(", ");
              return (
                <div key={`${geo.name}-${geo.latitude}-${geo.longitude}`} className="cc-place-row">
                  <button
                    type="button"
                    className="cc-place-btn"
                    disabled={busy}
                    onClick={() => void handlePickCity(geo)}
                  >
                    <span className="cc-place-name">
                      {geo.name}
                      {sub ? ` · ${sub}` : ""}
                    </span>
                  </button>
                  <Plus size={15} color="var(--c-accent)" aria-hidden="true" />
                </div>
              );
            })}
          </div>
        )}

        <div style={{ marginTop: "auto", paddingTop: 12, display: "flex", justifyContent: "flex-end" }}>
          <Button variant="secondary" disabled={busy} onClick={handleSkipWhere}>
            {t("onboarding.whereSkip")}
          </Button>
        </div>
      </Drawer>
    </div>
  );
}
