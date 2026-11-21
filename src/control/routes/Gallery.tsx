import {
  CaretDown,
  Check,
  Clock,
  CloudSun,
  DiscordLogo,
  Gauge,
  MusicNote,
  NotePencil,
  Plus,
  Sparkle,
  X,
  type Icon,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Appearance } from "../../shared/bindings/Appearance";
import type { AutoRule } from "../../shared/bindings/AutoRule";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { Location } from "../../shared/bindings/Location";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { CLUSTER_PRESETS, WIDGET_LIST, WIDGET_REGISTRY, type FreeWidgetKind } from "../../widget/registry";
import { Button } from "../components/Button";
import { Segmented } from "../components/Segmented";
import { WidgetPreview } from "../components/WidgetPreview";
import { useControlStore } from "../store";

type MonitorInfo = ReturnType<typeof useControlStore.getState>["monitors"][number];

const CUSTOM_MODULE_CHOICES: ReadonlyArray<{ kind: FreeWidgetKind; icon: Icon }> = [
  { kind: "clock", icon: Clock },
  { kind: "weather", icon: CloudSun },
  { kind: "system", icon: Gauge },
  { kind: "nowPlaying", icon: MusicNote },
  { kind: "note", icon: NotePencil },
  { kind: "discord", icon: DiscordLogo },
];

const MAX_CUSTOM_MODULES = 5;

type CustomShape = "box" | "pill";

function customAutoRule(kind: FreeWidgetKind, shape: CustomShape): AutoRule {
  if (kind === "nowPlaying") return "expandWhenPlaying";
  if (shape === "box" && kind === "system") return "expandWhenHighLoad";
  if (shape === "box" && kind === "weather") return "fillRemainingSpace";
  return "none";
}

function buildCustomCluster(shape: CustomShape, selected: FreeWidgetKind[]): ClusterConfig {
  return {
    presetId: shape === "pill" ? "pillBar" : null,
    width: 264,
    maxHeight: shape === "pill" ? 240 : 320,
    resizeBehavior: "adaptive",
    anchor: "top",
    modules: selected.map((kind) => ({
      id: crypto.randomUUID(),
      kind,
      defaultMode: kind === "nowPlaying" ? "hidden" : "compact",
      autoRule: customAutoRule(kind, shape),
      settings: { ...WIDGET_REGISTRY[kind].defaultSettings },
    })),
  };
}

type GalleryFilter =
  | "all"
  | "clusters"
  | "music"
  | "system"
  | "time"
  | "weather"
  | "productivity";

const KIND_CATEGORY: Record<FreeWidgetKind, GalleryFilter> = {
  nowPlaying: "music",
  system: "system",
  clock: "time",
  weather: "weather",
  note: "productivity",
  discord: "productivity",
};

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
          mod("aio", "discord", "compact", "none"),
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

export function Gallery() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const setRoute = useControlStore((s) => s.setRoute);
  const pushToast = useControlStore((s) => s.pushToast);

  const [filter, setFilter] = useState<GalleryFilter>("all");
  const [selectedSizes, setSelectedSizes] = useState<Partial<Record<FreeWidgetKind, WidgetSize>>>({});
  const [addedKey, setAddedKey] = useState<string | null>(null);
  const [customOpen, setCustomOpen] = useState(false);
  const addedTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (addedTimerRef.current !== null) window.clearTimeout(addedTimerRef.current);
    };
  }, []);

  if (!config) return null;

  const markAdded = (key: string) => {
    setAddedKey(key);
    if (addedTimerRef.current !== null) window.clearTimeout(addedTimerRef.current);
    addedTimerRef.current = window.setTimeout(() => {
      addedTimerRef.current = null;
      setAddedKey(null);
    }, 1600);
  };

  const handleAddSingle = async (kind: FreeWidgetKind, size: WidgetSize, monitorName?: string) => {
    const inst = await ipc.widgetAdd(kind, size, monitorName ?? null);
    markAdded(`w:${kind}`);
    const name = t(`widgets.${kind}.name`);
    pushToast({
      message: t("toast.added", { name }),
      actionLabel: t("toast.show"),
      onAction: () => setRoute("desktop", inst.id),
    });
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

  const showClusters = filter === "all" || filter === "clusters";
  const singleWidgets =
    filter === "clusters"
      ? []
      : filter === "all"
        ? WIDGET_LIST
        : WIDGET_LIST.filter((meta) => KIND_CATEGORY[meta.kind] === filter);

  const primaryMonitor = monitors.find((m) => m.isPrimary) ?? monitors[0] ?? null;
  const plateUrl = toAssetUrl(primaryMonitor?.platePath ?? "");

  return (
    <div className="cc-route">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("gallery.title")}</h1>
          <p className="cc-page-subtitle">{t("gallery.subtitle")}</p>
        </div>
      </div>

      <div>
        <Segmented<GalleryFilter>
          value={filter}
          ariaLabel={t("gallery.title")}
          options={[
            { value: "all", label: t("gallery.filters.all") },
            { value: "clusters", label: t("gallery.filters.clusters") },
            { value: "music", label: t("gallery.filters.music") },
            { value: "system", label: t("gallery.filters.system") },
            { value: "time", label: t("gallery.filters.time") },
            { value: "weather", label: t("gallery.filters.weather") },
            { value: "productivity", label: t("gallery.filters.productivity") },
          ]}
          onChange={setFilter}
        />
      </div>

      {showClusters && (
        <section className="cc-gallery-cluster-section">
          <div className="cc-section-head">
            <span className="cc-section-label">{t("gallery.clustersHeading")}</span>
            <span className="cc-field-help">{t("gallery.clustersSub")}</span>
          </div>

          <div className="cc-gallery-clusters">
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

                    <div className="cc-gallery-foot" style={{ justifyContent: "flex-end" }}>
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
            <article className="cc-gallery-card">
              <button
                type="button"
                className="cc-gallery-stage cc-custom-cta"
                onClick={() => setCustomOpen(true)}
              >
                {plateUrl && (
                  <div
                    className="cc-preview-bg"
                    aria-hidden="true"
                    style={{ backgroundImage: `url("${plateUrl}")` }}
                  />
                )}
                <div className="cc-stage-scrim" aria-hidden="true" />
                <span className="cc-custom-cta-mark" aria-hidden="true">
                  <Sparkle size={22} weight="duotone" />
                </span>
              </button>

              <div className="cc-gallery-body">
                <div className="cc-gallery-meta">
                  <div className="cc-gallery-title">{t("gallery.custom.title")}</div>
                  <div className="cc-gallery-desc" title={t("gallery.custom.desc")}>
                    {t("gallery.custom.desc")}
                  </div>
                </div>

                <div className="cc-gallery-foot" style={{ justifyContent: "flex-end" }}>
                  <Button variant="secondary" onClick={() => setCustomOpen(true)}>
                    <Sparkle size={13} weight="bold" aria-hidden="true" />
                    <span>{t("gallery.custom.open")}</span>
                  </Button>
                </div>
              </div>
            </article>
          </div>
        </section>
      )}

      <CustomizeDialog
        open={customOpen}
        monitors={monitors}
        locations={config.weather.locations}
        appearance={config.appearance}
        plateUrl={plateUrl}
        onClose={() => setCustomOpen(false)}
        onAdd={(cluster, mon) => void handleAddCustom(cluster, mon)}
      />

      {singleWidgets.length > 0 && (
        <section className="cc-gallery-cluster-section">
          {showClusters && (
            <div className="cc-section-head">
              <span className="cc-section-label">{t("gallery.singleHeading")}</span>
            </div>
          )}
          <div className="cc-gallery-grid">
            {singleWidgets.map((meta) => {
              const currentSize = selectedSizes[meta.kind] ?? meta.defaultSize;
              const isAdded = addedKey === `w:${meta.kind}`;
              return (
                <article key={meta.kind} className="cc-gallery-card">
                  <WidgetPreview
                    id={`preview-card-${meta.kind}`}
                    kind={meta.kind}
                    size={currentSize}
                    settings={meta.defaultSettings}
                    appearance={config.appearance}
                    monitors={monitors}
                    locations={config.weather.locations}
                    maxWidth={248}
                    maxHeight={144}
                    stageHeight={168}
                    retriggerKey={`${meta.kind}-${currentSize}`}
                  />

                  <div className="cc-gallery-body">
                    <div className="cc-gallery-meta">
                      <div className="cc-gallery-title">{t(`widgets.${meta.kind}.name`)}</div>
                      <div className="cc-gallery-desc" title={t(`widgets.${meta.kind}.desc`)}>
                        {t(`widgets.${meta.kind}.desc`)}
                      </div>
                    </div>

                    <div className="cc-gallery-foot">
                      <div className="cc-size-chips">
                        {meta.allowedSizes.map((size) => (
                          <button
                            key={size}
                            type="button"
                            className={`cc-chip${currentSize === size ? " is-active" : ""}`}
                            onClick={() =>
                              setSelectedSizes((prev) => ({ ...prev, [meta.kind]: size }))
                            }
                          >
                            {t(`size.${size}`)}
                          </button>
                        ))}
                      </div>

                      <MonitorAddButton
                        label={t("gallery.add")}
                        added={isAdded}
                        monitors={monitors}
                        onAdd={(mon) => void handleAddSingle(meta.kind, currentSize, mon)}
                      />
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        </section>
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
  open,
  monitors,
  locations,
  appearance,
  plateUrl,
  onClose,
  onAdd,
}: {
  open: boolean;
  monitors: MonitorInfo[];
  locations: Location[];
  appearance: Appearance;
  plateUrl: string;
  onClose: () => void;
  onAdd: (cluster: ClusterConfig, monitor?: string) => void;
}) {
  const [shape, setShape] = useState<CustomShape>("box");
  const [selected, setSelected] = useState<FreeWidgetKind[]>([
    "clock",
    "weather",
    "system",
  ]);

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
    () => buildCustomCluster(shape, selected),
    [shape, selected],
  );

  if (!open) return null;

  const toggleModule = (kind: FreeWidgetKind) => {
    setSelected((prev) =>
      prev.includes(kind)
        ? prev.filter((item) => item !== kind)
        : prev.length >= MAX_CUSTOM_MODULES
          ? prev
          : [...prev, kind],
    );
  };

  return (
    <div className="cc-backdrop" onClick={onClose}>
      <div
        className="cc-custom"
        role="dialog"
        aria-modal="true"
        aria-label={t("gallery.custom.heading")}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="cc-custom-head">
          <h3>{t("gallery.custom.heading")}</h3>
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
            {CUSTOM_MODULE_CHOICES.map(({ kind, icon: ModuleIcon }) => {
              const isOn = selected.includes(kind);
              const isFull = !isOn && selected.length >= MAX_CUSTOM_MODULES;
              return (
                <button
                  key={kind}
                  type="button"
                  className={`cc-modchip${isOn ? " is-on" : ""}`}
                  aria-pressed={isOn}
                  disabled={isFull}
                  onClick={() => toggleModule(kind)}
                >
                  <ModuleIcon size={15} weight={isOn ? "fill" : "regular"} aria-hidden="true" />
                  <span>{t(`widgets.${kind}.name`)}</span>
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
          {selected.length === 0 ? (
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
    </div>
  );
}
