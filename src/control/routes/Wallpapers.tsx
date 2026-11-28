import {
  Check,
  GearSix,
  Play,
  Plus,
  SpinnerGap,
  Stop,
  Trash,
  UploadSimple,
  WarningCircle,
  X,
} from "@phosphor-icons/react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MonitorWallpaper } from "../../shared/bindings/MonitorWallpaper";
import type { WallpaperEngineStatus } from "../../shared/bindings/WallpaperEngineStatus";
import type { WallpaperFit } from "../../shared/bindings/WallpaperFit";
import type { WallpaperItem } from "../../shared/bindings/WallpaperItem";
import type { WallpaperResolution } from "../../shared/bindings/WallpaperResolution";
import type { WindowsQualityStatus } from "../../shared/bindings/WindowsQualityStatus";
import { t } from "../../shared/i18n";
import { ipc, on, subscribeAll, toAssetUrl } from "../../shared/ipc";
import { errorMessage, reportFailure } from "../../shared/logging";
import { Button } from "../components/Button";
import { SettingRow } from "../components/Card";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Slider } from "../components/Slider";
import { Switch } from "../components/Switch";
import { useControlStore } from "../store";

type KindFilter = "all" | "shader" | "photo" | "custom";

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
  const [busyId, setBusyId] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [engineMenuOpen, setEngineMenuOpen] = useState(false);
  const engineMenuRef = useRef<HTMLDivElement>(null);

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
      setErrorMsg(errorMessage(err));
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
        setErrorMsg(errorMessage(err));
        setCatalogLoading(false);
      });
    ipc
      .wallpaperEngineStatus()
      .then((st) => {
        if (active) setEngineStatus(st);
      })
      .catch((reason: unknown) => reportFailure("wallpaper_engine_status", reason));
    ipc
      .wallpaperWindowsQualityGet()
      .then((wq) => {
        if (active) setWinQuality(wq);
      })
      .catch((reason: unknown) => reportFailure("wallpaper_windows_quality_get", reason));
    return () => {
      active = false;
    };
  }, [locale]);

  useEffect(() => {
    let active = true;
    const disposeListeners = subscribeAll([
      [
        "wallpaper://status",
        on.wallpaperStatus((status) => {
          if (active) setEngineStatus(status);
        }),
      ],
      [
        "wallpaper://thumbs-ready",
        on.wallpaperThumbsReady(() => {
          if (active) void refreshCatalog();
        }),
      ],
    ]);

    return () => {
      active = false;
      disposeListeners();
      if (speedTimer.current !== null) window.clearTimeout(speedTimer.current);
      if (brightnessTimer.current !== null) window.clearTimeout(brightnessTimer.current);
    };
  }, [refreshCatalog]);

  useEffect(() => {
    if (!engineMenuOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!engineMenuRef.current?.contains(event.target as Node)) {
        setEngineMenuOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        setEngineMenuOpen(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [engineMenuOpen]);

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
    if (effectiveTarget !== "*") {
      return assignments[effectiveTarget] ?? assignments["*"] ?? null;
    }
    return (
      assignments["*"] ??
      (selectedMonitorInfo ? (assignments[selectedMonitorInfo.name] ?? null) : null)
    );
  }, [config, effectiveTarget, selectedMonitorInfo]);

  const activeItem = useMemo(() => {
    if (!activeAssignment) return null;
    return catalog.find((item) => item.id === activeAssignment.wallpaperId) ?? null;
  }, [catalog, activeAssignment]);

  const filteredItems = useMemo(() => {
    return catalog.filter((item) => {
      if (kindFilter === "custom" && !item.isCustom) return false;
      if (kindFilter === "shader" && item.kind !== "shader" && item.kind !== "video") return false;
      if (kindFilter === "photo" && (item.kind !== "photo" || item.isCustom)) return false;
      return true;
    });
  }, [catalog, kindFilter]);

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
      const msg = errorMessage(err);
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
      const msg = errorMessage(err);
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
      pushToast({ message: errorMessage(err) });
    }
  };

  const handleStopLive = async () => {
    try {
      await ipc.wallpaperStopLive(effectiveTarget);
      pushToast({ message: t("wallpapers.toast.stopped") });
    } catch (err) {
      pushToast({ message: errorMessage(err) });
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
      pushToast({ message: errorMessage(err) });
    }
  };

  const statusText = formatEngineStatus(engineStatus, numFmt1, numFmt2);
  const nowThumbUrl = activeItem?.thumbPath
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

  const activeError = errorMsg ?? engineStatus.lastError;
  const statusTone =
    engineStatus.runState === "running"
      ? "is-live"
      : engineStatus.runState === "error"
        ? "is-error"
        : engineStatus.runState === "idle"
          ? ""
          : "is-paused";

  const showScalingControl =
    activeItem !== null &&
    activeAssignment !== null &&
    (activeItem.kind === "photo" || activeItem.kind === "video");

  const scalingOptions: Array<{ value: WallpaperFit; label: string }> =
    activeItem?.kind === "video"
      ? [
          { value: "fill", label: t("wallpapers.controls.fitFill") },
          { value: "fit", label: t("wallpapers.controls.fitFit") },
          { value: "stretch", label: t("wallpapers.controls.fitStretch") },
        ]
      : [
          { value: "fill", label: t("wallpapers.controls.fitFill") },
          { value: "fit", label: t("wallpapers.controls.fitFit") },
          { value: "center", label: t("wallpapers.controls.fitCenter") },
          ...(monitors.length > 1
            ? [{ value: "span" as const, label: t("wallpapers.controls.fitSpan") }]
            : []),
        ];

  return (
    <div className="cc-route">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("wallpapers.title")}</h1>
          <p className="cc-page-subtitle">{t("wallpapers.subtitle")}</p>
        </div>
        <div className="cc-wp-header-actions">
          <div className="cc-wp-gear-wrap" ref={engineMenuRef}>
            <button
              type="button"
              className={`cc-wp-gear-btn${engineMenuOpen ? " is-open" : ""}`}
              aria-label={t("wallpapers.sections.engine")}
              aria-haspopup="dialog"
              aria-expanded={engineMenuOpen}
              title={t("wallpapers.sections.engine")}
              onClick={() => setEngineMenuOpen((open) => !open)}
            >
              <GearSix size={16} aria-hidden="true" />
            </button>

            {engineMenuOpen && (
              <div
                className="cc-wp-popover"
                role="dialog"
                aria-label={t("wallpapers.sections.engine")}
              >
                <div className="cc-wp-popover-head">
                  <div className="cc-wp-popover-title-wrap">
                    <h3>{t("wallpapers.sections.engine")}</h3>
                    <span className="cc-wp-popover-status">
                      <span className={`cc-wp-status-dot ${statusTone}`} aria-hidden="true" />
                      <span className="trunc">{statusText}</span>
                    </span>
                  </div>
                  <button
                    type="button"
                    className="cc-iconbtn"
                    aria-label={t("window.close")}
                    onClick={() => setEngineMenuOpen(false)}
                  >
                    <X size={15} aria-hidden="true" />
                  </button>
                </div>

                <div className="cc-wp-popover-body">
                  {showScalingControl && activeAssignment && (
                    <div className="cc-wp-popover-group">
                      <span className="cc-wp-popover-group-label">
                        {t("wallpapers.sections.scaling")}
                      </span>
                      <SettingRow
                        title={t("wallpapers.controls.fit")}
                        help={t("wallpapers.controls.fitHelp")}
                        stacked
                      >
                        <Segmented<WallpaperFit>
                          value={activeAssignment.fit}
                          ariaLabel={t("wallpapers.controls.fit")}
                          options={scalingOptions}
                          onChange={(fit) =>
                            void ipc.wallpaperUpdateAssignment(effectiveTarget, { fit })
                          }
                        />
                      </SettingRow>
                    </div>
                  )}

                  <div className="cc-wp-popover-group">
                    <span className="cc-wp-popover-group-label">
                      {t("wallpapers.sections.performance")}
                    </span>
                    <SettingRow
                      title={t("wallpapers.settings.fpsCap")}
                      help={t("wallpapers.settings.fpsCapHelp")}
                      stacked
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
                      stacked
                    >
                      <Segmented<WallpaperResolution>
                        value={engine.resolution}
                        ariaLabel={t("wallpapers.settings.resolution")}
                        options={[
                          { value: "native", label: t("wallpapers.settings.resolutionNative") },
                          {
                            value: "balanced",
                            label: t("wallpapers.settings.resolutionBalanced"),
                          },
                        ]}
                        onChange={(resolution) =>
                          void ipc.wallpaperEnginePatch({ resolution })
                        }
                      />
                    </SettingRow>
                  </div>

                  <div className="cc-wp-popover-group">
                    <span className="cc-wp-popover-group-label">
                      {t("wallpapers.sections.autoPause")}
                    </span>
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
                        onChange={(pauseOnBattery) =>
                          void ipc.wallpaperEnginePatch({ pauseOnBattery })
                        }
                      />
                    </SettingRow>
                  </div>

                  <div className="cc-wp-popover-group">
                    <span className="cc-wp-popover-group-label">
                      {t("wallpapers.sections.quality")}
                    </span>
                    <SettingRow
                      title={t("wallpapers.settings.syncGlass")}
                      help={t("wallpapers.settings.syncGlassHelp")}
                    >
                      <Switch
                        checked={engine.syncGlassPlates}
                        ariaLabel={t("wallpapers.settings.syncGlass")}
                        onChange={(syncGlassPlates) =>
                          void ipc.wallpaperEnginePatch({ syncGlassPlates })
                        }
                      />
                    </SettingRow>
                    <SettingRow
                      title={t("wallpapers.settings.windowsQuality")}
                      help={t("wallpapers.settings.windowsQualityHelp")}
                    >
                      <Switch
                        checked={winQuality?.losslessEnabled ?? false}
                        disabled={!winQuality}
                        ariaLabel={t("wallpapers.settings.windowsQuality")}
                        onChange={(enabled) => void handleToggleWindowsQuality(enabled)}
                      />
                    </SettingRow>
                  </div>
                </div>
              </div>
            )}
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
      </div>

      {activeError && (
        <div className="cc-wp-error-banner" role="alert">
          <WarningCircle size={15} aria-hidden="true" />
          <span>{activeError}</span>
        </div>
      )}

      {/* Current wallpaper + live controls */}
      <div className="cc-wp-now">
        <div className="cc-wp-now-thumb" aria-hidden="true">
          {nowThumbUrl && (
            <div
              className="cc-wp-now-img"
              style={{ backgroundImage: `url("${nowThumbUrl}")` }}
            />
          )}
        </div>
        <div className="cc-wp-now-main">
          <div className="cc-wp-now-head">
            <div className="cc-wp-now-title-group">
              <span className="cc-wp-now-title trunc">
                {activeItem ? activeItem.title : t("wallpapers.controls.windowsCurrent")}
              </span>
              <span className="cc-wp-now-sub trunc">
                {activeItem ? `${statusText} · ${activeItem.subtitle}` : statusText}
              </span>
            </div>
            {monitors.length > 1 && (
              <div className="cc-wp-now-monitor">
                <Select
                  value={effectiveTarget}
                  ariaLabel={t("wallpapers.monitorTarget")}
                  options={monitorOptions}
                  onChange={setTargetMonitor}
                />
              </div>
            )}
            {isActiveLive && (
              <Button variant="secondary" size="sm" onClick={() => void handleStopLive()}>
                <Stop size={12} weight="fill" aria-hidden="true" />
                <span>{t("wallpapers.controls.stopLive")}</span>
              </Button>
            )}
          </div>

          {activeItem && activeAssignment && isActiveLive && (
            <div className="cc-wp-now-params">
              {activeItem.kind === "shader" && (
                <>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.speed")}</span>
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
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.brightness")}</span>
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
                </>
              )}

              {activeItem.kind === "video" && (
                <>
                  <div className="cc-wp-param-row">
                    <span className="cc-field-label">{t("wallpapers.controls.speed")}</span>
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
            </div>
          )}
        </div>
      </div>

      <div className="cc-wp-filters">
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
      </div>

      {catalogLoading ? (
        <div className="cc-wp-grid" aria-busy="true">
          {Array.from({ length: 6 }).map((_, idx) => (
            <div key={idx} className="cc-wp-card is-skeleton">
              <div className="cc-wp-card-stage cc-wp-shimmer" />
            </div>
          ))}
        </div>
      ) : filteredItems.length === 0 && kindFilter !== "custom" ? (
        <div className="cc-empty">
          <div className="cc-empty-illus" aria-hidden="true">
            <div className="cc-empty-box" />
            <div className="cc-empty-box is-tall" />
          </div>
          <h3>{t("wallpapers.empty.filterTitle")}</h3>
          <p>{t("wallpapers.empty.filterBody")}</p>
          <Button variant="secondary" onClick={() => setKindFilter("all")}>
            {t("wallpapers.empty.resetFilters")}
          </Button>
        </div>
      ) : (
        <div className="cc-wp-grid">
          {kindFilter === "custom" && (
            <button
              type="button"
              className="cc-wp-card cc-wp-add"
              disabled={importing}
              onClick={() => void handleImport()}
            >
              <Plus size={20} aria-hidden="true" />
              <span>{importing ? t("wallpapers.importing") : t("wallpapers.addCustom")}</span>
            </button>
          )}
          {filteredItems.map((item) => {
            const isLiveItem = item.kind === "shader" || item.kind === "video";
            const isSelected =
              activeAssignment?.wallpaperId === item.id &&
              !(isLiveItem && engineStatus.runState === "error");
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

                    {isLiveItem && (
                      <span className="cc-wp-live-pill">
                        <Play size={9} weight="fill" aria-hidden="true" />
                        <span>{t("wallpapers.badges.live")}</span>
                      </span>
                    )}

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
                    <span className="cc-wp-card-title trunc">{item.title}</span>
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
