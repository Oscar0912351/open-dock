import type { CSSProperties } from "react";
import { useControlStore } from "../store";

export function ToastStack() {
  const toasts = useControlStore((s) => s.toasts);
  const dismiss = useControlStore((s) => s.dismissToast);

  if (toasts.length === 0) return null;

  return (
    <div className="cc-toast-stack" role="region" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className="cc-toast" role="status">
          <span>{toast.message}</span>
          {toast.actionLabel && toast.onAction && (
            <button
              type="button"
              className="cc-toast-action"
              onClick={() => {
                toast.onAction?.();
                dismiss(toast.id);
              }}
            >
              {toast.actionLabel}
            </button>
          )}
          {toast.undo && (
            <span
              className="cc-toast-bar"
              style={{ "--toast-ms": `${toast.durationMs}ms` } as CSSProperties}
              aria-hidden="true"
            />
          )}
        </div>
      ))}
    </div>
  );
}
