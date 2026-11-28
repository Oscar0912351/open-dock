import { ArrowClockwise, Copy, DownloadSimple, Minus, NotePencil, Square, Warning, X } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import type { UpdateStatus } from "../../shared/bindings/UpdateStatus";
import { t } from "../../shared/i18n";
import { DockLogo } from "../../shared/icons/DockLogo";
import { ipc, on, windowControls } from "../../shared/ipc";
import { reportFailure } from "../../shared/logging";
import { useControlStore } from "../store";

interface TitleBarProps {
  maximized?: boolean;
  showOnboarding?: boolean;
}

export function TitleBar({
  maximized = false,
  showOnboarding = false,
}: TitleBarProps) {
  useControlStore((s) => s.locale);
  const route = useControlStore((s) => s.route);
  const editMode = useControlStore((s) => s.editMode);
  const setRoute = useControlStore((s) => s.setRoute);

  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [updatePct, setUpdatePct] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    void ipc
      .updateStatus()
      .then((status) => {
        if (!cancelled) setUpdate(status);
      })
      .catch((reason: unknown) => reportFailure("update_status", reason));

    let unlistenStatus: (() => void) | null = null;
    void on
      .updateStatus((status) => {
        if (cancelled) return;
        setUpdate(status);
        if (status.state !== "downloading") setUpdatePct(null);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenStatus = fn;
        }
      });

    let unlistenProgress: (() => void) | null = null;
    void on
      .updateProgress((p) => {
        if (cancelled) return;
        setUpdatePct(
          p.total > 0 ? Math.min(100, Math.round((p.downloaded / p.total) * 100)) : null,
        );
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenProgress = fn;
        }
      });

    return () => {
      cancelled = true;
      unlistenStatus?.();
      unlistenProgress?.();
    };
  }, []);

  const downloading =
    update?.state === "downloading" || update?.state === "available";

  return (
    <header className="cc-titlebar">
      <div className="cc-titlebar-rail" data-tauri-drag-region>
        <div className="cc-titlebar-brand" data-tauri-drag-region>
          <DockLogo size={18} />
          <span data-tauri-drag-region>{t("app.name")}</span>
        </div>
      </div>

      <div className="cc-titlebar-main">
        <div className="cc-titlebar-ctx" data-tauri-drag-region style={{ flex: 1, height: "100%" }}>
          {!showOnboarding && (
            <>
              <span className="cc-titlebar-route" data-tauri-drag-region>
                {t(`nav.${route}`)}
              </span>
              {editMode && (
                <span className="cc-titlebar-edit-pill" data-tauri-drag-region>
                  <NotePencil size={11} aria-hidden="true" />
                  <span>{t("edit.barLead")}</span>
                </span>
              )}
            </>
          )}
        </div>

        <div className="cc-titlebar-actions">
          {!showOnboarding && update?.state === "ready" && (
            <button
              type="button"
              className="cc-titlebar-update is-ready"
              title={t("settings.updateRestartTitle")}
              onClick={() => void ipc.updateInstall()}
            >
              <ArrowClockwise size={11} aria-hidden="true" />
              {t("settings.updateRestart")}
            </button>
          )}
          {!showOnboarding && downloading && (
            <button
              type="button"
              className="cc-titlebar-update is-downloading"
              title={t("settings.updateHelpDownloading", {
                version: update?.latest?.version ?? "",
              })}
              onClick={() => setRoute("settings")}
            >
              <DownloadSimple size={11} aria-hidden="true" />
              {updatePct !== null ? `${updatePct} %` : t("settings.updateDownloadingShort")}
            </button>
          )}
          {!showOnboarding && update?.state === "failed" && (
            <button
              type="button"
              className="cc-titlebar-update is-failed"
              title={t("settings.updateHelpFailed")}
              onClick={() => setRoute("settings")}
            >
              <Warning size={11} aria-hidden="true" />
              {t("settings.updateStatusFailed")}
            </button>
          )}
          <button
            type="button"
            className="cc-titlebar-btn"
            aria-label={t("window.minimize")}
            onClick={() => void windowControls.minimize()}
          >
            <Minus size={15} aria-hidden="true" />
          </button>
          <button
            type="button"
            className="cc-titlebar-btn"
            aria-label={t("window.maximize")}
            onClick={() => void windowControls.toggleMaximize()}
          >
            {maximized ? <Copy size={13} aria-hidden="true" /> : <Square size={12} aria-hidden="true" />}
          </button>
          <button
            type="button"
            className="cc-titlebar-btn is-close"
            aria-label={t("window.close")}
            onClick={() => void windowControls.close()}
          >
            <X size={15} aria-hidden="true" />
          </button>
        </div>
      </div>
    </header>
  );
}
