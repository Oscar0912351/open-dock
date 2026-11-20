import { CaretDown, Check, Plus } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { ClusterModule } from "../../shared/bindings/ClusterModule";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { CLUSTER_PRESETS, WIDGET_LIST, WIDGET_REGISTRY, type FreeWidgetKind } from "../../widget/registry";
import { Button } from "../components/Button";
import { Segmented } from "../components/Segmented";
import { WidgetPreview } from "../components/WidgetPreview";
import { useControlStore } from "../store";

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
          </div>
        </section>
      )}

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
