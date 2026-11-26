import { useCallback, useEffect, useRef, useState } from "react";
import type { ClusterConfig } from "../../shared/bindings/ClusterConfig";
import type { MotionPreset } from "../../shared/bindings/MotionPreset";
import { ipc } from "../../shared/ipc";

interface UseClusterWindowResizeOptions {
  initialIdleHeight: number;
  targetHeightU: number;
  cluster: ClusterConfig;
  instanceId: string;
  motion: MotionPreset;
  preview: boolean;
  liveBackdrop: boolean;
  registerTransitionEnd: (handler: (() => void) | null) => void;
}

export function shouldReduceMotion(motion: MotionPreset): boolean {
  if (motion === "off") return true;
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

export function useClusterWindowResize({
  initialIdleHeight,
  targetHeightU,
  cluster,
  instanceId,
  motion,
  preview,
  liveBackdrop,
  registerTransitionEnd,
}: UseClusterWindowResizeOptions) {
  // 2-trins udvid/kollaps-håndtryk mod Win32 (plan/04b §2.1)
  const [boxHeightU, setBoxHeightU] = useState<number>(initialIdleHeight);
  const boxHeightRef = useRef<number>(initialIdleHeight);
  const [transitionKind, setTransitionKind] = useState<"expand" | "collapse" | "idle">("idle");
  const winHeightRef = useRef<number>(initialIdleHeight);
  const collapseTimerRef = useRef<number | null>(null);
  const pendingCollapseTargetRef = useRef<number | null>(null);
  const resizeSeqRef = useRef(0);

  const updateBoxHeight = useCallback((nextH: number) => {
    boxHeightRef.current = nextH;
    setBoxHeightU(nextH);
  }, []);

  const commitWinSize = useCallback(
    (nextH: number) => {
      winHeightRef.current = nextH;
      pendingCollapseTargetRef.current = null;
      if (collapseTimerRef.current !== null) {
        window.clearTimeout(collapseTimerRef.current);
        collapseTimerRef.current = null;
      }
      setTransitionKind("idle");
      if (preview) return;
      void ipc
        .clusterSetWindowSize(instanceId, cluster.width, nextH, cluster.anchor)
        .catch(() => {});
    },
    [instanceId, cluster.width, cluster.anchor, preview],
  );

  const liveBackdropRef = useRef(liveBackdrop);
  useEffect(() => {
    liveBackdropRef.current = liveBackdrop;
  }, [liveBackdrop]);

  useEffect(() => {
    registerTransitionEnd(() => {
      const target = pendingCollapseTargetRef.current;
      if (target !== null) {
        commitWinSize(target);
      } else {
        setTransitionKind("idle");
        if (liveBackdropRef.current && !preview) {
          void ipc
            .widgetAnimateBackdropBox(instanceId, 0, 0, cluster.anchor, 0)
            .catch(() => {});
        }
      }
    });
    return () => registerTransitionEnd(null);
  }, [commitWinSize, registerTransitionEnd, preview, instanceId, cluster.anchor]);

  useEffect(() => {
    if (preview) {
      winHeightRef.current = targetHeightU;
      boxHeightRef.current = targetHeightU;
      return;
    }

    if (cluster.resizeBehavior === "fixed") {
      if (winHeightRef.current !== targetHeightU) {
        boxHeightRef.current = targetHeightU;
        commitWinSize(targetHeightU);
      }
      return;
    }

    const currentWinH = winHeightRef.current;
    const currentBoxH = boxHeightRef.current;
    if (targetHeightU === currentWinH && currentBoxH === targetHeightU) {
      return;
    }

    if (collapseTimerRef.current !== null) {
      window.clearTimeout(collapseTimerRef.current);
      collapseTimerRef.current = null;
    }
    pendingCollapseTargetRef.current = null;

    if (targetHeightU > currentWinH) {
      // Udvidelse: udvid Win32-vindue FØR CSS-animation
      const seq = ++resizeSeqRef.current;
      const fromH = currentBoxH;
      winHeightRef.current = targetHeightU;
      const frame = window.requestAnimationFrame(() => {
        setTransitionKind("expand");
      });

      if (shouldReduceMotion(motion)) {
        void ipc
          .clusterSetWindowSize(instanceId, cluster.width, targetHeightU, cluster.anchor)
          .catch(() => {})
          .finally(() => {
            if (resizeSeqRef.current === seq) {
              updateBoxHeight(targetHeightU);
            }
          });
        return () => window.cancelAnimationFrame(frame);
      }

      void ipc
        .clusterSetWindowSize(instanceId, cluster.width, targetHeightU, cluster.anchor)
        .catch(() => {})
        .finally(() => {
          if (resizeSeqRef.current === seq) {
            updateBoxHeight(targetHeightU);
            if (liveBackdrop && Math.abs(fromH - targetHeightU) >= 0.25) {
              void ipc
                .widgetAnimateBackdropBox(instanceId, fromH, targetHeightU, cluster.anchor, 340)
                .catch(() => {});
            }
          }
        });
      return () => window.cancelAnimationFrame(frame);
    }

    if (targetHeightU < currentWinH) {
      // Kollaps: start CSS-overgang først, krymp Win32 efter transitionend / 300ms fallback
      if (shouldReduceMotion(motion)) {
        const frame = window.requestAnimationFrame(() => {
          setTransitionKind("idle");
          updateBoxHeight(targetHeightU);
          commitWinSize(targetHeightU);
        });
        return () => window.cancelAnimationFrame(frame);
      }

      const fromH = currentBoxH;
      const frame = window.requestAnimationFrame(() => {
        setTransitionKind("collapse");
        updateBoxHeight(targetHeightU);
        pendingCollapseTargetRef.current = targetHeightU;
        if (liveBackdrop && Math.abs(fromH - targetHeightU) >= 0.25) {
          void ipc
            .widgetAnimateBackdropBox(instanceId, fromH, targetHeightU, cluster.anchor, 280)
            .catch(() => {});
        }
        collapseTimerRef.current = window.setTimeout(() => {
          const target = pendingCollapseTargetRef.current;
          if (target !== null) {
            commitWinSize(target);
          }
        }, 300);
      });
      return () => window.cancelAnimationFrame(frame);
    }

    // Hvis currentWinH allerede matcher (fx gen-afbrudt kollaps), sæt boxHeight direkte
    const fromH = currentBoxH;
    const frame = window.requestAnimationFrame(() => {
      setTransitionKind("expand");
      updateBoxHeight(targetHeightU);
      if (liveBackdrop && !shouldReduceMotion(motion) && Math.abs(fromH - targetHeightU) >= 0.25) {
        void ipc
          .widgetAnimateBackdropBox(instanceId, fromH, targetHeightU, cluster.anchor, 340)
          .catch(() => {});
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [targetHeightU, cluster.resizeBehavior, cluster.width, cluster.anchor, instanceId, motion, commitWinSize, updateBoxHeight, preview, liveBackdrop]);

  useEffect(() => {
    return () => {
      if (collapseTimerRef.current !== null) window.clearTimeout(collapseTimerRef.current);
    };
  }, []);

  return { boxHeightU, transitionKind };
}
