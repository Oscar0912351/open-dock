import {
  CaretDown,
  Check,
  Clock,
  CloudSun,
  Gauge,
  Headset,
  MusicNote,
  NotePencil,
  Plus,
  Robot,
  Sparkle,
  X,
  type Icon,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Appearance } from "../../shared/bindings/Appearance";
import type { AutoRule } from "../../shared/bindings/AutoRule";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { Location } from "../../shared/bindings/Location";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { reportError } from "../../shared/logging";
import { CLUSTER_PRESETS, WIDGET_REGISTRY, type FreeWidgetKind } from "../../widget/registry";
import { Button } from "../components/Button";
import { getOverlayPortalRoot } from "../components/Drawer";
import { Segmented } from "../components/Segmented";
import { WidgetPreview } from "../components/WidgetPreview";
import { useControlStore } from "../store";

type MonitorInfo = ReturnType<typeof useControlStore.getState>["monitors"][number];

const CUSTOM_MODULE_CHOICES: ReadonlyArray<{
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
];

const MAX_CUSTOM_MODULES = 7;

type CustomShape = "box" | "pill";

interface CustomDialogSeed {
  seq: number;
  shape: CustomShape;
  selected: FreeWidgetKind[];
  editingWidgetId: string | null;
  baseCluster: ClusterConfig | null;
}

function customAutoRule(kind: FreeWidgetKind, shape: CustomShape): AutoRule {
  if (kind === "nowPlaying") return "expandWhenPlaying";
  if (kind === "discord") return "expandWhenInCall";
  if (shape === "box" && kind === "system") return "expandWhenHighLoad";
  if (shape === "box" && kind === "agents") return "expandWhenActive";
  if (shape === "box" && kind === "weather") return "fillRemainingSpace";
  return "none";
}

function createCustomModule(kind: FreeWidgetKind, shape: CustomShape): ClusterModule {
  return {
    id: crypto.randomUUID(),
    kind,
    defaultMode: kind === "nowPlaying" || kind === "discord" ? "hidden" : "compact",
    autoRule: customAutoRule(kind, shape),
    settings: { ...WIDGET_REGISTRY[kind].defaultSettings },
  };
}

function buildCustomCluster(shape: CustomShape, selected: FreeWidgetKind[]): ClusterConfig {
  return {
    presetId: shape === "pill" ? "pillBar" : null,
    width: 264,
    maxHeight: shape === "pill" ? 240 : 320,
    resizeBehavior: "adaptive",
    anchor: "top",
    modules: selected.map((kind) => createCustomModule(kind, shape)),
  };
}

function updateExistingCluster(
  base: ClusterConfig,
  shape: CustomShape,
  selected: FreeWidgetKind[],
): ClusterConfig {
  const baseShape: CustomShape = base.presetId === "pillBar" ? "pill" : "box";
  const shapeChanged = shape !== baseShape;
  const existingByKind = new Map<FreeWidgetKind, ClusterModule>();
  for (const m of base.modules) {
    if (m.kind !== "cluster" && !existingByKind.has(m.kind)) {
      existingByKind.set(m.kind, m);
    }
  }

  const modules: ClusterModule[] = selected.map((kind) => {
    const existing = existingByKind.get(kind);
    if (!existing) {
      return createCustomModule(kind, shape);
    }
    if (!shapeChanged) {
      return existing;
    }
    let autoRule = existing.autoRule;
    if (
      shape === "pill" &&
      (autoRule === "fillRemainingSpace" ||
        autoRule === "expandWhenHighLoad" ||
        autoRule === "expandWhenActive")
    ) {
      autoRule = "none";
    } else if (shape === "box" && autoRule === "none") {
      autoRule = customAutoRule(kind, "box");
    }
    return { ...existing, autoRule };
  });

  return {
    ...base,
    presetId: shapeChanged ? (shape === "pill" ? "pillBar" : null) : base.presetId,
    maxHeight: shapeChanged ? (shape === "pill" ? 240 : 320) : base.maxHeight,
    modules,
  };
}

function mod(
  presetKey: string,
  kind: FreeWidgetKind,
  defaultMode: ClusterModule["defaultMode"],
  autoRule: ClusterModule["autoRule"],
  settingsOverride?: Record<string, unknown>,
): ClusterModule {
  return {
    id: `preview-${presetKey}-${kind}`,
    kind,
    defaultMode,
    autoRule,
    settings: settingsOverride ?? { ...WIDGET_REGISTRY[kind].defaultSettings },
  };
}

export function buildPresetClusters(presetId: "allInOne" | "miniTrio" | "pillBar"): ClusterConfig[] {
  if (presetId === "allInOne") {
    return [
      {
        presetId: "allInOne",
        width: 264,
        maxHeight: 320,
        resizeBehavior: "adaptive",
        anchor: "top",
        modules: [
          mod("aio", "clock", "compact", "none"),
          mod("aio", "weather", "compact", "fillRemainingSpace"),
          mod("aio", "system", "compact", "expandWhenHighLoad"),
          mod("aio", "nowPlaying", "hidden", "expandWhenPlaying"),
          mod("aio", "discord", "hidden", "expandWhenInCall"),
        ],
      },
    ];
  }
  if (presetId === "miniTrio") {
    return [
      {
        presetId: "miniTrioA",
        width: 232,
        maxHeight: 240,
        resizeBehavior: "adaptive",
        anchor: "top",
        modules: [
          mod("trioA", "clock", "compact", "none"),
          mod("trioA", "weather", "compact", "none"),
        ],
      },
      {
        presetId: "miniTrioB",
        width: 232,
        maxHeight: 240,
        resizeBehavior: "adaptive",
        anchor: "top",
        modules: [
          mod("trioB", "nowPlaying", "hidden", "expandWhenPlaying"),
          mod("trioB", "clock", "compact", "none"),
        ],
      },
      {
        presetId: "miniTrioC",
        width: 232,
        maxHeight: 240,
        resizeBehavior: "adaptive",
        anchor: "top",
        modules: [
          mod("trioC", "system", "compact", "expandWhenHighLoad"),
          mod("trioC", "note", "compact", "none", {
            title: "Indkøb",
            mode: "checklist",
            fontSize: "m",
          }),
        ],
      },
    ];
  }
  return [
    {
      presetId: "pillBar",
      width: 264,
      maxHeight: 240,
      resizeBehavior: "adaptive",
      anchor: "top",
      modules: [
        mod("pill", "clock", "compact", "none"),
        mod("pill", "weather", "compact", "none"),
        mod("pill", "system", "compact", "none"),
        mod("pill", "nowPlaying", "hidden", "expandWhenPlaying"),
      ],
    },
  ];
}

function seedFromPreset(presetId: "allInOne" | "miniTrio" | "pillBar"): {
  shape: CustomShape;
  selected: FreeWidgetKind[];
} {
  const selected: FreeWidgetKind[] = [];
  for (const box of buildPresetClusters(presetId)) {
    for (const m of box.modules) {
      const kind = m.kind as FreeWidgetKind;
      if (!selected.includes(kind)) selected.push(kind);
    }
  }
  return { shape: presetId === "pillBar" ? "pill" : "box", selected };
}

export function Gallery() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const setRoute = useControlStore((s) => s.setRoute);
  const pushToast = useControlStore((s) => s.pushToast);

  const [addedKey, setAddedKey] = useState<string | null>(null);
  const [customSeed, setCustomSeed] = useState<CustomDialogSeed | null>(null);
  const dialogSeqRef = useRef(0);
  const addedTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (addedTimerRef.current !== null) window.clearTimeout(addedTimerRef.current);
    };
  }, []);

  if (!config) return null;

  const openCustomizeDialog = (params: Omit<CustomDialogSeed, "seq">) => {
    dialogSeqRef.current += 1;
    setCustomSeed({ seq: dialogSeqRef.current, ...params });
  };

  const markAdded = (key: string) => {
    setAddedKey(key);
    if (addedTimerRef.current !== null) window.clearTimeout(addedTimerRef.current);
    addedTimerRef.current = window.setTimeout(() => {
      addedTimerRef.current = null;
      setAddedKey(null);
    }, 1600);
  };

  const handleAddPreset = async (
    presetId: "allInOne" | "miniTrio" | "pillBar",
    monitorName?: string,
  ) => {
    const created = await ipc.clusterAddPreset(presetId, monitorName ?? null);
    markAdded(`p:${presetId}`);
    const name = t(`cluster.presets.${presetId}`);
    const firstId = created[0]?.id ?? null;
    pushToast({
      message: t("toast.added", { name }),
      actionLabel: t("toast.show"),
      onAction: () => setRoute("desktop", firstId),
    });
  };

  const handleAddCustom = async (cluster: ClusterConfig, monitorName?: string) => {
    const created = await ipc.clusterAdd([cluster], monitorName ?? null);
    const firstId = created[0]?.id ?? null;
    pushToast({
      message: t("toast.added", { name: t("cluster.title") }),
      actionLabel: t("toast.show"),
      onAction: () => setRoute("desktop", firstId),
    });
  };

  const handleSaveExisting = async (
    widgetId: string,
    cluster: ClusterConfig,
  ): Promise<boolean> => {
    try {
      await ipc.widgetUpdate(widgetId, { cluster });
      pushToast({
        message: t("toast.saved", { name: t("cluster.title") }),
        actionLabel: t("toast.show"),
        onAction: () => setRoute("desktop", widgetId),
      });
      return true;
    } catch (reason) {
      void reportError(`widgetUpdate (${widgetId}): ${String(reason)}`);
      pushToast({ message: t("gallery.custom.saveError") });
      return false;
    }
  };

  const primaryMonitor = monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null;
  const plateUrl = toAssetUrl(primaryMonitor?.platePath ?? "");

  // "Nuværende" shows and edits the cluster the user actually runs; falls back to "Din egen" if none.
  const ownWidget =
    config.widgets.find((w) => w.kind === "cluster" && w.cluster) ?? null;
  const ownCluster = ownWidget?.cluster ?? null;
  const ownSeed = ownCluster
    ? {
        shape: (ownCluster.presetId === "pillBar" ? "pill" : "box") as CustomShape,
        selected: ownCluster.modules
          .map((m) => m.kind as FreeWidgetKind)
          .filter((k, i, a) => a.indexOf(k) === i),
      }
    : { shape: "box" as CustomShape, selected: ["clock", "weather", "system"] as FreeWidgetKind[] };
  const isEditingOwn = ownWidget !== null && ownCluster !== null;
  const openOwnDialog = () =>
    openCustomizeDialog({
      ...ownSeed,
      editingWidgetId: ownWidget?.id ?? null,
      baseCluster: ownCluster,
    });

  const customCard = (
    <article className="cc-gallery-card">
      <button
        type="button"
        className="cc-gallery-stage cc-custom-cta"
        onClick={openOwnDialog}
      >
        {plateUrl && (
          <div
            className="cc-preview-bg"
            aria-hidden="true"
            style={{ backgroundImage: `url("${plateUrl}")` }}
          />
        )}
        <div className="cc-stage-scrim" aria-hidden="true" />
        <WidgetPreview
          id="preview-custom-cta"
          kind="cluster"
          size="m"
          cluster={ownCluster ?? buildCustomCluster("box", ownSeed.selected)}
          appearance={config.appearance}
          monitors={monitors}
          locations={config.weather.locations}
          maxWidth={256}
          maxHeight={184}
          previewShowAll
        />
      </button>

      <div className="cc-gallery-body">
        <div className="cc-gallery-meta">
          <div className="cc-gallery-title">
            {isEditingOwn ? t("gallery.custom.currentTitle") : t("gallery.custom.title")}
          </div>
          <div
            className="cc-gallery-desc"
            title={isEditingOwn ? t("gallery.custom.currentDesc") : t("gallery.custom.desc")}
          >
            {isEditingOwn ? t("gallery.custom.currentDesc") : t("gallery.custom.desc")}
          </div>
        </div>

        <div className="cc-gallery-foot is-end">
          <Button
            variant="secondary"
            onClick={openOwnDialog}
          >
            {isEditingOwn ? (
              <>
                <NotePencil size={13} weight="bold" aria-hidden="true" />
                <span>{t("gallery.custom.edit")}</span>
              </>
            ) : (
              <>
                <Sparkle size={13} weight="bold" aria-hidden="true" />
                <span>{t("gallery.custom.open")}</span>
              </>
            )}
          </Button>
        </div>
      </div>
    </article>
  );

  return (
    <div className="cc-route">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("gallery.title")}</h1>
          <p className="cc-page-subtitle">{t("gallery.subtitle")}</p>
        </div>
      </div>

      <section className="cc-gallery-cluster-section">
          <div className="cc-section-head">
            <span className="cc-section-label">{t("gallery.clustersHeading")}</span>
            <span className="cc-field-help">{t("gallery.clustersSub")}</span>
          </div>

          <div className="cc-gallery-clusters">
            {isEditingOwn && customCard}
            {CLUSTER_PRESETS.map((preset) => {
              const boxes = buildPresetClusters(preset.presetId);
              const isAdded = addedKey === `p:${preset.presetId}`;
              return (
                <article key={preset.presetId} className="cc-gallery-card">
                  <div className="cc-gallery-stage">
                    {plateUrl && (
                      <div
                        className="cc-preview-bg"
                        aria-hidden="true"
                        style={{ backgroundImage: `url("${plateUrl}")` }}
                      />
                    )}
                    <div className="cc-stage-scrim" aria-hidden="true" />

                    <div className="cc-trio-stack">
                      {boxes.map((box, idx) => (
                        <WidgetPreview
                          key={`${preset.presetId}-${idx}`}
                          id={`preview-${preset.presetId}-${idx}`}
                          kind="cluster"
                          size="m"
                          cluster={box}
                          appearance={config.appearance}
                          monitors={monitors}
                          locations={config.weather.locations}
                          maxWidth={256}
                          maxHeight={boxes.length > 1 ? 58 : 184}
                          interactive
                          previewShowAll
                        />
                      ))}
                    </div>
                  </div>

                  <div className="cc-gallery-body">
                    <div className="cc-gallery-meta">
                      <div className="cc-gallery-title">
                        {t(`cluster.presets.${preset.presetId}`)}
                      </div>
                      <div
                        className="cc-gallery-desc"
                        title={t(`gallery.presetDesc.${preset.presetId}`)}
                      >
                        {t(`gallery.presetDesc.${preset.presetId}`)}
                      </div>
                    </div>

                    <div className="cc-gallery-foot">
                      <Button
                        variant="secondary"
                        onClick={() =>
                          openCustomizeDialog({
                            ...seedFromPreset(preset.presetId),
                            editingWidgetId: null,
                            baseCluster: null,
                          })
                        }
                      >
                        <span>{t("gallery.customize")}</span>
                      </Button>
                      <MonitorAddButton
                        label={t("gallery.placeOnDesktop")}
                        added={isAdded}
                        monitors={monitors}
                        onAdd={(mon) => void handleAddPreset(preset.presetId, mon)}
                      />
                    </div>
                  </div>
                </article>
              );
            })}
            {!isEditingOwn && customCard}
          </div>
      </section>

      {customSeed && (
        <CustomizeDialog
          key={customSeed.seq}
          seed={customSeed}
          open
          monitors={monitors}
          locations={config.weather.locations}
          appearance={config.appearance}
          plateUrl={plateUrl}
          onClose={() => setCustomSeed(null)}
          onAdd={(cluster, mon) => void handleAddCustom(cluster, mon)}
          onSave={handleSaveExisting}
        />
      )}
    </div>
  );
}

function MonitorAddButton({
  label,
  added,
  monitors,
  onAdd,
}: {
  label: string;
  added: boolean;
  monitors: ReturnType<typeof useControlStore.getState>["monitors"];
  onAdd: (monitorName?: string) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  const sortedMonitors = useMemo(() => [...monitors], [monitors]);
  const variant = added ? "primary" : "secondary";

  if (sortedMonitors.length <= 1) {
    return (
      <Button variant={variant} onClick={() => onAdd()}>
        {added ? (
          <Check size={13} weight="bold" aria-hidden="true" />
        ) : (
          <Plus size={13} weight="bold" aria-hidden="true" />
        )}
        <span>{added ? t("gallery.added") : label}</span>
      </Button>
    );
  }

  return (
    <div ref={wrapRef} className={`cc-split${added ? " is-primary" : ""}`}>
      <Button variant={variant} onClick={() => onAdd()}>
        {added ? (
          <Check size={13} weight="bold" aria-hidden="true" />
        ) : (
          <Plus size={13} weight="bold" aria-hidden="true" />
        )}
        <span>{added ? t("gallery.added") : label}</span>
      </Button>
      <button
        type="button"
        className="cc-split-caret"
        aria-label={t("desktop.monitorLabelField")}
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((prev) => !prev)}
      >
        <CaretDown size={12} weight="bold" aria-hidden="true" />
      </button>

      {menuOpen && (
        <div className="cc-split-menu">
          {sortedMonitors.map((m, idx) => (
            <button
              key={m.name}
              type="button"
              className="cc-split-opt"
              onClick={() => {
                setMenuOpen(false);
                onAdd(m.name);
              }}
            >
              {m.isPrimary
                ? t("gallery.monitorPrimary", { n: idx + 1 })
                : t("gallery.monitorNth", { n: idx + 1 })}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function CustomizeDialog({
  seed,
  open,
  monitors,
  locations,
  appearance,
  plateUrl,
  onClose,
  onAdd,
  onSave,
}: {
  seed: CustomDialogSeed;
  open: boolean;
  monitors: MonitorInfo[];
  locations: Location[];
  appearance: Appearance;
  plateUrl: string;
  onClose: () => void;
  onAdd: (cluster: ClusterConfig, monitor?: string) => void;
  onSave: (widgetId: string, cluster: ClusterConfig) => Promise<boolean>;
}) {
  const [shape, setShape] = useState<CustomShape>(seed.shape);
  const [selected, setSelected] = useState<FreeWidgetKind[]>(seed.selected);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, onClose]);

  const cluster = useMemo(
    () =>
      seed.baseCluster
        ? updateExistingCluster(seed.baseCluster, shape, selected)
        : buildCustomCluster(shape, selected),
    [seed.baseCluster, shape, selected],
  );

  if (!open) return null;

  const isEditing = seed.editingWidgetId !== null;
  const isDirty =
    shape !== seed.shape ||
    selected.length !== seed.selected.length ||
    selected.some((kind, i) => seed.selected[i] !== kind);
  const heading = isEditing ? t("gallery.custom.editHeading") : t("gallery.custom.heading");

  const handleSave = async () => {
    if (!seed.editingWidgetId || saving) return;
    setSaving(true);
    const ok = await onSave(seed.editingWidgetId, cluster);
    if (ok) {
      onClose();
    } else {
      setSaving(false);
    }
  };

  const toggleChoice = (kinds: FreeWidgetKind[]) => {
    setSelected((prev) => {
      const anyOn = kinds.some((kind) => prev.includes(kind));
      if (anyOn) return prev.filter((kind) => !kinds.includes(kind));
      const next = [...prev, ...kinds.filter((kind) => !prev.includes(kind))];
      return next.length <= MAX_CUSTOM_MODULES ? next : prev;
    });
  };

  return createPortal(
    <div className="cc-backdrop" onClick={onClose}>
      <div
        className="cc-custom"
        role="dialog"
        aria-modal="true"
        aria-label={heading}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="cc-custom-head">
          <h3>{heading}</h3>
          <button
            type="button"
            className="cc-iconbtn"
            aria-label={t("window.close")}
            onClick={onClose}
          >
            <X size={15} aria-hidden="true" />
          </button>
        </div>

        <div className="cc-custom-preview">
          {plateUrl && (
            <div
              className="cc-preview-bg"
              aria-hidden="true"
              style={{ backgroundImage: `url("${plateUrl}")` }}
            />
          )}
          <div className="cc-stage-scrim" aria-hidden="true" />
          {cluster.modules.length > 0 ? (
            <WidgetPreview
              id="preview-custom"
              kind="cluster"
              size="m"
              cluster={cluster}
              appearance={appearance}
              monitors={monitors}
              locations={locations}
              maxWidth={320}
              maxHeight={196}
              interactive
              previewShowAll
            />
          ) : (
            <span className="cc-custom-empty">{t("gallery.custom.empty")}</span>
          )}
        </div>

        <div className="cc-custom-section">
          <div className="cc-custom-section-head">
            <span className="cc-field-label">{t("gallery.custom.modules")}</span>
            <span className="cc-field-help cc-mono">
              {selected.length} / {MAX_CUSTOM_MODULES}
            </span>
          </div>
          <div className="cc-modchips" role="group" aria-label={t("gallery.custom.modules")}>
            {CUSTOM_MODULE_CHOICES.map(({ id, kinds, icon: ModuleIcon }) => {
              const isOn = kinds.some((kind) => selected.includes(kind));
              const missing = kinds.filter((kind) => !selected.includes(kind)).length;
              const isFull = !isOn && selected.length + missing > MAX_CUSTOM_MODULES;
              return (
                <button
                  key={id}
                  type="button"
                  className={`cc-modchip${isOn ? " is-on" : ""}`}
                  aria-pressed={isOn}
                  disabled={isFull}
                  onClick={() => toggleChoice(kinds)}
                >
                  <ModuleIcon size={15} weight={isOn ? "fill" : "regular"} aria-hidden="true" />
                  <span>{t(`widgets.${id}.name`)}</span>
                </button>
              );
            })}
          </div>
        </div>

        <div className="cc-custom-section">
          <span className="cc-field-label">{t("gallery.custom.shape")}</span>
          <Segmented<CustomShape>
            fullWidth
            value={shape}
            ariaLabel={t("gallery.custom.shape")}
            options={[
              { value: "box", label: t("gallery.custom.box") },
              { value: "pill", label: t("gallery.custom.pill") },
            ]}
            onChange={setShape}
          />
        </div>

        <div className="cc-custom-foot">
          <Button variant="ghost" onClick={onClose}>
            {t("gallery.custom.cancel")}
          </Button>
          {isEditing ? (
            <Button
              variant="primary"
              disabled={selected.length === 0 || !isDirty || saving}
              onClick={() => void handleSave()}
            >
              {t("gallery.custom.save")}
            </Button>
          ) : selected.length === 0 ? (
            <Button variant="primary" disabled>
              {t("gallery.placeOnDesktop")}
            </Button>
          ) : (
            <MonitorAddButton
              label={t("gallery.placeOnDesktop")}
              added={false}
              monitors={monitors}
              onAdd={(mon) => {
                onClose();
                onAdd(cluster, mon);
              }}
            />
          )}
        </div>
      </div>
    </div>,
    getOverlayPortalRoot(),
  );
}
