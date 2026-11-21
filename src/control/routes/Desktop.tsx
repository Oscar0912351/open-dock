import {
  Clock,
  CloudSun,
  Copy,
  Cpu,
  MusicNote,
  Note,
  NotePencil,
  Plus,
  Robot,
  SquaresFour,
  Headset,
  X,
} from "@phosphor-icons/react";
import type { WidgetKind } from "../../shared/bindings/WidgetKind";
import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Inspector } from "../components/Inspector";
import { Kbd } from "../components/Kbd";
import { MonitorMap } from "../components/MonitorMap";
import { formatWidgetTitle, useControlStore } from "../store";

const KIND_ICONS: Record<WidgetKind, typeof Clock> = {
  clock: Clock,
  nowPlaying: MusicNote,
  system: Cpu,
  weather: CloudSun,
  note: Note,
  discord: Headset,
  cluster: SquaresFour,
  agents: Robot,
};

export function Desktop() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const placements = useControlStore((s) => s.placements);
  const selectedId = useControlStore((s) => s.selectedWidgetId);
  const setSelectedWidgetId = useControlStore((s) => s.setSelectedWidgetId);
  const setRoute = useControlStore((s) => s.setRoute);
  const editMode = useControlStore((s) => s.editMode);

  if (!config) return null;

  const count = config.widgets.length;
  const countLabel =
    count === 1 ? t("desktop.widgetCountOne") : t("desktop.widgetCount", { n: count });

  return (
    <div className="cc-desktop-studio">
      <div className="cc-desktop-workspace">
        <div className="cc-desktop-header">
          <div className="cc-desktop-title-wrap">
            <h1 className="cc-page-title">{t("desktop.title")}</h1>
            <span className="cc-desktop-count">{countLabel}</span>
          </div>

          <div className="cc-desktop-toolbar-actions">
            <Button
              variant={editMode ? "primary" : "secondary"}
              onClick={() => void ipc.editSet(!editMode)}
            >
              <NotePencil size={14} aria-hidden="true" />
              <span>
                {editMode ? t("desktop.stopEditing") : t("desktop.editOnDesktop")}
              </span>
              {!editMode && <Kbd keys={config.general.hotkeys.editMode} />}
            </Button>

            <Button variant="secondary" onClick={() => setRoute("gallery")}>
              <Plus size={13} weight="bold" aria-hidden="true" />
              <span>{t("desktop.addWidget")}</span>
            </Button>
          </div>
        </div>

        <MonitorMap
          monitors={monitors}
          widgets={config.widgets}
          placements={placements}
          selectedId={selectedId}
          onSelect={setSelectedWidgetId}
        />

        {count === 0 ? (
          <div className="cc-empty">
            <div className="cc-empty-illus" aria-hidden="true">
              <div className="cc-empty-box" />
              <div className="cc-empty-box is-tall" />
              <div className="cc-empty-box" />
            </div>
            <h3>{t("desktop.emptyTitle")}</h3>
            <p>{t("desktop.emptyBody")}</p>
            <Button variant="primary" onClick={() => setRoute("gallery")}>
              {t("desktop.goGallery")}
            </Button>
          </div>
        ) : (
          <section className="cc-section">
            <span className="cc-section-label">{t("desktop.onDesktop")}</span>
            <div className="cc-widget-list">
              {config.widgets.map((w) => {
                const Icon = KIND_ICONS[w.kind];
                const monIdx = monitors.findIndex((m) => m.name === w.placement.monitor.name);
                const monLabel = t("gallery.monitorNth", { n: monIdx >= 0 ? monIdx + 1 : 1 });
                const sizeText =
                  w.kind === "cluster" && w.cluster
                    ? `${w.cluster.width} px`
                    : t(`size.${w.size}`);
                const sub = `${sizeText} · ${monLabel} · ${t(`layer.${w.layer}`)}`;
                const isSelected = selectedId === w.id;

                return (
                  <div
                    key={w.id}
                    className={`cc-widget-row${isSelected ? " is-selected" : ""}`}
                    onClick={() => setSelectedWidgetId(w.id)}
                  >
                    <div className="cc-widget-row-ic">
                      <Icon size={15} aria-hidden="true" />
                    </div>
                    <div className="cc-widget-row-meta">
                      <span className="cc-widget-row-title">{formatWidgetTitle(w)}</span>
                      <span className="cc-widget-row-sub">{sub}</span>
                    </div>
                    <div
                      style={{ display: "flex", gap: 4 }}
                      onClick={(e) => e.stopPropagation()}
                    >
                      <button
                        type="button"
                        className="cc-iconbtn"
                        aria-label={t("desktop.duplicate")}
                        title={t("desktop.duplicate")}
                        onClick={() => {
                          void ipc.widgetDuplicate(w.id).then((copy) => setSelectedWidgetId(copy.id));
                        }}
                      >
                        <Copy size={14} aria-hidden="true" />
                      </button>
                      <button
                        type="button"
                        className="cc-iconbtn is-danger"
                        aria-label={t("desktop.remove")}
                        title={t("desktop.remove")}
                        onClick={() => void ipc.widgetRemove(w.id)}
                      >
                        <X size={14} aria-hidden="true" />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
        )}
      </div>

      <Inspector />
    </div>
  );
}
