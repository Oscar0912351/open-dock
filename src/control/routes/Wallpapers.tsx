import {
  Check,
  FilmStrip,
  Image as ImageIcon,
  Sparkle,
  SpinnerGap,
  Stop,
  Trash,
  UploadSimple,
  WarningCircle,
} from "@phosphor-icons/react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MonitorWallpaper } from "../../shared/bindings/MonitorWallpaper";
import type { WallpaperEngineStatus } from "../../shared/bindings/WallpaperEngineStatus";
import type { WallpaperFit } from "../../shared/bindings/WallpaperFit";
import type { WallpaperItem } from "../../shared/bindings/WallpaperItem";
import type { WallpaperMood } from "../../shared/bindings/WallpaperMood";
import type { WallpaperResolution } from "../../shared/bindings/WallpaperResolution";
import type { WindowsQualityStatus } from "../../shared/bindings/WindowsQualityStatus";
import { t } from "../../shared/i18n";
import { ipc, on, toAssetUrl } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Card, SettingRow } from "../components/Card";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Slider } from "../components/Slider";
import { Switch } from "../components/Switch";
import { useControlStore } from "../store";

type KindFilter = "all" | "shader" | "photo" | "custom";
type MoodFilter = "all" | WallpaperMood;

const MOOD_OPTIONS: ReadonlyArray<{ value: MoodFilter; labelKey: string }> = [
  { value: "all", labelKey: "wallpapers.moods.all" },
  { value: "dark", labelKey: "wallpapers.moods.dark" },
  { value: "nature", labelKey: "wallpapers.moods.nature" },
  { value: "minimal", labelKey: "wallpapers.moods.minimal" },
  { value: "warm", labelKey: "wallpapers.moods.warm" },
  { value: "architecture", labelKey: "wallpapers.moods.architecture" },
  { value: "light", labelKey: "wallpapers.moods.light" },
];

const FPS_OPTIONS = [
  { value: "24", label: "24 FPS" },
  { value: "30", label: "30 FPS" },
  { value: "60", label: "60 FPS" },
] as const;

export function Wallpapers() {
  const locale = useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const pushToast = useControlStore((s) => s.pushToast);

  const [catalog, setCatalog] = useState<WallpaperItem[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [engineStatus, setEngineStatus] = useState<WallpaperEngineStatus>({
    runState: "idle",
    activeSurfaces: 0,
    actualFps: 0,
    frameTimeMs: 0,
    backend: "",
    adapterName: "",
    renderWidth: 0,
    renderHeight: 0,
    lastError: null,
  });
  const [winQuality, setWinQuality] = useState<WindowsQualityStatus | null>(null);
  const [targetMonitor, setTargetMonitor] = useState<string>("*");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [moodFilter, setMoodFilter] = useState<MoodFilter>("all");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);

  const [speedDraft, setSpeedDraft] = useState<{ base: number; pct: number } | null>(null);
  const [brightnessDraft, setBrightnessDraft] = useState<{ base: number; pct: number } | null>(
    null,
  );
  const speedTimer = useRef<number | null>(null);
  const brightnessTimer = useRef<number | null>(null);

  const refreshCatalog = useCallback(async () => {
    try {
      const list = await ipc.wallpaperCatalog();
      setCatalog(list);
      setErrorMsg(null);
    } catch (err) {
      setErrorMsg(String(err));
    } finally {
      setCatalogLoading(false);
    }
  }, []);

  useEffect(() => {
    let active = true;
    ipc
      .wallpaperCatalog()
      .then((list) => {
        if (!active) return;
        setCatalog(list);
        setErrorMsg(null);
        setCatalogLoading(false);
      })
      .catch((err) => {
        if (!active) return;
        setErrorMsg(String(err));
        setCatalogLoading(false);
      });
    ipc
      .wallpaperEngineStatus()
      .then((st) => {
        if (active) setEngineStatus(st);
      })
      .catch(() => {});
    ipc
      .wallpaperWindowsQualityGet()
      .then((wq) => {
        if (active) setWinQuality(wq);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [locale]);

  useEffect(() => {
    let active = true;
    const unlisteners: Array<() => void> = [];

    on.wallpaperStatus((status) => {
      if (active) setEngineStatus(status);
    })
      .then((fn) => {
        if (active) unlisteners.push(fn);
        else fn();
      })
      .catch(() => {});

    on.wallpaperThumbsReady(() => {
      if (active) void refreshCatalog();
    })
      .then((fn) => {
        if (active) unlisteners.push(fn);
        else fn();
      })
      .catch(() => {});

    return () => {
      active = false;
      for (const dispose of unlisteners) dispose();
      if (speedTimer.current !== null) window.clearTimeout(speedTimer.current);
      if (brightnessTimer.current !== null) window.clearTimeout(brightnessTimer.current);
    };
  }, [refreshCatalog]);

  const effectiveTarget = useMemo(() => {
    if (targetMonitor === "*") return "*";
    return monitors.some((m) => m.name === targetMonitor) ? targetMonitor : "*";
  }, [targetMonitor, monitors]);

  const primaryMonitor = useMemo(
    () => monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null,
    [monitors],
  );

  const selectedMonitorInfo = useMemo(() => {
    if (effectiveTarget === "*") return primaryMonitor;
    return monitors.find((m) => m.name === effectiveTarget) ?? primaryMonitor;
  }, [effectiveTarget, monitors, primaryMonitor]);

  const activeAssignment: MonitorWallpaper | null = useMemo(() => {
    if (!config) return null;
    const assignments = config.wallpaper.assignments;
    if (effectiveTarget !== "*" && assignments[effectiveTarget]) {
      return assignments[effectiveTarget] ?? null;
    }
    if (assignments["*"]) return assignments["*"] ?? null;
    if (selectedMonitorInfo && assignments[selectedMonitorInfo.name]) {
      return assignments[selectedMonitorInfo.name] ?? null;
    }
    const firstKey = Object.keys(assignments)[0];
    return firstKey ? (assignments[firstKey] ?? null) : null;
  }, [config, effectiveTarget, selectedMonitorInfo]);

  const activeItem = useMemo(() => {
    if (!activeAssignment) return null;
    return catalog.find((item) => item.id === activeAssignment.wallpaperId) ?? null;
  }, [catalog, activeAssignment]);

  const filteredItems = useMemo(() => {
    return catalog.filter((item) => {
      if (kindFilter === "custom" && !item.isCustom) return false;
      if (kindFilter === "shader" && item.kind !== "shader") return false;
      if (kindFilter === "photo" && (item.kind !== "photo" || item.isCustom)) return false;
      if (moodFilter !== "all" && !item.moods.includes(moodFilter)) return false;
      return true;
    });
  }, [catalog, kindFilter, moodFilter]);

  if (!config) return null;

  const { engine } = config.wallpaper;

  const numFmt1 = new Intl.NumberFormat(locale === "da" ? "da-DK" : "en-US", {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  });
  const numFmt2 = new Intl.NumberFormat(locale === "da" ? "da-DK" : "en-US", {
    minimumFractionDigits: 1,
    maximumFractionDigits: 2,
  });
  const numFmtInt = new Intl.NumberFormat(locale === "da" ? "da-DK" : "en-US");

  const baseSpeed = activeAssignment?.speed ?? 1;
  const speedPct =
    speedDraft && speedDraft.base === baseSpeed
      ? speedDraft.pct
      : Math.round(baseSpeed * 100);

  const baseBrightness = activeAssignment?.brightness ?? 1;
  const brightnessPct =
    brightnessDraft && brightnessDraft.base === baseBrightness
      ? brightnessDraft.pct
      : Math.round(baseBrightness * 100);

  const handleApply = async (item: WallpaperItem) => {
    if (busyId) return;
    setBusyId(item.id);
    setErrorMsg(null);
    try {
      await ipc.wallpaperApply(item.id, effectiveTarget, null);
      pushToast({ message: t("wallpapers.toast.applied", { title: item.title }) });
      void refreshCatalog();
    } catch (err) {
      const msg = extractErrMessage(err);
      setErrorMsg(msg);
      pushToast({ message: msg });
    } finally {
      setBusyId(null);
    }
  };

  const handleImport = async () => {
    if (importing) return;
    try {
      const selected = await openFileDialog({
        multiple: false,
        directory: false,
        filters: [
          {
            name: "Billeder & Video",
            extensions: ["png", "jpg", "jpeg", "webp", "bmp", "mp4", "webm", "mkv", "mov", "wmv"],
          },
        ],
      });
      if (!selected || typeof selected !== "string") return;
      setImporting(true);
      setErrorMsg(null);
      const imported = await ipc.wallpaperImport(selected);
      await ipc.wallpaperApply(imported.id, effectiveTarget, null);
      await refreshCatalog();
      setKindFilter("custom");
      pushToast({ message: t("wallpapers.toast.imported", { title: imported.title }) });
    } catch (err) {
      const msg = extractErrMessage(err);
      setErrorMsg(msg);
      pushToast({ message: msg });
    } finally {
      setImporting(false);
    }
  };

  const handleDeleteCustom = async (item: WallpaperItem) => {
    try {
      await ipc.wallpaperDeleteCustom(item.id);
      await refreshCatalog();
      pushToast({ message: t("wallpapers.toast.removed") });
    } catch (err) {
      pushToast({ message: extractErrMessage(err) });
    }
  };

  const handleStopLive = async () => {
    try {
      await ipc.wallpaperStopLive(effectiveTarget);
      pushToast({ message: t("wallpapers.toast.stopped") });
    } catch (err) {
      pushToast({ message: extractErrMessage(err) });
    }
  };

  const handleSpeedChange = (pct: number) => {
    setSpeedDraft({ base: baseSpeed, pct });
    if (speedTimer.current !== null) window.clearTimeout(speedTimer.current);
    speedTimer.current = window.setTimeout(() => {
      speedTimer.current = null;
      void ipc.wallpaperUpdateAssignment(effectiveTarget, { speed: pct / 100 });
    }, 60);
  };

  const handleBrightnessChange = (pct: number) => {
    setBrightnessDraft({ base: baseBrightness, pct });
    if (brightnessTimer.current !== null) window.clearTimeout(brightnessTimer.current);
    brightnessTimer.current = window.setTimeout(() => {
      brightnessTimer.current = null;
      void ipc.wallpaperUpdateAssignment(effectiveTarget, { brightness: pct / 100 });
    }, 60);
  };

  const handleToggleWindowsQuality = async (enabled: boolean) => {
    try {
      const next = await ipc.wallpaperWindowsQualitySet(enabled);
      setWinQuality(next);
      pushToast({
        message: enabled ? t("wallpapers.toast.qualityOn") : t("wallpapers.toast.qualityOff"),
      });
    } catch (err) {
      pushToast({ message: extractErrMessage(err) });
    }
  };

  const statusText = formatEngineStatus(engineStatus, numFmt1, numFmt2);
  const heroThumbUrl = activeItem?.thumbPath
    ? toAssetUrl(activeItem.thumbPath)
    : toAssetUrl(selectedMonitorInfo?.platePath ?? "");
  const isActiveLive = activeItem?.kind === "shader" || activeItem?.kind === "video";

  const monitorOptions = [
    { value: "*", label: t("wallpapers.allMonitors") },
    ...monitors.map((m, idx) => ({
      value: m.name,
      label: m.isPrimary
        ? t("gallery.monitorPrimary", { n: idx + 1 })
        : t("gallery.monitorNth", { n: idx + 1 }),
    })),
  ];

  const telemetryBackend =
    engineStatus.activeSurfaces > 0 && engineStatus.backend
      ? engineStatus.backend
      : t("wallpapers.telemetry.staticMode");
  const telemetryPerf =
    engineStatus.runState === "running"
      ? `${numFmt1.format(engineStatus.actualFps)} FPS · ${numFmt2.format(engineStatus.frameTimeMs)} ms`
      : t("wallpapers.telemetry.zeroGpu");
  const telemetryRes =
    engineStatus.renderWidth > 0 && engineStatus.renderHeight > 0
      ? `${numFmtInt.format(engineStatus.renderWidth)}×${numFmtInt.format(engineStatus.renderHeight)}`
      : selectedMonitorInfo
        ? `${numFmtInt.format(selectedMonitorInfo.rect.width)}×${numFmtInt.format(selectedMonitorInfo.rect.height)}`
        : t("state.dash");

  const activeError = errorMsg ?? engineStatus.lastError;

  return (
    <div className="cc-route">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("wallpapers.title")}</h1>
          <p className="cc-page-subtitle">{t("wallpapers.subtitle")}</p>
        </div>
        <Button variant="primary" disabled={importing} onClick={() => void handleImport()}>
          {importing ? (
            <SpinnerGap size={14} className="cc-wp-spin" aria-hidden="true" />
          ) : (
            <UploadSimple size={14} aria-hidden="true" />
          )}
          <span>{importing ? t("wallpapers.importing") : t("wallpapers.importBtn")}</span>
        </Button>
      </div>

      {activeError && (
        <div className="cc-wp-error-banner" role="alert">
          <WarningCircle size={15} aria-hidden="true" />
          <span>{activeError}</span>
        </div>
      )}

      {/* Active Wallpaper Hero */}
      <div className="cc-wp-hero">
        <div className="cc-wp-hero-stage">
          {heroThumbUrl && (
            <div
              className="cc-wp-hero-img"
              aria-hidden="true"
              style={{ backgroundImage: `url("${heroThumbUrl}")` }}
            />
          )}
          <div className="cc-stage-scrim" aria-hidden="true" />
          <div
            className={`cc-wp-status-pill${engineStatus.runState === "running" ? " is-live" : ""}`}
          >
            <span className="cc-wp-status-dot" aria-hidden="true" />
            <span>{statusText}</span>
          </div>
          <div className="cc-wp-res-pill">
            <span>{activeItem ? activeItem.resolutionLabel : telemetryRes}</span>
            {engineStatus.adapterName && engineStatus.activeSurfaces > 0 && (
              <>
                <span>·</span>
                <span>{engineStatus.adapterName}</span>
              </>
            )}
          </div>
        </div>

        <div className="cc-wp-hero-controls">
          <div className="cc-wp-hero-head">
            <div className="cc-wp-hero-title-group">
              <span className="cc-section-label">{t("wallpapers.controls.activeTitle")}</span>
              <h2 className="cc-wp-hero-title">
                {activeItem ? activeItem.title : t("wallpapers.controls.windowsCurrent")}
              </h2>
              <p className="cc-wp-hero-sub">
                {activeItem ? activeItem.subtitle : t("wallpapers.controls.windowsCurrentDesc")}
              </p>
            </div>
            {monitors.length > 1 && (
              <div style={{ width: 170 }}>
                <Select
                  value={effectiveTarget}
                  ariaLabel={t("wallpapers.monitorTarget")}
                  options={monitorOptions}
                  onChange={setTargetMonitor}
                />
              </div>
            )}
          </div>

          {activeItem && activeAssignment && (
            <div className="cc-wp-inline-params">
              {activeItem.kind === "shader" && (
                <>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.speed")}</span>
                    <div style={{ width: 196 }}>
                      <Slider
                        value={speedPct}
                        min={25}
                        max={200}
                        step={5}
                        ariaLabel={t("wallpapers.controls.speed")}
                        formatValue={(v) => `${v} %`}
                        onChange={handleSpeedChange}
                      />
                    </div>
                  </div>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.brightness")}</span>
                    <div style={{ width: 196 }}>
                      <Slider
                        value={brightnessPct}
                        min={60}
                        max={120}
                        step={5}
                        ariaLabel={t("wallpapers.controls.brightness")}
                        formatValue={(v) => `${v} %`}
                        onChange={handleBrightnessChange}
                      />
                    </div>
                  </div>
                </>
              )}

              {activeItem.kind === "video" && (
                <>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.fit")}</span>
                    <Segmented<WallpaperFit>
                      value={activeAssignment.fit}
                      ariaLabel={t("wallpapers.controls.fit")}
                      options={[
                        { value: "fill", label: t("wallpapers.controls.fitFill") },
                        { value: "fit", label: t("wallpapers.controls.fitFit") },
                        { value: "stretch", label: t("wallpapers.controls.fitStretch") },
                      ]}
                      onChange={(fit) =>
                        void ipc.wallpaperUpdateAssignment(effectiveTarget, { fit })
                      }
                    />
                  </div>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.speed")}</span>
                    <div style={{ width: 196 }}>
                      <Slider
                        value={speedPct}
                        min={25}
                        max={200}
                        step={5}
                        ariaLabel={t("wallpapers.controls.speed")}
                        formatValue={(v) => `${v} %`}
                        onChange={handleSpeedChange}
                      />
                    </div>
                  </div>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.muted")}</span>
                    <Switch
                      checked={activeAssignment.muted}
                      ariaLabel={t("wallpapers.controls.muted")}
                      onChange={(muted) =>
                        void ipc.wallpaperUpdateAssignment(effectiveTarget, { muted })
                      }
                    />
                  </div>
                </>
              )}

              {activeItem.kind === "photo" && (
                <div className="cc-wp-param-row">
                  <span className="cc-field-label">{t("wallpapers.controls.fit")}</span>
                  <Segmented<WallpaperFit>
                    value={activeAssignment.fit}
                    ariaLabel={t("wallpapers.controls.fit")}
                    options={[
                      { value: "fill", label: t("wallpapers.controls.fitFill") },
                      { value: "fit", label: t("wallpapers.controls.fitFit") },
                      { value: "center", label: t("wallpapers.controls.fitCenter") },
                      { value: "span", label: t("wallpapers.controls.fitSpan") },
                    ]}
                    onChange={(fit) =>
                      void ipc.wallpaperUpdateAssignment(effectiveTarget, { fit })
                    }
                  />
                </div>
              )}
            </div>
          )}

          <div className="cc-wp-hero-foot">
            <span className="cc-field-help">{t("wallpapers.applyHint")}</span>
            {isActiveLive && (
              <Button variant="secondary" size="sm" onClick={() => void handleStopLive()}>
                <Stop size={12} weight="fill" aria-hidden="true" />
                <span>{t("wallpapers.controls.stopLive")}</span>
              </Button>
            )}
          </div>
        </div>
      </div>

      {/* Filter Bar */}
      <div className="cc-wp-toolbar">
        <Segmented<KindFilter>
          value={kindFilter}
          ariaLabel={t("wallpapers.title")}
          options={[
            { value: "all", label: t("wallpapers.filters.all") },
            { value: "shader", label: t("wallpapers.filters.shader") },
            { value: "photo", label: t("wallpapers.filters.photo") },
            { value: "custom", label: t("wallpapers.filters.custom") },
          ]}
          onChange={setKindFilter}
        />

        <div className="cc-wp-moods" role="group" aria-label={t("wallpapers.moods.all")}>
          {MOOD_OPTIONS.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={`cc-chip${moodFilter === opt.value ? " is-active" : ""}`}
              onClick={() => setMoodFilter(opt.value)}
            >
              {t(opt.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* Wallpaper Grid */}
      {catalogLoading ? (
        <div className="cc-wp-grid" aria-busy="true">
          {Array.from({ length: 6 }).map((_, idx) => (
            <div key={idx} className="cc-wp-card is-skeleton">
              <div className="cc-wp-card-stage cc-wp-shimmer" />
              <div className="cc-wp-card-meta">
                <span className="cc-wp-skel-line" />
              </div>
            </div>
          ))}
        </div>
      ) : filteredItems.length === 0 ? (
        <div className="cc-empty">
          <div className="cc-empty-illus" aria-hidden="true">
            <div className="cc-empty-box" />
            <div className="cc-empty-box is-tall" />
          </div>
          <h3>
            {kindFilter === "custom"
              ? t("wallpapers.empty.customTitle")
              : t("wallpapers.empty.filterTitle")}
          </h3>
          <p>
            {kindFilter === "custom"
              ? t("wallpapers.empty.customBody")
              : t("wallpapers.empty.filterBody")}
          </p>
          {kindFilter === "custom" ? (
            <Button variant="primary" onClick={() => void handleImport()}>
              <UploadSimple size={14} aria-hidden="true" />
              <span>{t("wallpapers.importBtn")}</span>
            </Button>
          ) : (
            <Button
              variant="secondary"
              onClick={() => {
                setKindFilter("all");
                setMoodFilter("all");
              }}
            >
              {t("wallpapers.empty.resetFilters")}
            </Button>
          )}
        </div>
      ) : (
        <div className="cc-wp-grid">
          {filteredItems.map((item) => {
            const isSelected = activeAssignment?.wallpaperId === item.id;
            const isBusy = busyId === item.id;
            const thumbUrl = toAssetUrl(item.thumbPath);

            return (
              <article
                key={item.id}
                className={`cc-wp-card${isSelected ? " is-selected" : ""}${isBusy ? " is-busy" : ""}`}
              >
                <button
                  type="button"
                  className="cc-wp-card-btn"
                  disabled={busyId !== null}
                  aria-pressed={isSelected}
                  onClick={() => void handleApply(item)}
                >
                  <div className="cc-wp-card-stage">
                    {thumbUrl ? (
                      <div
                        className="cc-wp-card-img"
                        style={{ backgroundImage: `url("${thumbUrl}")` }}
                      />
                    ) : (
                      <div className="cc-wp-card-img cc-wp-shimmer" />
                    )}
                    <div className="cc-wp-card-scrim" aria-hidden="true" />

                    <span className={`cc-wp-kind-badge is-${item.kind}`}>
                      {item.kind === "shader" ? (
                        <Sparkle size={11} weight="fill" aria-hidden="true" />
                      ) : item.kind === "video" ? (
                        <FilmStrip size={11} weight="fill" aria-hidden="true" />
                      ) : (
                        <ImageIcon size={11} aria-hidden="true" />
                      )}
                      <span>
                        {item.kind === "shader"
                          ? t("wallpapers.badges.live")
                          : item.kind === "video"
                            ? t("wallpapers.badges.video")
                            : item.isCustom
                              ? t("wallpapers.badges.custom")
                              : t("wallpapers.badges.photo4k")}
                      </span>
                    </span>

                    {isSelected && !isBusy && (
                      <span className="cc-theme-check" aria-hidden="true">
                        <Check size={13} weight="bold" />
                      </span>
                    )}

                    {isBusy && (
                      <div className="cc-wp-busy-overlay">
                        <SpinnerGap size={18} className="cc-wp-spin" aria-hidden="true" />
                        <span>
                          {item.kind === "photo" && !item.isCached
                            ? t("wallpapers.badges.downloading")
                            : t("wallpapers.badges.applying")}
                        </span>
                      </div>
                    )}
                  </div>

                  <div className="cc-wp-card-meta">
                    <div className="cc-wp-card-title-row">
                      <span className="cc-wp-card-title">{item.title}</span>
                      <span className="cc-wp-card-res">{item.resolutionLabel}</span>
                    </div>
                    <span className="cc-wp-card-sub" title={item.subtitle}>
                      {item.subtitle}
                    </span>
                  </div>
                </button>

                {item.isCustom && (
                  <button
                    type="button"
                    className="cc-wp-delete-btn"
                    aria-label={t("wallpapers.deleteCustom")}
                    title={t("wallpapers.deleteCustom")}
                    onClick={(e) => {
                      e.stopPropagation();
                      void handleDeleteCustom(item);
                    }}
                  >
                    <Trash size={13} aria-hidden="true" />
                  </button>
                )}
              </article>
            );
          })}
        </div>
      )}

      {/* Engine & Windows Quality Settings */}
      <div className="cc-themes-controls-grid">
        <section className="cc-section">
          <span className="cc-section-label">{t("wallpapers.sections.engine")}</span>
          <Card compact>
            <SettingRow
              title={t("wallpapers.settings.fpsCap")}
              help={t("wallpapers.settings.fpsCapHelp")}
            >
              <Segmented<string>
                value={String(engine.fpsCap)}
                ariaLabel={t("wallpapers.settings.fpsCap")}
                options={FPS_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
                onChange={(val) => void ipc.wallpaperEnginePatch({ fpsCap: Number(val) })}
              />
            </SettingRow>

            <SettingRow
              title={t("wallpapers.settings.resolution")}
              help={t("wallpapers.settings.resolutionHelp")}
            >
              <Segmented<WallpaperResolution>
                value={engine.resolution}
                ariaLabel={t("wallpapers.settings.resolution")}
                options={[
                  { value: "native", label: t("wallpapers.settings.resolutionNative") },
                  { value: "balanced", label: t("wallpapers.settings.resolutionBalanced") },
                ]}
                onChange={(resolution) => void ipc.wallpaperEnginePatch({ resolution })}
              />
            </SettingRow>

            <SettingRow
              title={t("wallpapers.settings.pauseCovered")}
              help={t("wallpapers.settings.pauseCoveredHelp")}
            >
              <Switch
                checked={engine.pauseWhenCovered}
                ariaLabel={t("wallpapers.settings.pauseCovered")}
                onChange={(pauseWhenCovered) =>
                  void ipc.wallpaperEnginePatch({ pauseWhenCovered })
                }
              />
            </SettingRow>

            <SettingRow
              title={t("wallpapers.settings.pauseFullscreen")}
              help={t("wallpapers.settings.pauseFullscreenHelp")}
            >
              <Switch
                checked={engine.pauseOnFullscreen}
                ariaLabel={t("wallpapers.settings.pauseFullscreen")}
                onChange={(pauseOnFullscreen) =>
                  void ipc.wallpaperEnginePatch({ pauseOnFullscreen })
                }
              />
            </SettingRow>

            <SettingRow
              title={t("wallpapers.settings.pauseBattery")}
              help={t("wallpapers.settings.pauseBatteryHelp")}
            >
              <Switch
                checked={engine.pauseOnBattery}
                ariaLabel={t("wallpapers.settings.pauseBattery")}
                onChange={(pauseOnBattery) => void ipc.wallpaperEnginePatch({ pauseOnBattery })}
              />
            </SettingRow>
          </Card>
        </section>

        <section className="cc-section">
          <span className="cc-section-label">{t("wallpapers.sections.quality")}</span>
          <Card compact>
            <SettingRow
              title={t("wallpapers.settings.windowsQuality")}
              help={t("wallpapers.settings.windowsQualityHelp")}
            >
              <div className="cc-wp-quality-ctrl">
                {winQuality && (
                  <span
                    className={`cc-wp-quality-badge${winQuality.losslessEnabled ? " is-on" : ""}`}
                  >
                    {winQuality.losslessEnabled
                      ? t("wallpapers.settings.qualityLosslessBadge")
                      : t("wallpapers.settings.qualityStandardBadge")}
                  </span>
                )}
                <Switch
                  checked={winQuality?.losslessEnabled ?? false}
                  disabled={!winQuality}
                  ariaLabel={t("wallpapers.settings.windowsQuality")}
                  onChange={(enabled) => void handleToggleWindowsQuality(enabled)}
                />
              </div>
            </SettingRow>

            <SettingRow
              title={t("wallpapers.settings.syncGlass")}
              help={t("wallpapers.settings.syncGlassHelp")}
            >
              <Switch
                checked={engine.syncGlassPlates}
                ariaLabel={t("wallpapers.settings.syncGlass")}
                onChange={(syncGlassPlates) => void ipc.wallpaperEnginePatch({ syncGlassPlates })}
              />
            </SettingRow>

            <div className="cc-wp-telemetry-strip">
              <div className="cc-wp-telemetry-cell">
                <span className="cc-wp-telemetry-label">{t("wallpapers.telemetry.backend")}</span>
                <span className="cc-wp-telemetry-val" title={telemetryBackend}>
                  {telemetryBackend}
                </span>
              </div>
              <div className="cc-wp-telemetry-cell">
                <span className="cc-wp-telemetry-label">{t("wallpapers.telemetry.frameTime")}</span>
                <span className="cc-wp-telemetry-val">{telemetryPerf}</span>
              </div>
              <div className="cc-wp-telemetry-cell">
                <span className="cc-wp-telemetry-label">
                  {t("wallpapers.telemetry.resolution")}
                </span>
                <span className="cc-wp-telemetry-val">{telemetryRes}</span>
              </div>
            </div>
          </Card>
        </section>
      </div>
    </div>
  );
}

function formatEngineStatus(
  status: WallpaperEngineStatus,
  numFmt1: Intl.NumberFormat,
  numFmt2: Intl.NumberFormat,
): string {
  switch (status.runState) {
    case "running":
      return t("wallpapers.status.running", {
        fps: numFmt1.format(status.actualFps),
        ms: numFmt2.format(status.frameTimeMs),
      });
    case "pausedCovered":
      return t("wallpapers.status.pausedCovered");
    case "pausedFullscreen":
      return t("wallpapers.status.pausedFullscreen");
    case "pausedBattery":
      return t("wallpapers.status.pausedBattery");
    case "pausedLocked":
      return t("wallpapers.status.pausedLocked");
    case "pausedManual":
      return t("wallpapers.status.pausedManual");
    case "error":
      return t("wallpapers.status.error");
    case "idle":
    default:
      return t("wallpapers.status.idle");
  }
}

function extractErrMessage(err: unknown): string {
  if (typeof err === "object" && err !== null && "message" in err) {
    return String((err as { message: unknown }).message);
  }
  return String(err);
}
