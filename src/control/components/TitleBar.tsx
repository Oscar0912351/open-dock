import { Copy, Minus, Moon, NotePencil, Square, Sun, X } from "@phosphor-icons/react";
import type { ControlTheme } from "../../shared/bindings/ControlTheme";
import { t } from "../../shared/i18n";
import { DockLogo } from "../../shared/icons/DockLogo";
import { ipc, windowControls } from "../../shared/ipc";
import { useControlStore } from "../store";

interface TitleBarProps {
  maximized?: boolean;
  showOnboarding?: boolean;
  uiTheme?: ControlTheme;
}

export function TitleBar({
  maximized = false,
  showOnboarding = false,
  uiTheme = "dark",
}: TitleBarProps) {
  useControlStore((s) => s.locale);
  const route = useControlStore((s) => s.route);
  const editMode = useControlStore((s) => s.editMode);
  const appVersion = useControlStore((s) => s.appVersion);

  const nextTheme: ControlTheme = uiTheme === "dark" ? "light" : "dark";
  const toggleLabel = uiTheme === "dark" ? t("window.switchLight") : t("window.switchDark");

  return (
    <header className="cc-titlebar">
      <div className="cc-titlebar-rail" data-tauri-drag-region>
        <div className="cc-titlebar-brand" data-tauri-drag-region>
          <DockLogo size={18} />
          <span data-tauri-drag-region>{t("app.name")}</span>
        </div>
        {!showOnboarding && (
          <span className="cc-titlebar-version" data-tauri-drag-region>
            v{appVersion}
          </span>
        )}
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
          <button
            type="button"
            className="cc-titlebar-btn is-theme-toggle"
            aria-label={toggleLabel}
            title={toggleLabel}
            onClick={() => void ipc.generalPatch({ controlTheme: nextTheme })}
          >
            {uiTheme === "dark" ? (
              <Sun size={15} aria-hidden="true" />
            ) : (
              <Moon size={15} aria-hidden="true" />
            )}
          </button>

          <span className="cc-titlebar-divider" aria-hidden="true" />

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
