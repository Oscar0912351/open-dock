import { useState } from "react";
import type { ControlTheme } from "../../shared/bindings/ControlTheme";
import type { Language } from "../../shared/bindings/Language";
import { t } from "../../shared/i18n";
import { DockLogo } from "../../shared/icons/DockLogo";
import { ipc } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Card, SettingRow } from "../components/Card";
import { ConfirmDialog } from "../components/Drawer";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { ShortcutInput } from "../components/ShortcutInput";
import { Switch } from "../components/Switch";
import { useControlStore } from "../store";

export function Settings() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const appVersion = useControlStore((s) => s.appVersion);

  const [resetConfirmOpen, setResetConfirmOpen] = useState(false);

  if (!config) return null;

  const { general } = config;

  return (
    <div className="cc-route is-narrow">
      <div className="cc-page-header">
        <div className="cc-page-header-copy">
          <h1 className="cc-page-title">{t("settings.title")}</h1>
          <p className="cc-page-subtitle">{t("settings.subtitle")}</p>
        </div>
      </div>

      {/* Generelt */}
      <section className="cc-section">
        <span className="cc-section-label">{t("settings.general")}</span>
        <Card compact>
          <SettingRow title={t("settings.uiTheme")} help={t("settings.uiThemeHelp")}>
            <Segmented<ControlTheme>
              value={general.controlTheme}
              ariaLabel={t("settings.uiTheme")}
              options={[
                { value: "dark", label: t("settings.uiThemeDark") },
                { value: "light", label: t("settings.uiThemeLight") },
              ]}
              onChange={(controlTheme) => void ipc.generalPatch({ controlTheme })}
            />
          </SettingRow>

          <SettingRow title={t("settings.autostart")} help={t("settings.autostartHelp")}>
            <Switch
              checked={general.autostart}
              ariaLabel={t("settings.autostart")}
              onChange={(autostart) => void ipc.generalPatch({ autostart })}
            />
          </SettingRow>

          <SettingRow title={t("settings.language")}>
            <div style={{ width: 170 }}>
              <Select<Language>
                value={general.language}
                ariaLabel={t("settings.language")}
                options={[
                  { value: "da", label: "Dansk" },
                  { value: "en", label: "English" },
                ]}
                onChange={(language) => void ipc.generalPatch({ language })}
              />
            </div>
          </SettingRow>

          <SettingRow title={t("settings.gridSnap")} help={t("settings.gridSnapHelp")}>
            <Switch
              checked={general.gridSnap}
              ariaLabel={t("settings.gridSnap")}
              onChange={(gridSnap) => void ipc.generalPatch({ gridSnap })}
            />
          </SettingRow>
        </Card>
      </section>

      {/* Ydelse */}
      <section className="cc-section">
        <span className="cc-section-label">{t("settings.performance")}</span>
        <Card compact>
          <SettingRow title={t("settings.pauseFullscreen")} help={t("settings.pauseFullscreenHelp")}>
            <Switch
              checked={general.pauseOnFullscreen}
              ariaLabel={t("settings.pauseFullscreen")}
              onChange={(pauseOnFullscreen) => void ipc.generalPatch({ pauseOnFullscreen })}
            />
          </SettingRow>

          <SettingRow title={t("settings.pauseBattery")} help={t("settings.pauseBatteryHelp")}>
            <Switch
              checked={general.pauseOnBattery}
              ariaLabel={t("settings.pauseBattery")}
              onChange={(pauseOnBattery) => void ipc.generalPatch({ pauseOnBattery })}
            />
          </SettingRow>
        </Card>
      </section>

      {/* Genveje */}
      <section className="cc-section">
        <span className="cc-section-label">{t("settings.shortcuts")}</span>
        <Card compact>
          <SettingRow title={t("settings.shortcutEdit")}>
            <ShortcutInput
              value={general.hotkeys.editMode}
              conflictWith={general.hotkeys.toggleVisible}
              ariaLabel={t("settings.shortcutEdit")}
              onChange={(editMode) =>
                ipc
                  .generalPatch({
                    hotkeys: { ...general.hotkeys, editMode },
                  })
                  .then(() => undefined)
              }
            />
          </SettingRow>

          <SettingRow title={t("settings.shortcutToggle")}>
            <ShortcutInput
              value={general.hotkeys.toggleVisible}
              conflictWith={general.hotkeys.editMode}
              ariaLabel={t("settings.shortcutToggle")}
              onChange={(toggleVisible) =>
                ipc
                  .generalPatch({
                    hotkeys: { ...general.hotkeys, toggleVisible },
                  })
                  .then(() => undefined)
              }
            />
          </SettingRow>
        </Card>
      </section>

      {/* Om */}
      <section className="cc-section">
        <span className="cc-section-label">{t("settings.about")}</span>
        <Card compact>
          <div className="cc-about-hero">
            <div className="cc-about-brand">
              <DockLogo size={36} />
              <div className="cc-about-meta">
                <h3>
                  <span>{t("app.name")}</span>
                  <span className="cc-titlebar-version">v{appVersion}</span>
                </h3>
                <p>{t("app.tagline")}</p>
              </div>
            </div>
          </div>
        </Card>
      </section>

      {/* Farezone */}
      <section className="cc-section">
        <span className="cc-section-label">{t("settings.dangerZone")}</span>
        <Card compact className="is-danger-card">
          <SettingRow title={t("settings.resetAll")}>
            <Button variant="danger" onClick={() => setResetConfirmOpen(true)}>
              {t("settings.resetAll")}
            </Button>
          </SettingRow>

          <SettingRow title={t("settings.quitApp")}>
            <Button variant="secondary" onClick={() => void ipc.appQuit()}>
              {t("settings.quitApp")}
            </Button>
          </SettingRow>
        </Card>
      </section>

      <ConfirmDialog
        open={resetConfirmOpen}
        danger
        title={t("settings.resetConfirmTitle")}
        body={t("settings.resetConfirmBody")}
        confirmLabel={t("settings.resetConfirmAction")}
        onConfirm={() => void ipc.appReset()}
        onClose={() => setResetConfirmOpen(false)}
      />
    </div>
  );
}
