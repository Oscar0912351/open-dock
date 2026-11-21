import { Clock, CloudSun, Cpu, MusicNote, Note, Robot, SquaresFour, Headset } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import type { MonitorInfo } from "../../shared/bindings/MonitorInfo";
import type { WidgetInstance } from "../../shared/bindings/WidgetInstance";
import type { WidgetKind } from "../../shared/bindings/WidgetKind";
import type { WidgetPlacement } from "../../shared/bindings/WidgetPlacement";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { t } from "../../shared/i18n";
import { ipc, toAssetUrl } from "../../shared/ipc";
import { computeClusterTargetHeight, DEFAULT_CLUSTER_SIGNALS, resolveModuleModes } from "../../widget/cluster/resolveLayout";

const BASE_DIMS: Record<WidgetSize, { w: number; h: number }> = {
  pill: { w: 204, h: 48 },
  s: { w: 132, h: 132 },
  m: { w: 276, h: 132 },
  l: { w: 276, h: 276 },
};

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

const KIND_LABELS: Record<WidgetKind, string> = {
  clock: "widgets.clock.name",
  nowPlaying: "widgets.nowPlaying.name",
  system: "widgets.system.name",
  weather: "widgets.weather.name",
  note: "widgets.note.name",
  discord: "widgets.discord.name",
  cluster: "widgets.cluster.name",
  agents: "widgets.agents.name",
};

function findTargetMonitor(w: WidgetInstance, monitors: MonitorInfo[]): MonitorInfo | null {
  return (
    monitors.find(
      (m) =>
        m.name === w.placement.monitor.name &&
        m.rect.width === w.placement.monitor.rect.width &&
        m.rect.height === w.placement.monitor.rect.height,
    ) ??
    monitors.find((m) => m.name === w.placement.monitor.name) ??
    monitors.find(
      (m) =>
        m.rect.x === w.placement.monitor.rect.x &&
        m.rect.y === w.placement.monitor.rect.y &&
        m.rect.width === w.placement.monitor.rect.width &&
        m.rect.height === w.placement.monitor.rect.height,
    ) ??
    monitors.find((m) => m.isPrimary) ??
    monitors[0] ??
    null
  );
}

function getWidgetLogicalBase(
  w: WidgetInstance,
  livePlacement?: WidgetPlacement | null,
): { w: number; h: number } {
  if (w.kind === "cluster" && w.cluster) {
    if (livePlacement && w.scale > 0) {
      return {
        w: w.cluster.width,
        h: Math.max(48, livePlacement.rect.height / w.scale),
      };
    }
    const idleModes = resolveModuleModes(
      w.cluster.modules,
      DEFAULT_CLUSTER_SIGNALS,
      null,
      w.cluster.resizeBehavior,
      w.cluster.presetId === "pillBar",
    );
    return {
      w: w.cluster.width,
      h: computeClusterTargetHeight(
        w.cluster.modules,
        idleModes,
        w.cluster.presetId,
        w.cluster.resizeBehavior,
        w.cluster.maxHeight,
      ),
    };
  }
  return BASE_DIMS[w.size];
}

interface MonitorMapProps {
  monitors: MonitorInfo[];
  widgets: WidgetInstance[];
  placements: Record<string, WidgetPlacement>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  interactive?: boolean;
}

export function MonitorMap({
  monitors,
  widgets,
  placements,
  selectedId,
  onSelect,
  interactive = true,
}: MonitorMapProps) {
  const [dragState, setDragState] = useState<{
    id: string;
    dx: number;
    dy: number;
  } | null>(null);
  const [cardWidth, setCardWidth] = useState(500);

  const cardRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const activeDragRef = useRef<{
    id: string;
    startClientX: number;
    startClientY: number;
    startPhysX: number;
    startPhysY: number;
    physW: number;
    physH: number;
    monScale: number;
    moved: boolean;
  } | null>(null);

  useEffect(() => {
    const el = cardRef.current;
    if (!el) return;
    const sync = () => {
      if (el.clientWidth > 0) setCardWidth(el.clientWidth);
    };
    sync();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(() => {
    if (monitors.length === 0) return null;
    let minX = Number.POSITIVE_INFINITY;
    let minY = Number.POSITIVE_INFINITY;
    let maxX = Number.NEGATIVE_INFINITY;
    let maxY = Number.NEGATIVE_INFINITY;
    for (const m of monitors) {
      minX = Math.min(minX, m.rect.x);
      minY = Math.min(minY, m.rect.y);
      maxX = Math.max(maxX, m.rect.x + m.rect.width);
      maxY = Math.max(maxY, m.rect.y + m.rect.height);
    }
    const totalW = Math.max(1, maxX - minX);
    const totalH = Math.max(1, maxY - minY);
    const maxCanvasW = Math.min(620, Math.max(240, cardWidth - 36));
    const s = Math.min(maxCanvasW / totalW, 232 / totalH);
    return {
      minX,
      minY,
      s,
      canvasW: Math.round(totalW * s),
      canvasH: Math.round(totalH * s),
    };
  }, [monitors, cardWidth]);

  if (!layout) return null;

  const handleWidgetPointerDown = (
    event: PointerEvent<HTMLButtonElement>,
    w: WidgetInstance,
    physX: number,
    physY: number,
    physW: number,
    physH: number,
    monScale: number,
  ) => {
    if (!interactive || event.button !== 0) return;
    event.stopPropagation();
    onSelect(w.id);
    event.currentTarget.setPointerCapture(event.pointerId);
    activeDragRef.current = {
      id: w.id,
      startClientX: event.clientX,
      startClientY: event.clientY,
      startPhysX: physX,
      startPhysY: physY,
      physW,
      physH,
      monScale,
      moved: false,
    };
  };

  const handleWidgetPointerMove = (event: PointerEvent<HTMLButtonElement>) => {
    const active = activeDragRef.current;
    if (!active) return;
    const dx = event.clientX - active.startClientX;
    const dy = event.clientY - active.startClientY;
    if (!active.moved && Math.hypot(dx, dy) < 3) return;
    active.moved = true;
    setDragState({ id: active.id, dx, dy });
  };

  const finishDrag = (event: PointerEvent<HTMLButtonElement>, cancel: boolean) => {
    const active = activeDragRef.current;
    activeDragRef.current = null;
    if (!active) return;
    try {
      event.currentTarget.releasePointerCapture(event.pointerId);
    } catch {
      // Ignore if capture already released.
    }
    setDragState(null);
    if (cancel || !active.moved) return;

    const dxCanvas = event.clientX - active.startClientX;
    const dyCanvas = event.clientY - active.startClientY;
    const nextPhysX = active.startPhysX + dxCanvas / layout.s;
    const nextPhysY = active.startPhysY + dyCanvas / layout.s;
    const centerPhysX = nextPhysX + active.physW / 2;
    const centerPhysY = nextPhysY + active.physH / 2;

    const dropMon =
      monitors.find(
        (m) =>
          centerPhysX >= m.rect.x &&
          centerPhysX < m.rect.x + m.rect.width &&
          centerPhysY >= m.rect.y &&
          centerPhysY < m.rect.y + m.rect.height,
      ) ??
      monitors.find((m) => m.isPrimary) ??
      monitors[0];
    if (!dropMon) return;

    const scale = dropMon.scale > 0 ? dropMon.scale : 1;
    const logX = (nextPhysX - dropMon.workArea.x) / scale;
    const logY = (nextPhysY - dropMon.workArea.y) / scale;
    void ipc.widgetMove(active.id, dropMon.name, logX, logY);
  };

  return (
    <div ref={cardRef} className="cc-map-card">
      <div
        ref={canvasRef}
        className="cc-map-canvas"
        style={{ width: `${layout.canvasW}px`, height: `${layout.canvasH}px` }}
      >
        {monitors.map((m) => (
          <div
            key={m.name}
            className="cc-map-mon"
            style={{
              left: `${(m.rect.x - layout.minX) * layout.s}px`,
              top: `${(m.rect.y - layout.minY) * layout.s}px`,
              width: `${m.rect.width * layout.s}px`,
              height: `${m.rect.height * layout.s}px`,
              backgroundImage: m.platePath ? `url("${toAssetUrl(m.platePath)}")` : undefined,
            }}
          />
        ))}

        {widgets.map((w) => {
          const mon = findTargetMonitor(w, monitors);
          if (!mon) return null;
          const monScale = mon.scale > 0 ? mon.scale : 1;
          const base = getWidgetLogicalBase(w, placements[w.id]);
          const physW = base.w * w.scale * monScale;
          const physH = base.h * w.scale * monScale;
          const physX = mon.workArea.x + w.placement.x * monScale;
          const physY = mon.workArea.y + w.placement.y * monScale;

          const activeDrag = dragState?.id === w.id ? dragState : null;
          const left = (physX - layout.minX) * layout.s + (activeDrag?.dx ?? 0);
          const top = (physY - layout.minY) * layout.s + (activeDrag?.dy ?? 0);
          const width = Math.max(18, physW * layout.s);
          const height = Math.max(14, physH * layout.s);

          const Icon = KIND_ICONS[w.kind];
          const label = t(KIND_LABELS[w.kind]);
          const showLabel = width >= 54 && height >= 24;

          return (
            <button
              key={w.id}
              type="button"
              aria-label={label}
              title={label}
              className={`cc-map-widget${selectedId === w.id ? " is-selected" : ""}`}
              style={{
                left: `${left}px`,
                top: `${top}px`,
                width: `${width}px`,
                height: `${height}px`,
                cursor: interactive ? undefined : "default",
              }}
              onPointerDown={(e) =>
                handleWidgetPointerDown(e, w, physX, physY, physW, physH, monScale)
              }
              onPointerMove={handleWidgetPointerMove}
              onPointerUp={(e) => finishDrag(e, false)}
              onPointerCancel={(e) => finishDrag(e, true)}
            >
              <Icon size={Math.min(14, Math.max(10, height * 0.45))} aria-hidden="true" />
              {showLabel && <span className="cc-map-widget-name">{label}</span>}
            </button>
          );
        })}
      </div>

      <div className="cc-map-captions">
        {monitors.map((m, idx) => (
          <span key={m.name} className="cc-map-caption">
            {t("desktop.monitorLabel", {
              n: idx + 1,
              w: m.rect.width,
              h: m.rect.height,
              scale: Math.round(m.scale * 100),
            })}
          </span>
        ))}
      </div>
    </div>
  );
}
