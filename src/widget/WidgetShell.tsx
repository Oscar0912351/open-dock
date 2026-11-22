import { ArrowsOutSimple, X } from "@phosphor-icons/react";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  type CSSProperties,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { t } from "../shared/i18n";
import type { GlassEngine } from "../shared/bindings/GlassEngine";
import type { MotionPreset } from "../shared/bindings/MotionPreset";
import type { RadiusPreset } from "../shared/bindings/RadiusPreset";
import type { ResizeBehavior } from "../shared/bindings/ResizeBehavior";
import type { ThemeId } from "../shared/bindings/ThemeId";
import type { VerticalAnchor } from "../shared/bindings/VerticalAnchor";
import type { WidgetSize } from "../shared/bindings/WidgetSize";
import { GlassPlate } from "./GlassPlate";
import "./WidgetShell.css";

export type ShellSize = WidgetSize;
export type ShellTheme = ThemeId;

const SIZES: Record<WidgetSize, { width: number; height: number; paddingX: number; paddingY: number }> = {
  pill: { width: 204, height: 48, paddingX: 12, paddingY: 8 },
  s: { width: 132, height: 132, paddingX: 14, paddingY: 14 },
  m: { width: 276, height: 132, paddingX: 14, paddingY: 14 },
  l: { width: 276, height: 276, paddingX: 16, paddingY: 16 },
};

const RADII: Record<RadiusPreset, Record<WidgetSize, number>> = {
  soft: { pill: 8, s: 10, m: 10, l: 12 },
  round: { pill: 14, s: 22, m: 22, l: 26 },
  extra: { pill: 24, s: 36, m: 36, l: 42 },
};

const CLUSTER_RADII: Record<RadiusPreset, number> = {
  soft: 10,
  round: 22,
  extra: 34,
};

export function computeScrimAlpha(
  theme: ThemeId,
  luma: number,
  autoContrast: boolean,
): number {
  if (!autoContrast) return 0;
  if (theme === "laerk") {
    return Math.min(0.18, Math.max(0, (0.35 - luma) * 0.5));
  }
  return Math.min(0.28, Math.max(0, (luma - 0.45) * 0.6));
}

export interface GlassPlacementInfo {
  platePath: string;
  monitorLogicalW: number;
  monitorLogicalH: number;
  wx: number;
  wy: number;
  luma: number;
}

export interface ClusterFrameInfo {
  width: number;
  boxHeight: number;
  padY?: number;
  anchor: VerticalAnchor;
  resizeBehavior: ResizeBehavior;
  transitionKind?: "expand" | "collapse" | "idle";
  onHeightTransitionEnd?: () => void;
}

interface WidgetShellProps {
  size: WidgetSize;
  theme: ThemeId;
  engine: GlassEngine;
  radius?: RadiusPreset;
  motion?: MotionPreset;
  glassStrength?: number;
  accent?: string | null;
  autoContrast?: boolean;
  startDelayMs?: number;
  kind: string;
  paused: boolean;
  preview?: boolean;
  ariaLabel?: string;
  ambientColor?: [number, number, number] | null;
  hiddenIdle?: boolean;
  isEditing?: boolean;
  isSelected?: boolean;
  isDragging?: boolean;
  canCycleSize?: boolean;
  onBeginDrag?: () => void;
  onLongPressDrag?: () => void;
  onRemoveWidget?: () => void;
  onCycleSize?: () => void;
  glass?: GlassPlacementInfo | null;
  clusterFrame?: ClusterFrameInfo | null;
  rootRef?: RefObject<HTMLDivElement | null>;
  plateRef?: RefObject<HTMLDivElement | null>;
  onContextMenu?: (event: MouseEvent<HTMLDivElement>) => void;
  children: ReactNode;
}

export function WidgetShell({
  size,
  theme,
  engine,
  radius = "round",
  motion = "full",
  glassStrength = 1.0,
  accent = null,
  autoContrast = true,
  startDelayMs = 0,
  kind,
  paused,
  preview = false,
  ariaLabel,
  ambientColor = null,
  hiddenIdle = false,
  isEditing = false,
  isSelected = false,
  isDragging = false,
  canCycleSize = true,
  onBeginDrag,
  onLongPressDrag,
  onRemoveWidget,
  onCycleSize,
  glass,
  clusterFrame = null,
  rootRef,
  plateRef,
  onContextMenu,
  children,
}: WidgetShellProps) {
  const contentRef = useRef<HTMLDivElement>(null);
  const dimensions = SIZES[size];
  const rBase = clusterFrame ? CLUSTER_RADII[radius] : RADII[radius][size];
  const scrimAlpha = computeScrimAlpha(theme, glass?.luma ?? 0.15, autoContrast);

  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const enterNodes = content.querySelectorAll<HTMLElement>("[data-enter]");
    enterNodes.forEach((node, index) => {
      node.style.setProperty("--i", String(index));
    });
  }, [children, kind, size]);

  const baseW = clusterFrame ? clusterFrame.width : dimensions.width;
  const baseH = clusterFrame ? clusterFrame.boxHeight : dimensions.height;
  const px = clusterFrame ? 12 : dimensions.paddingX;
  const py = clusterFrame ? (clusterFrame.padY ?? 10) : dimensions.paddingY;

  const style: CSSProperties & Record<string, string | number> = {
    "--base-w": baseW,
    "--base-h": baseH,
    "--box-h": baseH,
    "--r-base": rBase,
    "--px": px,
    "--py": py,
    "--gs": Math.min(1.4, Math.max(0.6, glassStrength)),
    "--t0": `${startDelayMs}ms`,
    "--scrim-a": scrimAlpha.toFixed(4),
  };
  if (clusterFrame) {
    style["--cluster-dur"] = clusterFrame.transitionKind === "collapse" ? "280ms" : "340ms";
  }
  if (accent) {
    style["--accent"] = accent;
  }
  if (ambientColor) {
    style["--art-rgb"] = `${ambientColor[0]}, ${ambientColor[1]}, ${ambientColor[2]}`;
  }

  const rootClasses = [
    "widget-root",
    preview ? "is-preview" : "",
    `theme-${theme}`,
    `size-${size}`,
    `radius-${radius}`,
    `engine-${engine}`,
    `motion-${motion}`,
    paused ? "is-paused" : "",
    hiddenIdle ? "is-hidden-idle" : "",
    isEditing ? "is-editing" : "",
    isEditing && isSelected ? "is-selected" : "",
    isEditing && isDragging ? "is-dragging" : "",
    clusterFrame ? "is-cluster" : "",
    clusterFrame ? `anchor-${clusterFrame.anchor}` : "",
    clusterFrame ? `is-${clusterFrame.resizeBehavior}` : "",
  ]
    .filter(Boolean)
    .join(" ");

  const longPressCleanupRef = useRef<(() => void) | null>(null);
  const suppressClickRef = useRef(false);
  const suppressTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (isEditing) {
      longPressCleanupRef.current?.();
      longPressCleanupRef.current = null;
    }
  }, [isEditing]);

  useEffect(() => {
    return () => {
      longPressCleanupRef.current?.();
      longPressCleanupRef.current = null;
      if (suppressTimerRef.current !== null) {
        window.clearTimeout(suppressTimerRef.current);
        suppressTimerRef.current = null;
      }
    };
  }, []);

  const handlePointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    if (isEditing) {
      if ((event.target as HTMLElement).closest(".ed-btn")) return;
      onBeginDrag?.();
      return;
    }
    if (preview || !onLongPressDrag) return;
    const target = event.target as HTMLElement | null;
    if (
      target?.closest(
        "button, a, input, textarea, select, [role='button'], [role='slider'], [role='menuitem'], [role='tab'], [contenteditable='true'], [data-no-long-press]",
      )
    ) {
      return;
    }

    longPressCleanupRef.current?.();
    const startX = event.clientX;
    const startY = event.clientY;

    const clearSuppressSoon = () => {
      if (suppressTimerRef.current !== null) {
        window.clearTimeout(suppressTimerRef.current);
      }
      suppressTimerRef.current = window.setTimeout(() => {
        suppressClickRef.current = false;
        suppressTimerRef.current = null;
      }, 600);
    };

    const cleanup = () => {
      window.clearTimeout(timerId);
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onEnd, true);
      window.removeEventListener("pointercancel", onEnd, true);
      window.removeEventListener("blur", onEnd);
      if (longPressCleanupRef.current === cleanup) {
        longPressCleanupRef.current = null;
      }
    };

    const onMove = (moveEvent: globalThis.PointerEvent) => {
      if (
        Math.abs(moveEvent.clientX - startX) > 6 ||
        Math.abs(moveEvent.clientY - startY) > 6
      ) {
        cleanup();
      }
    };

    const onEnd = () => {
      cleanup();
    };

    const timerId = window.setTimeout(() => {
      cleanup();
      suppressClickRef.current = true;
      window.addEventListener("pointerup", clearSuppressSoon, { once: true, capture: true });
      clearSuppressSoon();
      onLongPressDrag();
    }, 420);

    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", onEnd, true);
    window.addEventListener("pointercancel", onEnd, true);
    window.addEventListener("blur", onEnd);
    longPressCleanupRef.current = cleanup;
  };

  const handleClickCapture = (event: MouseEvent<HTMLDivElement>) => {
    if (!suppressClickRef.current) return;
    suppressClickRef.current = false;
    if (suppressTimerRef.current !== null) {
      window.clearTimeout(suppressTimerRef.current);
      suppressTimerRef.current = null;
    }
    event.preventDefault();
    event.stopPropagation();
  };

  return (
    <div ref={rootRef} className={rootClasses} style={style}>
      <div
        className="widget"
        data-kind={kind}
        role="group"
        aria-label={ariaLabel}
        onPointerDown={handlePointerDown}
        onClickCapture={handleClickCapture}
        onContextMenu={onContextMenu}
        onTransitionEnd={(event) => {
          if (
            event.target === event.currentTarget &&
            event.propertyName === "height"
          ) {
            clusterFrame?.onHeightTransitionEnd?.();
          }
        }}
      >
        {engine === "wallpaper" ? (
          <div className="glass-anchor-layer" aria-hidden="true">
            <GlassPlate
              plateRef={plateRef}
              platePath={glass?.platePath ?? ""}
              monitorLogicalW={glass?.monitorLogicalW ?? 1920}
              monitorLogicalH={glass?.monitorLogicalH ?? 1080}
              wx={glass?.wx ?? 0}
              wy={glass?.wy ?? 0}
            />
          </div>
        ) : (
          <div className="glass-anchor-layer" aria-hidden="true">
            <div ref={plateRef} className="glass-plate" aria-hidden="true" />
          </div>
        )}
        <div className="glass-scrim" aria-hidden="true" />
        <div
          className={`glass-ambient${ambientColor ? " has-ambient" : ""}`}
          aria-hidden="true"
        />
        <div className="glass-surface" aria-hidden="true" />
        <div className="glass-veil" aria-hidden="true" />
        <div ref={contentRef} className="content">
          {children}
        </div>
        <div className="edit-chrome" aria-hidden={!isEditing}>
          {isEditing && (
            <>
              <button
                type="button"
                className="ed-btn ed-x"
                aria-label={t("edit.removeWidget")}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  onRemoveWidget?.();
                }}
              >
                <X weight="bold" aria-hidden="true" />
              </button>
              {canCycleSize && (
                <button
                  type="button"
                  className="ed-btn ed-size"
                  aria-label={t("edit.cycleSize")}
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    onCycleSize?.();
                  }}
                >
                  <ArrowsOutSimple weight="bold" aria-hidden="true" />
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
