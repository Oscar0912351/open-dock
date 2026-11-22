import { X } from "@phosphor-icons/react";
import { useEffect, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { t } from "../../shared/i18n";
import { Button } from "./Button";

const openDrawerStack: symbol[] = [];

export function getOverlayPortalRoot(): Element {
  return document.querySelector(".cc-window") ?? document.body;
}

export function Drawer({
  open,
  title,
  subtitle,
  footer,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  subtitle?: string;
  footer?: ReactNode;
  onClose: () => void;
  children: ReactNode;
}) {
  useEffect(() => {
    if (!open) return;
    const token = Symbol("drawer");
    openDrawerStack.push(token);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && openDrawerStack[openDrawerStack.length - 1] === token) {
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      const idx = openDrawerStack.lastIndexOf(token);
      if (idx >= 0) openDrawerStack.splice(idx, 1);
    };
  }, [open, onClose]);

  if (!open) return null;

  return createPortal(
    <div
      className="cc-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onClick={onClose}
    >
      <div className="cc-drawer" onClick={(e) => e.stopPropagation()}>
        <div className="cc-drawer-head">
          <div className="cc-drawer-head-copy">
            <h3>{title}</h3>
            {subtitle && <span className="cc-drawer-subtitle">{subtitle}</span>}
          </div>
          <button
            type="button"
            className="cc-iconbtn"
            aria-label={t("window.close")}
            onClick={onClose}
          >
            <X size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="cc-drawer-body">{children}</div>
        {footer && <div className="cc-drawer-foot">{footer}</div>}
      </div>
    </div>,
    getOverlayPortalRoot(),
  );
}

export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel,
  danger = false,
  onConfirm,
  onClose,
}: {
  open: boolean;
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
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

  if (!open) return null;

  return createPortal(
    <div
      className="cc-backdrop"
      role="alertdialog"
      aria-modal="true"
      aria-label={title}
      onClick={onClose}
    >
      <div className="cc-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        <p>{body}</p>
        <div className="cc-dialog-actions">
          <Button variant="ghost" onClick={onClose}>
            {t("settings.cancel")}
          </Button>
          <Button
            variant={danger ? "danger" : "primary"}
            onClick={() => {
              onClose();
              onConfirm();
            }}
          >
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>,
    getOverlayPortalRoot(),
  );
}
