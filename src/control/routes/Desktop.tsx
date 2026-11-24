import {
  Clock,
  CloudSun,
  Cpu,
  Headset,
  MusicNote,
  Note,
  NotePencil,
  Plus,
  Robot,
  SlidersHorizontal,
  SquaresFour,
  X,
} from "@phosphor-icons/react";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import type { WidgetKind } from "../../shared/bindings/WidgetKind";
import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Inspector } from "../components/Inspector";
import { formatWidgetTitle, useControlStore } from "../store";
import { Themes } from "./Themes";

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

function formatWidgetSummary(
  w: WidgetInstance,
  monitors: ReturnType<typeof useControlStore.getState>["monitors"],
): string {
  const parts: string[] = [];

  if (w.kind === "cluster" && w.cluster) {
    const names: string[] = [];
    for (const m of w.cluster.modules) {
      if (m.kind === "cluster") continue;
      const label = t(`widgets.${m.kind}.name`);
      if (!names.includes(label)) names.push(label);
    }
    if (names.length <= 3) {
      parts.push(names.join(" · "));
    } else {
      parts.push(`${names.slice(0, 2).join(" · ")} +${names.length - 2}`);
    }
  } else {
    parts.push(t(`size.${w.size}`));
  }

  if (monitors.length > 1) {
    const monIdx = monitors.findIndex((m) => m.name === w.placement.monitor.name);
    if (monIdx >= 0) {
      parts.push(t("gallery.monitorNth", { n: monIdx + 1 }));
    }
  }

  if (w.layer === "top") {
    parts.push(t("desktop.badgeTop"));
  }
  if (w.clickThrough) {
    parts.push(t("desktop.badgeClickThrough"));
  }

  return parts.join(" · ");
}

export function Desktop() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const monitors = useControlStore((s) => s.monitors);
  const selectedId = useControlStore((s) => s.selectedWidgetId);
  const setSelectedWidgetId = useControlStore((s) => s.setSelectedWidgetId);
  const setRoute = useControlStore((s) => s.setRoute);
  const editMode = useControlStore((s) => s.editMode);

  if (!config) return null;

  const count = config.widgets.length;
  const countLabel =
    count === 1 ? t("desktop.widgetCountOne") : t("desktop.widgetCount", { n: count });

  return (
    <div className="cc-route">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("desktop.title")}</h1>
          <p className="cc-page-subtitle">{t("desktop.subtitle")}</p>
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
          </Button>

          <Button variant="primary" onClick={() => setRoute("gallery")}>
            <Plus size={13} weight="bold" aria-hidden="true" />
            <span>{t("desktop.addWidget")}</span>
          </Button>
        </div>
      </div>

      <section className="cc-section">
        <div className="cc-desk-sec-head">
          <span className="cc-section-label">{t("desktop.onDesktop")}</span>
          {count > 0 && <span className="cc-desk-sec-count">{countLabel}</span>}
        </div>

        {count === 0 ? (
          <div className="cc-desk-empty">
            <div className="cc-desk-empty-copy">
              <strong>{t("desktop.emptyTitle")}</strong>
              <span>{t("desktop.emptyBody")}</span>
            </div>
            <Button variant="secondary" onClick={() => setRoute("gallery")}>
              {t("desktop.goGallery")}
            </Button>
          </div>
        ) : (
          <div className="cc-desk-grid">
            {config.widgets.map((w) => {
              const Icon = KIND_ICONS[w.kind];
              const isSelected = selectedId === w.id;
              const summary = formatWidgetSummary(w, monitors);

              return (
                <div
                  key={w.id}
                  role="button"
                  tabIndex={0}
                  className={`cc-desk-card${isSelected ? " is-selected" : ""}`}
                  onClick={() => setSelectedWidgetId(w.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setSelectedWidgetId(w.id);
                    }
                  }}
                >
                  <div className="cc-desk-card-ic" aria-hidden="true">
                    <Icon size={16} />
                  </div>
                  <div className="cc-desk-card-meta">
                    <span className="cc-desk-card-title">{formatWidgetTitle(w)}</span>
                    <span className="cc-desk-card-sub" title={summary}>
                      {summary}
                    </span>
                  </div>
                  <div
                    className="cc-desk-card-actions"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      className="cc-iconbtn"
                      aria-label={t("desktop.customize")}
                      title={t("desktop.customize")}
                      onClick={() => setSelectedWidgetId(w.id)}
                    >
                      <SlidersHorizontal size={14} aria-hidden="true" />
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
        )}
      </section>

      <section className="cc-desk-appearance">
        <span className="cc-section-label">{t("desktop.appearanceHeading")}</span>
        <Themes embedded />
      </section>

      <Inspector />
    </div>
  );
}
