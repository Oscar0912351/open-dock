import {
  ArrowDown,
  ArrowUp,
  CaretDown,
  CaretRight,
  Copy,
  CursorClick,
  DotsSixVertical,
  Trash,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { AutoRule } from "../../shared/bindings/AutoRule";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { GlassEngine } from "../../shared/bindings/GlassEngine";
import type { Layer } from "../../shared/bindings/Layer";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { ResizeBehavior } from "../../shared/bindings/ResizeBehavior";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { VerticalAnchor } from "../../shared/bindings/VerticalAnchor";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { WIDGET_LIST, WIDGET_REGISTRY, type FreeWidgetKind } from "../../widget/registry";
import { formatWidgetTitle, useControlStore } from "../store";
import { Button } from "./Button";
import { Input } from "./Input";
import { Segmented } from "./Segmented";
import { Select, type SelectOption } from "./Select";
import { SettingsForm } from "./SettingsForm";
import { Slider } from "./Slider";
import { Switch } from "./Switch";
import { WidgetPreview } from "./WidgetPreview";

type InspectorTab = "content" | "appearance" | "placement";

function computeEffectiveEngine(
  preferred: GlassEngine,
  layer: Layer,
  size: WidgetSize,
  isWin11: boolean,
): GlassEngine {
  let chosen = preferred;
  if (layer !== "desktop" && chosen === "wallpaper") {
    chosen = "acrylic";
  }
  if (chosen === "acrylic" && (!isWin11 || size === "pill")) {
    chosen = "tint";
  }
  return chosen;
}

const ENGINE_LABELS: Record<GlassEngine, string> = {
  wallpaper: "engine.wallpaper",
  acrylic: "engine.acrylic",
  tint: "engine.tint",
};

function autoRulesForKind(kind: FreeWidgetKind): SelectOption<AutoRule>[] {
  const common: SelectOption<AutoRule>[] = [
    { value: "none", label: t("desktop.ruleNone") },
  ];
  switch (kind) {
    case "nowPlaying":
      return [
        ...common,
        { value: "expandWhenPlaying", label: t("desktop.ruleExpandWhenPlaying") },
        { value: "collapseWhenIdle", label: t("desktop.ruleCollapseWhenIdle") },
      ];
    case "system":
      return [
        ...common,
        { value: "expandWhenHighLoad", label: t("desktop.ruleExpandWhenHighLoad") },
        { value: "fillRemainingSpace", label: t("desktop.ruleFillRemainingSpace") },
      ];
    case "weather":
    case "note":
    case "clock":
    case "discord":
      return [
        ...common,
        { value: "fillRemainingSpace", label: t("desktop.ruleFillRemainingSpace") },
      ];
  }
}

export function Inspector() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const placements = useControlStore((s) => s.placements);
  const selectedId = useControlStore((s) => s.selectedWidgetId);
  const setSelectedWidgetId = useControlStore((s) => s.setSelectedWidgetId);
  const isWindows11 = useControlStore((s) => s.isWindows11);

  const instance = useMemo(
    () => config?.widgets.find((w) => w.id === selectedId) ?? null,
    [config, selectedId],
  );

  if (!config || !instance) {
    return (
      <aside className="cc-inspector">
        <div className="cc-inspector-empty">
          <div className="cc-inspector-empty-icon" aria-hidden="true">
            <CursorClick size={18} />
          </div>
          <strong style={{ color: "var(--c-ink-1)", fontWeight: 600 }}>
            {t("desktop.noneSelected")}
          </strong>
          <span>{t("desktop.noneSelectedHint")}</span>
        </div>
      </aside>
    );
  }

  return (
    <SelectedInspector
      key={instance.id}
      instance={instance}
      config={config}
      monitors={monitors}
      livePlacement={placements[instance.id] ?? null}
      isWindows11={isWindows11}
      onSelect={setSelectedWidgetId}
    />
  );
}

function SelectedInspector({
  instance,
  config,
  monitors,
  livePlacement,
  isWindows11,
  onSelect,
}: {
  instance: WidgetInstance;
  config: NonNullable<ReturnType<typeof useControlStore.getState>["config"]>;
  monitors: ReturnType<typeof useControlStore.getState>["monitors"];
  livePlacement: ReturnType<typeof useControlStore.getState>["placements"][string] | null;
  isWindows11: boolean;
  onSelect: (id: string | null) => void;
}) {
  const [tab, setTab] = useState<InspectorTab>("content");
  const [scaleOverride, setScaleOverride] = useState<{ base: number; pct: number } | null>(null);
  const scaleDraft =
    scaleOverride && scaleOverride.base === instance.scale
      ? scaleOverride.pct
      : Math.round(instance.scale * 100);

  const baseGlass = instance.glassOverride ?? config.appearance.glassStrength;
  const [glassOverride, setGlassOverride] = useState<{ base: number; pct: number } | null>(null);
  const glassDraft =
    glassOverride && glassOverride.base === baseGlass
      ? glassOverride.pct
      : Math.round(baseGlass * 100);

  const [posDraft, setPosDraft] = useState<{
    baseX: number;
    baseY: number;
    x: string;
    y: string;
  } | null>(null);
  const posMatches =
    posDraft !== null &&
    posDraft.baseX === instance.placement.x &&
    posDraft.baseY === instance.placement.y;
  const posX = posMatches ? posDraft.x : String(Math.round(instance.placement.x));
  const posY = posMatches ? posDraft.y : String(Math.round(instance.placement.y));

  const scaleTimer = useRef<number | null>(null);
  const glassTimer = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (scaleTimer.current !== null) window.clearTimeout(scaleTimer.current);
      if (glassTimer.current !== null) window.clearTimeout(glassTimer.current);
    };
  }, []);

  const handleScaleChange = (pct: number) => {
    setScaleOverride({ base: instance.scale, pct });
    if (scaleTimer.current !== null) window.clearTimeout(scaleTimer.current);
    scaleTimer.current = window.setTimeout(() => {
      scaleTimer.current = null;
      void ipc.widgetUpdate(instance.id, { scale: pct / 100 });
    }, 60);
  };

  const handleGlassChange = (pct: number) => {
    setGlassOverride({ base: baseGlass, pct });
    if (glassTimer.current !== null) window.clearTimeout(glassTimer.current);
    glassTimer.current = window.setTimeout(() => {
      glassTimer.current = null;
      void ipc.widgetUpdate(instance.id, { glassOverride: pct / 100 });
    }, 60);
  };

  const setPosX = (x: string) => {
    setPosDraft({
      baseX: instance.placement.x,
      baseY: instance.placement.y,
      x,
      y: posY,
    });
  };

  const setPosY = (y: string) => {
    setPosDraft({
      baseX: instance.placement.x,
      baseY: instance.placement.y,
      x: posX,
      y,
    });
  };

  const commitPosition = () => {
    const nx = Number(posX);
    const ny = Number(posY);
    if (!Number.isFinite(nx) || !Number.isFinite(ny)) {
      setPosDraft(null);
      return;
    }
    setPosDraft(null);
    void ipc.widgetMove(instance.id, instance.placement.monitor.name, nx, ny);
  };

  const isCluster = instance.kind === "cluster";
  const effectiveEngine = computeEffectiveEngine(
    config.appearance.glassEngine,
    instance.layer,
    instance.size,
    isWindows11,
  );
  const engineDiffers = effectiveEngine !== config.appearance.glassEngine;

  const themeOptions: SelectOption<string>[] = [
    {
      value: "",
      label: t("theme.asApp", { name: t(`theme.${config.appearance.themeId}`) }),
    },
    { value: "aurora", label: t("theme.aurora") },
    { value: "laerk", label: t("theme.laerk") },
    { value: "grafit", label: t("theme.grafit") },
  ];

  const layerOptions: SelectOption<Layer>[] = [
    { value: "desktop", label: t("layer.desktop"), help: t("layer.desktopHelp") },
    { value: "top", label: t("layer.top"), help: t("layer.topHelp") },
    { value: "normal", label: t("layer.normal"), help: t("layer.normalHelp") },
  ];

  const monitorOptions: SelectOption<string>[] = monitors.map((m, idx) => ({
    value: m.name,
    label: m.isPrimary
      ? t("gallery.monitorPrimary", { n: idx + 1 })
      : t("gallery.monitorNth", { n: idx + 1 }),
  }));

  const currentMonitorName =
    monitors.find((m) => m.name === instance.placement.monitor.name)?.name ??
    monitors.find((m) => m.isPrimary)?.name ??
    monitors[0]?.name ??
    instance.placement.monitor.name;

  const sizeLabel =
    isCluster && instance.cluster
      ? `${instance.cluster.width} px`
      : t(`size.${instance.size}`);

  return (
    <aside className="cc-inspector">
      {/* Pinned Top Header */}
      <div className="cc-inspector-head">
        <div style={{ minWidth: 0 }}>
          <div className="cc-inspector-head-title">{formatWidgetTitle(instance)}</div>
          <div className="cc-inspector-head-meta">{sizeLabel}</div>
        </div>
        <div style={{ display: "flex", gap: 4, flex: "none" }}>
          <button
            type="button"
            className="cc-iconbtn"
            aria-label={t("desktop.duplicate")}
            title={t("desktop.duplicate")}
            onClick={() => {
              void ipc.widgetDuplicate(instance.id).then((copy) => onSelect(copy.id));
            }}
          >
            <Copy size={14} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="cc-iconbtn is-danger"
            aria-label={t("desktop.removeWidget")}
            title={t("desktop.removeWidget")}
            onClick={() => void ipc.widgetRemove(instance.id)}
          >
            <Trash size={14} aria-hidden="true" />
          </button>
        </div>
      </div>

      {/* Pinned Live Preview Stage */}
      <div className="cc-inspector-preview">
        <WidgetPreview
          instance={instance}
          id={instance.id}
          kind={instance.kind}
          size={instance.size}
          settings={instance.settings}
          cluster={instance.cluster}
          appearance={config.appearance}
          monitors={monitors}
          locations={config.weather.locations}
          livePlacement={livePlacement}
          maxWidth={312}
          maxHeight={142}
          stageHeight={160}
          interactive={isCluster}
        />
      </div>

      {/* Pinned Tab Strip */}
      <div className="cc-inspector-tabs">
        <Segmented<InspectorTab>
          fullWidth
          value={tab}
          ariaLabel={formatWidgetTitle(instance)}
          options={[
            { value: "content", label: t("desktop.tabContent") },
            { value: "appearance", label: t("desktop.tabAppearance") },
            { value: "placement", label: t("desktop.tabPlacement") },
          ]}
          onChange={setTab}
        />
      </div>

      {/* Independent Scrollable Tab Content */}
      <div className="cc-inspector-body">
        {tab === "content" && (
          <>
            {instance.kind !== "cluster" && (
              <div className="cc-insp-section">
                <span className="cc-section-label">{t("desktop.secSize")}</span>
                <Segmented
                  fullWidth
                  value={instance.size}
                  ariaLabel={t("desktop.secSize")}
                  options={WIDGET_REGISTRY[instance.kind].allowedSizes.map((s) => ({
                    value: s,
                    label: t(`size.${s}`),
                  }))}
                  onChange={(size) => void ipc.widgetUpdate(instance.id, { size })}
                />
              </div>
            )}

            <div className="cc-insp-section">
              <span className="cc-section-label">{t("desktop.secContent")}</span>
              {instance.kind === "cluster" && instance.cluster ? (
                <ClusterEditor
                  cluster={instance.cluster}
                  onChange={(nextCluster) =>
                    void ipc.widgetUpdate(instance.id, { cluster: nextCluster })
                  }
                />
              ) : (
                instance.kind !== "cluster" && (
                  <SettingsForm
                    kind={instance.kind}
                    size={instance.size}
                    settings={instance.settings}
                    onPatch={(patch) =>
                      void ipc.widgetUpdate(instance.id, {
                        settings: { ...instance.settings, ...patch },
                      })
                    }
                  />
                )
              )}
            </div>
          </>
        )}

        {tab === "appearance" && (
          <div className="cc-insp-section">
            <div className="cc-insp-field">
              <span className="cc-field-label">{t("desktop.scale")}</span>
              <Slider
                value={scaleDraft}
                min={75}
                max={135}
                step={5}
                ariaLabel={t("desktop.scale")}
                formatValue={(v) => `${v} %`}
                onChange={handleScaleChange}
              />
            </div>

            <div className="cc-insp-field">
              <span className="cc-field-label">{t("desktop.themeLabel")}</span>
              <Select
                options={themeOptions}
                value={instance.themeOverride ?? ""}
                ariaLabel={t("desktop.themeLabel")}
                onChange={(val) =>
                  void ipc.widgetUpdate(instance.id, {
                    themeOverride: val ? (val as ThemeId) : null,
                  })
                }
              />
            </div>

            <div className="cc-insp-field">
              <div className="cc-insp-field-head">
                <span className="cc-field-label">{t("desktop.glassLabel")}</span>
                {instance.glassOverride !== null && (
                  <button
                    type="button"
                    className="cc-link-btn"
                    onClick={() => void ipc.widgetUpdate(instance.id, { glassOverride: null })}
                  >
                    {t("desktop.reset")}
                  </button>
                )}
              </div>
              <Slider
                value={glassDraft}
                min={60}
                max={140}
                step={5}
                ariaLabel={t("desktop.glassLabel")}
                formatValue={(v) => `${v} %`}
                onChange={handleGlassChange}
              />
            </div>

            <div className="cc-info-banner">
              <span>
                {engineDiffers && instance.layer !== "desktop"
                  ? t("engine.fallbackTop", { engine: t(ENGINE_LABELS[effectiveEngine]) })
                  : t("engine.effective", { engine: t(ENGINE_LABELS[effectiveEngine]) })}
              </span>
            </div>
          </div>
        )}

        {tab === "placement" && (
          <>
            <div className="cc-insp-section">
              <div className="cc-insp-field">
                <span className="cc-field-label">{t("desktop.layerLabel")}</span>
                <Select
                  options={layerOptions}
                  value={instance.layer}
                  ariaLabel={t("desktop.layerLabel")}
                  onChange={(layer) => void ipc.widgetUpdate(instance.id, { layer })}
                />
              </div>

              <div className="cc-insp-field">
                <div className="cc-insp-field-head">
                  <span className="cc-field-label">{t("desktop.clickThrough")}</span>
                  <Switch
                    checked={instance.clickThrough}
                    ariaLabel={t("desktop.clickThrough")}
                    onChange={(clickThrough) => void ipc.widgetUpdate(instance.id, { clickThrough })}
                  />
                </div>
                <span className="cc-field-help">{t("desktop.clickThroughHelp")}</span>
              </div>

              {monitorOptions.length > 0 && (
                <div className="cc-insp-field">
                  <span className="cc-field-label">{t("desktop.monitorLabelField")}</span>
                  <Select
                    options={monitorOptions}
                    value={currentMonitorName}
                    ariaLabel={t("desktop.monitorLabelField")}
                    onChange={(mon) =>
                      void ipc.widgetMove(instance.id, mon, instance.placement.x, instance.placement.y)
                    }
                  />
                </div>
              )}

              <div className="cc-xy-grid">
                <div className="cc-insp-field">
                  <span className="cc-field-label">{t("desktop.posX")}</span>
                  <Input
                    numeric
                    type="number"
                    value={posX}
                    aria-label={t("desktop.posX")}
                    onChange={(e) => setPosX(e.target.value)}
                    onBlur={commitPosition}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitPosition();
                    }}
                  />
                </div>
                <div className="cc-insp-field">
                  <span className="cc-field-label">{t("desktop.posY")}</span>
                  <Input
                    numeric
                    type="number"
                    value={posY}
                    aria-label={t("desktop.posY")}
                    onChange={(e) => setPosY(e.target.value)}
                    onBlur={commitPosition}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitPosition();
                    }}
                  />
                </div>
              </div>
            </div>

            <div className="cc-insp-section">
              <span className="cc-section-label">{t("desktop.secDanger")}</span>
              <div style={{ display: "flex", gap: 8 }}>
                <Button
                  variant="secondary"
                  style={{ flex: 1 }}
                  onClick={() => {
                    void ipc.widgetDuplicate(instance.id).then((copy) => onSelect(copy.id));
                  }}
                >
                  <Copy size={13} aria-hidden="true" />
                  <span>{t("desktop.duplicate")}</span>
                </Button>
                <Button
                  variant="danger"
                  style={{ flex: 1 }}
                  onClick={() => void ipc.widgetRemove(instance.id)}
                >
                  <Trash size={13} aria-hidden="true" />
                  <span>{t("desktop.removeWidget")}</span>
                </Button>
              </div>
            </div>
          </>
        )}
      </div>
    </aside>
  );
}

function ClusterEditor({
  cluster,
  onChange,
}: {
  cluster: ClusterConfig;
  onChange: (next: ClusterConfig) => void;
}) {
  const [expandedModId, setExpandedModId] = useState<string | null>(null);
  const [dragIdx, setDragIdx] = useState<number | null>(null);

  const patchCluster = (patch: Partial<ClusterConfig>) => {
    onChange({ ...cluster, ...patch });
  };

  const updateModule = (modId: string, patch: Partial<ClusterModule>) => {
    onChange({
      ...cluster,
      modules: cluster.modules.map((m) => (m.id === modId ? { ...m, ...patch } : m)),
    });
  };

  const moveModule = (fromIdx: number, toIdx: number) => {
    if (fromIdx === toIdx || toIdx < 0 || toIdx >= cluster.modules.length) return;
    const next = [...cluster.modules];
    const [item] = next.splice(fromIdx, 1);
    if (!item) return;
    next.splice(toIdx, 0, item);
    onChange({ ...cluster, modules: next });
  };

  const addModule = (kind: FreeWidgetKind) => {
    if (cluster.modules.length >= 5) return;
    const defaultRule: AutoRule =
      kind === "nowPlaying"
        ? "expandWhenPlaying"
        : kind === "system"
          ? "expandWhenHighLoad"
          : "none";
    const newMod: ClusterModule = {
      id: crypto.randomUUID(),
      kind,
      defaultMode: kind === "nowPlaying" ? "hidden" : "compact",
      autoRule: defaultRule,
      settings: { ...WIDGET_REGISTRY[kind].defaultSettings },
    };
    onChange({ ...cluster, modules: [...cluster.modules, newMod] });
    setExpandedModId(newMod.id);
  };

  const modeOptions: SelectOption<ModuleMode>[] = [
    { value: "hidden", label: t("desktop.modeHidden") },
    { value: "compact", label: t("desktop.modeCompact") },
    { value: "expanded", label: t("desktop.modeExpanded") },
  ];

  const widthVal =
    cluster.width <= 232 ? "232" : cluster.width >= 288 ? "288" : "264";

  return (
    <>
      <div className="cc-insp-field">
        <span className="cc-field-label">{t("desktop.clusterBehavior")}</span>
        <Segmented<ResizeBehavior>
          fullWidth
          value={cluster.resizeBehavior}
          ariaLabel={t("desktop.clusterBehavior")}
          options={[
            { value: "fixed", label: t("desktop.behaviorFixed") },
            { value: "adaptive", label: t("desktop.behaviorAdaptive") },
          ]}
          onChange={(resizeBehavior) => patchCluster({ resizeBehavior })}
        />
      </div>

      <div className="cc-insp-field">
        <span className="cc-field-label">{t("desktop.clusterWidth")}</span>
        <Segmented
          fullWidth
          value={widthVal}
          ariaLabel={t("desktop.clusterWidth")}
          options={[
            { value: "232", label: "232 px" },
            { value: "264", label: "264 px" },
            { value: "288", label: "288 px" },
          ]}
          onChange={(w) => patchCluster({ width: Number(w) })}
        />
      </div>

      <div className="cc-insp-field">
        <span className="cc-field-label">{t("desktop.clusterAnchor")}</span>
        <Segmented<VerticalAnchor>
          fullWidth
          value={cluster.anchor}
          ariaLabel={t("desktop.clusterAnchor")}
          options={[
            { value: "top", label: t("desktop.anchorTop") },
            { value: "bottom", label: t("desktop.anchorBottom") },
          ]}
          onChange={(anchor) => patchCluster({ anchor })}
        />
      </div>

      <div className="cc-insp-field">
        <div className="cc-insp-field-head">
          <span className="cc-field-label">{t("desktop.modulesHeading")}</span>
          <span className="cc-field-help cc-mono">{cluster.modules.length} / 5</span>
        </div>

        <div className="cc-sort-list">
          {cluster.modules.map((mod, idx) => {
            const isOpen = expandedModId === mod.id;
            const modKind = mod.kind === "cluster" ? "clock" : mod.kind;
            return (
              <div
                key={mod.id}
                draggable
                onDragStart={() => setDragIdx(idx)}
                onDragOver={(e) => {
                  if (dragIdx !== null) e.preventDefault();
                }}
                onDrop={() => {
                  if (dragIdx !== null) moveModule(dragIdx, idx);
                  setDragIdx(null);
                }}
                className="cc-mod-editor"
              >
                <div
                  className="cc-mod-editor-head"
                  onClick={() => setExpandedModId(isOpen ? null : mod.id)}
                >
                  <span className="cc-drag-grip" aria-label={t("desktop.dragHandle")}>
                    <DotsSixVertical size={13} />
                  </span>
                  {isOpen ? <CaretDown size={12} /> : <CaretRight size={12} />}
                  <span style={{ fontSize: 12.5, fontWeight: 500, flex: 1 }}>
                    {t(`widgets.${modKind}.name`)}
                  </span>
                  <div
                    style={{ display: "flex", gap: 2 }}
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      className="cc-iconbtn"
                      style={{ width: 24, height: 24 }}
                      disabled={idx === 0}
                      aria-label={t("desktop.moveUp")}
                      onClick={() => moveModule(idx, idx - 1)}
                    >
                      <ArrowUp size={12} />
                    </button>
                    <button
                      type="button"
                      className="cc-iconbtn"
                      style={{ width: 24, height: 24 }}
                      disabled={idx === cluster.modules.length - 1}
                      aria-label={t("desktop.moveDown")}
                      onClick={() => moveModule(idx, idx + 1)}
                    >
                      <ArrowDown size={12} />
                    </button>
                  </div>
                </div>

                {isOpen && (
                  <div className="cc-mod-editor-body">
                    <div className="cc-insp-field">
                      <span className="cc-field-label">{t("desktop.defaultMode")}</span>
                      <Select
                        options={modeOptions}
                        value={mod.defaultMode}
                        ariaLabel={t("desktop.defaultMode")}
                        onChange={(defaultMode) => updateModule(mod.id, { defaultMode })}
                      />
                    </div>
                    <div className="cc-insp-field">
                      <span className="cc-field-label">{t("desktop.autoRule")}</span>
                      <Select
                        options={autoRulesForKind(modKind)}
                        value={mod.autoRule}
                        ariaLabel={t("desktop.autoRule")}
                        onChange={(autoRule) => updateModule(mod.id, { autoRule })}
                      />
                    </div>
                    <SettingsForm
                      kind={modKind}
                      size="m"
                      settings={mod.settings}
                      onPatch={(patch) =>
                        updateModule(mod.id, { settings: { ...mod.settings, ...patch } })
                      }
                    />
                    {cluster.modules.length > 2 && (
                      <Button
                        variant="danger"
                        size="sm"
                        onClick={() => {
                          onChange({
                            ...cluster,
                            modules: cluster.modules.filter((m) => m.id !== mod.id),
                          });
                          setExpandedModId(null);
                        }}
                      >
                        <Trash size={12} aria-hidden="true" />
                        <span>{t("desktop.removeModule")}</span>
                      </Button>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>

        {cluster.modules.length < 5 && (
          <Select
            placeholder={`${t("desktop.addModule")} +`}
            value=""
            ariaLabel={t("desktop.addModule")}
            options={WIDGET_LIST.map((meta) => ({
              value: meta.kind,
              label: `${t(`widgets.${meta.kind}.name`)}`,
            }))}
            onChange={(val) => {
              if (val) addModule(val as FreeWidgetKind);
            }}
          />
        )}
      </div>
    </>
  );
}
