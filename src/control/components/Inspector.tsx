import {
  ArrowDown,
  ArrowUp,
  CaretDown,
  CaretRight,
  Clock,
  CloudSun,
  Copy,
  Gauge,
  Headset,
  MusicNote,
  NotePencil,
  Robot,
  Timer as TimerIcon,
  Trash,
  type Icon,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { AutoRule } from "../../shared/bindings/AutoRule";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { Layer } from "../../shared/bindings/Layer";
import type { ModuleMode } from "../../shared/bindings/ModuleMode";
import type { ResizeBehavior } from "../../shared/bindings/ResizeBehavior";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import type { VerticalAnchor } from "../../shared/bindings/VerticalAnchor";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { WIDGET_REGISTRY, type FreeWidgetKind } from "../../widget/registry";
import { formatWidgetTitle, useControlStore } from "../store";
import { Button } from "./Button";
import { Drawer } from "./Drawer";
import { Segmented } from "./Segmented";
import { Select, type SelectOption } from "./Select";
import { SettingsForm } from "./SettingsForm";
import { Slider } from "./Slider";
import { Switch } from "./Switch";
import { WidgetPreview } from "./WidgetPreview";

type ClusterShape = "box" | "pill";

const MAX_CLUSTER_MODULES = 7;

const CLUSTER_MODULE_CHOICES: ReadonlyArray<{
  id: string;
  kinds: FreeWidgetKind[];
  icon: Icon;
}> = [
  { id: "clock", kinds: ["clock"], icon: Clock },
  { id: "weather", kinds: ["weather"], icon: CloudSun },
  { id: "system", kinds: ["system"], icon: Gauge },
  { id: "nowPlaying", kinds: ["nowPlaying"], icon: MusicNote },
  { id: "discord", kinds: ["discord"], icon: Headset },
  { id: "note", kinds: ["note"], icon: NotePencil },
  { id: "agents", kinds: ["agents"], icon: Robot },
  { id: "timer", kinds: ["timer"], icon: TimerIcon },
];

function defaultAutoRule(kind: FreeWidgetKind, shape: ClusterShape): AutoRule {
  if (kind === "nowPlaying") return "expandWhenPlaying";
  if (kind === "discord") return "expandWhenInCall";
  if (shape === "box" && kind === "system") return "expandWhenHighLoad";
  if (shape === "box" && kind === "agents") return "expandWhenActive";
  if (shape === "box" && kind === "weather") return "fillRemainingSpace";
  return "none";
}

function createClusterModule(kind: FreeWidgetKind, shape: ClusterShape): ClusterModule {
  return {
    id: crypto.randomUUID(),
    kind,
    defaultMode: kind === "nowPlaying" || kind === "discord" ? "hidden" : "compact",
    autoRule: defaultAutoRule(kind, shape),
    settings: { ...WIDGET_REGISTRY[kind].defaultSettings },
  };
}

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
    case "agents":
      return [
        ...common,
        { value: "expandWhenActive", label: t("desktop.ruleExpandWhenActive") },
        { value: "fillRemainingSpace", label: t("desktop.ruleFillRemainingSpace") },
      ];
    case "discord":
      return [
        ...common,
        { value: "expandWhenInCall", label: t("desktop.ruleExpandWhenInCall") },
        { value: "fillRemainingSpace", label: t("desktop.ruleFillRemainingSpace") },
      ];
    case "weather":
    case "note":
    case "clock":
    case "timer":
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

  const instance = useMemo(
    () => config?.widgets.find((w) => w.id === selectedId) ?? null,
    [config, selectedId],
  );

  if (!config || !instance) return null;

  return (
    <WidgetEditorDrawer
      key={instance.id}
      instance={instance}
      config={config}
      monitors={monitors}
      livePlacement={placements[instance.id] ?? null}
      onClose={() => setSelectedWidgetId(null)}
      onSelect={setSelectedWidgetId}
    />
  );
}

function WidgetEditorDrawer({
  instance,
  config,
  monitors,
  livePlacement,
  onClose,
  onSelect,
}: {
  instance: WidgetInstance;
  config: NonNullable<ReturnType<typeof useControlStore.getState>["config"]>;
  monitors: ReturnType<typeof useControlStore.getState>["monitors"];
  livePlacement: ReturnType<typeof useControlStore.getState>["placements"][string] | null;
  onClose: () => void;
  onSelect: (id: string | null) => void;
}) {
  const [advancedOpen, setAdvancedOpen] = useState(false);

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

  const isCluster = instance.kind === "cluster";
  const primaryMonitor =
    monitors.find((m) => m.name === instance.placement.monitor.name) ??
    monitors.find((m) => m.isPrimary) ??
    monitors[0] ??
    null;
  const plateUrl = toAssetUrl(primaryMonitor?.platePath ?? "");

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

  const monIdx = monitors.findIndex((m) => m.name === currentMonitorName);
  const subtitleParts: string[] = [];
  if (isCluster && instance.cluster) {
    subtitleParts.push(
      instance.cluster.presetId === "pillBar"
        ? t("gallery.custom.pill")
        : t("gallery.custom.box"),
    );
  } else {
    subtitleParts.push(t(`size.${instance.size}`));
  }
  if (monitors.length > 1 && monIdx >= 0) {
    subtitleParts.push(t("gallery.monitorNth", { n: monIdx + 1 }));
  }

  const freeMeta = instance.kind === "cluster" ? null : WIDGET_REGISTRY[instance.kind];
  const hasSettings = Boolean(freeMeta && freeMeta.settingsSchema.length > 0);

  return (
    <Drawer
      open
      title={formatWidgetTitle(instance)}
      subtitle={subtitleParts.join(" · ")}
      onClose={onClose}
      footer={
        <div className="cc-wd-foot-actions">
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
            onClick={() => {
              onClose();
              void ipc.widgetRemove(instance.id);
            }}
          >
            <Trash size={13} aria-hidden="true" />
            <span>{t("desktop.removeWidget")}</span>
          </Button>
        </div>
      }
    >
      <div className="cc-wd-preview">
        {plateUrl && (
          <div
            className="cc-preview-bg"
            aria-hidden="true"
            style={{ backgroundImage: `url("${plateUrl}")` }}
          />
        )}
        <div className="cc-stage-scrim" aria-hidden="true" />
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
          maxWidth={320}
          maxHeight={156}
          stageHeight={176}
          interactive={isCluster}
          previewShowAll={isCluster}
        />
      </div>

      {isCluster && instance.cluster ? (
        <ClusterQuickEditor
          cluster={instance.cluster}
          onChange={(nextCluster) =>
            void ipc.widgetUpdate(instance.id, { cluster: nextCluster })
          }
        />
      ) : (
        <>
          {freeMeta && freeMeta.allowedSizes.length > 1 && (
            <div className="cc-insp-field">
              <span className="cc-field-label">{t("desktop.secSize")}</span>
              <Segmented
                fullWidth
                value={instance.size}
                ariaLabel={t("desktop.secSize")}
                options={freeMeta.allowedSizes.map((s) => ({
                  value: s,
                  label: t(`size.${s}`),
                }))}
                onChange={(size) => void ipc.widgetUpdate(instance.id, { size })}
              />
            </div>
          )}

          {hasSettings && instance.kind !== "cluster" && (
            <div className="cc-insp-section">
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
            </div>
          )}
        </>
      )}

      <div className="cc-wd-advanced">
        <button
          type="button"
          className="cc-wd-advanced-toggle"
          aria-expanded={advancedOpen}
          onClick={() => setAdvancedOpen((prev) => !prev)}
        >
          {advancedOpen ? (
            <CaretDown size={13} aria-hidden="true" />
          ) : (
            <CaretRight size={13} aria-hidden="true" />
          )}
          <span>{t("desktop.moreOptions")}</span>
        </button>

        {advancedOpen && (
          <div className="cc-wd-advanced-body">
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
                  onChange={(clickThrough) =>
                    void ipc.widgetUpdate(instance.id, { clickThrough })
                  }
                />
              </div>
              <span className="cc-field-help">{t("desktop.clickThroughHelp")}</span>
            </div>

            {monitorOptions.length > 1 && (
              <div className="cc-insp-field">
                <span className="cc-field-label">{t("desktop.monitorLabelField")}</span>
                <Select
                  options={monitorOptions}
                  value={currentMonitorName}
                  ariaLabel={t("desktop.monitorLabelField")}
                  onChange={(mon) =>
                    void ipc.widgetMove(
                      instance.id,
                      mon,
                      instance.placement.x,
                      instance.placement.y,
                    )
                  }
                />
              </div>
            )}

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
                    onClick={() =>
                      void ipc.widgetUpdate(instance.id, { glassOverride: null })
                    }
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

            {isCluster && instance.cluster && (
              <ClusterAdvancedEditor
                cluster={instance.cluster}
                onChange={(nextCluster) =>
                  void ipc.widgetUpdate(instance.id, { cluster: nextCluster })
                }
              />
            )}
          </div>
        )}
      </div>
    </Drawer>
  );
}

function ClusterQuickEditor({
  cluster,
  onChange,
}: {
  cluster: ClusterConfig;
  onChange: (next: ClusterConfig) => void;
}) {
  const [openModId, setOpenModId] = useState<string | null>(null);
  const shape: ClusterShape = cluster.presetId === "pillBar" ? "pill" : "box";

  const activeKinds = useMemo(
    () =>
      cluster.modules
        .map((m) => m.kind)
        .filter((k): k is FreeWidgetKind => k !== "cluster"),
    [cluster.modules],
  );

  const handleShapeChange = (nextShape: ClusterShape) => {
    if (nextShape === shape) return;
    const modules = cluster.modules.map((m) => {
      if (m.kind === "cluster") return m;
      let autoRule = m.autoRule;
      if (
        nextShape === "pill" &&
        (autoRule === "fillRemainingSpace" ||
          autoRule === "expandWhenHighLoad" ||
          autoRule === "expandWhenActive")
      ) {
        autoRule = "none";
      } else if (nextShape === "box" && autoRule === "none") {
        autoRule = defaultAutoRule(m.kind, "box");
      }
      return { ...m, autoRule };
    });
    onChange({
      ...cluster,
      presetId: nextShape === "pill" ? "pillBar" : null,
      maxHeight: nextShape === "pill" ? 240 : 320,
      modules,
    });
  };

  const toggleChoice = (kinds: FreeWidgetKind[]) => {
    const anyOn = kinds.some((k) => activeKinds.includes(k));
    if (anyOn) {
      const remaining = cluster.modules.filter(
        (m) => m.kind === "cluster" || !kinds.includes(m.kind),
      );
      if (remaining.length === 0) return;
      onChange({ ...cluster, modules: remaining });
      return;
    }
    const toAdd = kinds.filter((k) => !activeKinds.includes(k));
    if (cluster.modules.length + toAdd.length > MAX_CLUSTER_MODULES) return;
    const nextModules = [
      ...cluster.modules,
      ...toAdd.map((k) => createClusterModule(k, shape)),
    ];
    onChange({ ...cluster, modules: nextModules });
  };

  const updateModuleSettings = (modId: string, patch: Record<string, unknown>) => {
    onChange({
      ...cluster,
      modules: cluster.modules.map((m) =>
        m.id === modId ? { ...m, settings: { ...m.settings, ...patch } } : m,
      ),
    });
  };

  const configurableModules = cluster.modules.filter(
    (m) => m.kind !== "cluster" && WIDGET_REGISTRY[m.kind].settingsSchema.length > 0,
  );

  return (
    <div className="cc-insp-section">
      <div className="cc-insp-field">
        <span className="cc-field-label">{t("gallery.custom.shape")}</span>
        <Segmented<ClusterShape>
          fullWidth
          value={shape}
          ariaLabel={t("gallery.custom.shape")}
          options={[
            { value: "box", label: t("gallery.custom.box") },
            { value: "pill", label: t("gallery.custom.pill") },
          ]}
          onChange={handleShapeChange}
        />
      </div>

      <div className="cc-insp-field">
        <div className="cc-insp-field-head">
          <span className="cc-field-label">{t("gallery.custom.modules")}</span>
          <span className="cc-field-help">
            {cluster.modules.length} / {MAX_CLUSTER_MODULES}
          </span>
        </div>
        <div className="cc-modchips" role="group" aria-label={t("gallery.custom.modules")}>
          {CLUSTER_MODULE_CHOICES.map(({ id, kinds, icon: ModuleIcon }) => {
            const isOn = kinds.some((k) => activeKinds.includes(k));
            const remainingIfOff = cluster.modules.filter(
              (m) => m.kind === "cluster" || !kinds.includes(m.kind),
            ).length;
            const missing = kinds.filter((k) => !activeKinds.includes(k)).length;
            const disabled = isOn
              ? remainingIfOff === 0
              : cluster.modules.length + missing > MAX_CLUSTER_MODULES;
            const label = t(`widgets.${id}.name`);

            return (
              <button
                key={id}
                type="button"
                className={`cc-modchip${isOn ? " is-on" : ""}`}
                aria-pressed={isOn}
                disabled={disabled}
                onClick={() => toggleChoice(kinds)}
              >
                <ModuleIcon size={14} weight={isOn ? "fill" : "regular"} aria-hidden="true" />
                <span>{label}</span>
              </button>
            );
          })}
        </div>
      </div>

      {configurableModules.length > 0 && (
        <div className="cc-insp-field">
          <span className="cc-field-label">{t("desktop.moduleSettings")}</span>
          <div className="cc-sort-list">
            {configurableModules.map((mod) => {
              if (mod.kind === "cluster") return null;
              const isOpen = openModId === mod.id;
              return (
                <div key={mod.id} className="cc-mod-editor">
                  <button
                    type="button"
                    className="cc-mod-editor-head"
                    aria-expanded={isOpen}
                    onClick={() => setOpenModId(isOpen ? null : mod.id)}
                  >
                    {isOpen ? <CaretDown size={12} /> : <CaretRight size={12} />}
                    <span className="cc-mod-editor-title">
                      {t(`widgets.${mod.kind}.name`)}
                    </span>
                  </button>
                  {isOpen && (
                    <div className="cc-mod-editor-body">
                      <SettingsForm
                        kind={mod.kind}
                        size="m"
                        settings={mod.settings}
                        onPatch={(patch) => updateModuleSettings(mod.id, patch)}
                      />
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

function ClusterAdvancedEditor({
  cluster,
  onChange,
}: {
  cluster: ClusterConfig;
  onChange: (next: ClusterConfig) => void;
}) {
  const [openRuleId, setOpenRuleId] = useState<string | null>(null);

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
        <span className="cc-field-label">{t("desktop.clusterWidth")}</span>
        <Segmented
          fullWidth
          value={widthVal}
          ariaLabel={t("desktop.clusterWidth")}
          options={[
            { value: "232", label: t("desktop.widthNarrow") },
            { value: "264", label: t("desktop.widthNormal") },
            { value: "288", label: t("desktop.widthWide") },
          ]}
          onChange={(w) => patchCluster({ width: Number(w) })}
        />
      </div>

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

      {cluster.modules.length > 0 && (
        <div className="cc-insp-field">
          <span className="cc-field-label">{t("desktop.moduleOrderRules")}</span>
          <div className="cc-sort-list">
            {cluster.modules.map((mod, idx) => {
              const modKind: FreeWidgetKind = mod.kind === "cluster" ? "clock" : mod.kind;
              const isOpen = openRuleId === mod.id;
              return (
                <div key={mod.id} className="cc-mod-editor">
                  <div className="cc-mod-editor-row">
                    <button
                      type="button"
                      className="cc-mod-editor-head"
                      aria-expanded={isOpen}
                      onClick={() => setOpenRuleId(isOpen ? null : mod.id)}
                    >
                      {isOpen ? <CaretDown size={12} /> : <CaretRight size={12} />}
                      <span className="cc-mod-editor-title">
                        {t(`widgets.${modKind}.name`)}
                      </span>
                    </button>
                    <div className="cc-mod-editor-arrows">
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
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </>
  );
}
