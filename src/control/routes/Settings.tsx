import { Moon, Sun } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import type { ControlTheme } from "../../shared/bindings/ControlTheme";
import type { CpuSensorStatus } from "../../shared/bindings/CpuSensorStatus";
import type { DiscordRpcStatus } from "../../shared/bindings/DiscordRpcStatus";
import type { Language } from "../../shared/bindings/Language";
import type { UpdateProgress } from "../../shared/bindings/UpdateProgress";
import type { UpdateStatus } from "../../shared/bindings/UpdateStatus";
import { getDateTimeFormatter } from "../../shared/format";
import { t } from "../../shared/i18n";
import { DockLogo } from "../../shared/icons/DockLogo";
import { ipc, on } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Card, SettingRow } from "../components/Card";
import { ConfirmDialog } from "../components/Drawer";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { ShortcutInput } from "../components/ShortcutInput";
import { Switch } from "../components/Switch";
import { useControlStore } from "../store";

function formatError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    return String((err as { message: unknown }).message);
  }
  return String(err);
}

export function Settings() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const appVersion = useControlStore((s) => s.appVersion);

  const [resetConfirmOpen, setResetConfirmOpen] = useState(false);
  const [sensorStatus, setSensorStatus] = useState<CpuSensorStatus | null>(null);
  const [sensorBusy, setSensorBusy] = useState<"install" | "uninstall" | null>(null);
  const [sensorError, setSensorError] = useState<string | null>(null);

  const [discordStatus, setDiscordStatus] = useState<DiscordRpcStatus | null>(null);
  const [discordBusy, setDiscordBusy] = useState<"connect" | "disconnect" | null>(null);
  const [discordError, setDiscordError] = useState<string | null>(null);

  const [updateInfo, setUpdateInfo] = useState<UpdateStatus | null>(null);
  const [updateBusy, setUpdateBusy] = useState<"check" | "install" | null>(null);
  const [updateDl, setUpdateDl] = useState<UpdateProgress | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void ipc
      .cpuSensorStatus()
      .then((status) => {
        if (!cancelled) setSensorStatus(status);
      })
      .catch((err) => {
        if (!cancelled) setSensorError(formatError(err));
      });

    void ipc
      .discordRpcStatus()
      .then((st) => {
        if (cancelled) return;
        setDiscordStatus(st);
      })
      .catch(() => {});

    void ipc
      .updateStatus()
      .then((status) => {
        if (!cancelled) setUpdateInfo(status);
      })
      .catch(() => {});

    let unlisten: (() => void) | null = null;
    void on
      .discordRpcStatus((st) => {
        if (cancelled) return;
        setDiscordStatus(st);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      });

    let unlistenUpdate: (() => void) | null = null;
    void on
      .updateStatus((st) => {
        if (cancelled) return;
        setUpdateInfo(st);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenUpdate = fn;
        }
      });

    let unlistenProgress: (() => void) | null = null;
    void on
      .updateProgress((p) => {
        if (cancelled) return;
        setUpdateDl(p);
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
      unlisten?.();
      unlistenUpdate?.();
      unlistenProgress?.();
    };
  }, []);

  if (!config) return null;

  const { general } = config;

  const handleInstallSensor = async () => {
    setSensorBusy("install");
    setSensorError(null);
    try {
      const next = await ipc.cpuSensorInstall();
      setSensorStatus(next);
    } catch (err) {
      setSensorError(formatError(err));
      void ipc.cpuSensorStatus().then(setSensorStatus).catch(() => {});
    } finally {
      setSensorBusy(null);
    }
  };

  const handleUninstallSensor = async () => {
    setSensorBusy("uninstall");
    setSensorError(null);
    try {
      const next = await ipc.cpuSensorUninstall();
      setSensorStatus(next);
    } catch (err) {
      setSensorError(formatError(err));
      void ipc.cpuSensorStatus().then(setSensorStatus).catch(() => {});
    } finally {
      setSensorBusy(null);
    }
  };

  const handleDiscordConnectCentral = async () => {
    if (discordBusy !== null) return;
    setDiscordBusy("connect");
    setDiscordError(null);
    try {
      const next = await ipc.discordRpcConnectCentral();
      setDiscordStatus(next);
    } catch (err) {
      setDiscordError(formatError(err));
    } finally {
      setDiscordBusy(null);
    }
  };

  const handleDiscordDisconnect = async () => {
    if (discordBusy !== null) return;
    setDiscordBusy("disconnect");
    setDiscordError(null);
    try {
      const next = await ipc.discordRpcDisconnect();
      setDiscordStatus(next);
    } catch (err) {
      setDiscordError(formatError(err));
    } finally {
      setDiscordBusy(null);
    }
  };

  const handleCheckUpdate = async () => {
    if (updateBusy !== null) return;
    setUpdateBusy("check");
    setUpdateError(null);
    try {
      const next = await ipc.updateCheck();
      setUpdateInfo(next);
    } catch (err) {
      setUpdateError(formatError(err));
    } finally {
      setUpdateBusy(null);
    }
  };

  const handleInstallUpdate = async () => {
    if (updateBusy !== null) return;
    setUpdateBusy("install");
    setUpdateError(null);
    setUpdateDl(null);
    try {
      // Appen lukker sig selv når installeren starter.
      await ipc.updateInstall();
    } catch (err) {
      setUpdateError(formatError(err));
      setUpdateBusy(null);
      setUpdateDl(null);
    }
  };

  const sensorTone: "live" | "warn" | "muted" = !sensorStatus
    ? "muted"
    : sensorStatus.state === "running" && sensorStatus.driverInstalled && !sensorStatus.outdated
      ? "live"
      : sensorStatus.state === "running" ||
          sensorStatus.state === "stopped" ||
          sensorStatus.elevatedFallback
        ? "warn"
        : "muted";

  const sensorLabel = !sensorStatus
    ? t("settings.cpuSensorStatusLoading")
    : sensorStatus.state === "running"
      ? !sensorStatus.driverInstalled
        ? t("settings.cpuSensorStatusNoDriver")
        : sensorStatus.outdated
          ? t("settings.cpuSensorStatusOutdated")
          : t("settings.cpuSensorStatusRunning")
      : sensorStatus.state === "stopped"
        ? t("settings.cpuSensorStatusStopped")
        : sensorStatus.elevatedFallback
          ? t("settings.cpuSensorStatusElevated")
          : t("settings.cpuSensorStatusNotInstalled");

  const sensorHelp = !sensorStatus
    ? t("settings.cpuSensorHelpNotInstalled")
    : sensorStatus.state === "unsupported"
      ? t("settings.cpuSensorHelpUnsupported")
      : sensorStatus.state === "running"
        ? !sensorStatus.driverInstalled
          ? t("settings.cpuSensorHelpNoDriver")
          : sensorStatus.outdated
            ? t("settings.cpuSensorHelpOutdated")
            : t("settings.cpuSensorHelpRunning")
        : sensorStatus.state === "stopped"
          ? t("settings.cpuSensorHelpStopped")
          : sensorStatus.elevatedFallback
            ? t("settings.cpuSensorHelpElevated")
            : t("settings.cpuSensorHelpNotInstalled");

  const isUnsupported = sensorStatus?.state === "unsupported";
  const isInstalled =
    sensorStatus !== null &&
    (sensorStatus.state === "running" || sensorStatus.state === "stopped");
  const needsInstallOrRepair =
    sensorStatus !== null &&
    !isUnsupported &&
    (sensorStatus.state === "notInstalled" ||
      sensorStatus.state === "stopped" ||
      sensorStatus.outdated);

  const primaryActionLabel =
    sensorBusy === "install"
      ? t("settings.cpuSensorInstalling")
      : sensorStatus?.state === "stopped"
        ? t("settings.cpuSensorRepair")
        : sensorStatus?.outdated
          ? t("settings.cpuSensorUpdate")
          : t("settings.cpuSensorInstall");

  const updateState = updateInfo?.state ?? null;
  const downloadPct =
    updateDl && updateDl.total > 0
      ? Math.min(100, Math.round((updateDl.downloaded / updateDl.total) * 100))
      : null;
  const downloading = updateState === "downloading" || updateState === "available";

  const updateTone: "live" | "warn" | "muted" =
    updateBusy === "install" ||
    updateState === "ready" ||
    updateState === "failed" ||
    downloading
      ? "warn"
      : updateState === "upToDate"
        ? "live"
        : "muted";

  const updateLabel =
    updateBusy === "check"
      ? t("settings.updateChecking")
      : downloading
        ? downloadPct !== null
          ? t("settings.updateDownloading", { pct: downloadPct })
          : t("settings.updateStatusDownloading")
        : updateState === "ready"
          ? t("settings.updateStatusReady")
          : updateState === "failed"
            ? t("settings.updateStatusFailed")
            : updateState === "upToDate"
              ? t("settings.updateStatusUpToDate")
              : updateState === "dev"
                ? t("settings.updateStatusDev")
                : t("settings.updateStatusUnchecked");

  const buildDateFmt = getDateTimeFormatter("da-DK", "update-build", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
  const updateHelp =
    updateBusy === "check"
      ? t("settings.updateChecking")
      : updateState === "dev"
        ? t("settings.updateHelpDev")
        : downloading && updateInfo?.latest
          ? t("settings.updateHelpDownloading", {
              version: updateInfo.latest.version,
            })
          : updateState === "ready" && updateInfo?.latest
            ? t("settings.updateHelpReady", {
                version: updateInfo.latest.version,
                date: buildDateFmt.format(new Date(updateInfo.latest.buildId * 1000)),
              })
            : updateState === "failed"
              ? t("settings.updateHelpFailed")
              : updateState === "upToDate"
                ? t("settings.updateHelpUpToDate")
                : t("settings.updateHelpUnchecked");

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
                {
                  value: "dark",
                  label: t("settings.uiThemeDark"),
                  icon: <Moon size={13} aria-hidden="true" />,
                },
                {
                  value: "light",
                  label: t("settings.uiThemeLight"),
                  icon: <Sun size={13} aria-hidden="true" />,
                },
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
          <SettingRow title={t("settings.gameOverlay")} help={t("settings.gameOverlayHelp")}>
            <Switch
              checked={general.gameOverlay}
              ariaLabel={t("settings.gameOverlay")}
              onChange={(gameOverlay) => void ipc.generalPatch({ gameOverlay })}
            />
          </SettingRow>

          <SettingRow
            title={t("settings.pauseFullscreen")}
            help={
              general.gameOverlay
                ? t("settings.pauseFullscreenOverlayNote")
                : t("settings.pauseFullscreenHelp")
            }
          >
            <Switch
              checked={general.pauseOnFullscreen}
              disabled={general.gameOverlay}
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

      {/* Sensorer */}
      <section className="cc-section">
        <span className="cc-section-label">{t("settings.sensors")}</span>
        <Card compact>
          <SettingRow
            title={t("settings.cpuSensor")}
            help={sensorHelp}
            meta={
              <span className={`cc-sensor-state is-${sensorTone}`}>
                <span className="cc-sensor-dot" aria-hidden="true" />
                {sensorLabel}
              </span>
            }
          >
            {!isUnsupported && (
              <div className="cc-sensor-actions">
                {needsInstallOrRepair && (
                  <Button
                    variant={isInstalled ? "secondary" : "primary"}
                    disabled={sensorBusy !== null || sensorStatus === null}
                    onClick={() => void handleInstallSensor()}
                  >
                    {primaryActionLabel}
                  </Button>
                )}
                {isInstalled && (
                  <Button
                    variant="secondary"
                    disabled={sensorBusy !== null}
                    onClick={() => void handleUninstallSensor()}
                  >
                    {sensorBusy === "uninstall"
                      ? t("settings.cpuSensorUninstalling")
                      : t("settings.cpuSensorUninstall")}
                  </Button>
                )}
              </div>
            )}
          </SettingRow>
          {sensorError && (
            <div className="cc-sensor-error" role="alert">
              {sensorError}
            </div>
          )}
        </Card>
      </section>

      {/* Integrationer */}
      <section className="cc-section">
        <span className="cc-section-label">{t("discord.rpcSection")}</span>
        <Card compact>
          <SettingRow
            title={t("discord.rpcTitle")}
            help={
              discordStatus?.connected
                ? t("discord.rpcHelpConnected")
                : t("discord.rpcViaCentralHelp")
            }
            meta={
              <span
                className={`cc-sensor-state is-${
                  discordStatus?.connected
                    ? "live"
                    : discordStatus?.configured
                      ? "warn"
                      : "muted"
                }`}
              >
                <span className="cc-sensor-dot" aria-hidden="true" />
                {discordStatus?.connected
                  ? t("discord.rpcStatusConnected", {
                      user: discordStatus.username || "Discord",
                    })
                  : discordStatus?.configured
                    ? t("discord.rpcStatusReady")
                    : t("discord.rpcStatusFallback")}
              </span>
            }
          >
            <div className="cc-sensor-actions">
              {discordStatus?.configured ? (
                <>
                  <Button
                    variant="secondary"
                    disabled={discordBusy !== null}
                    onClick={() => void handleDiscordConnectCentral()}
                  >
                    {discordBusy === "connect"
                      ? t("discord.rpcConnecting")
                      : t("discord.rpcReconnect")}
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={discordBusy !== null}
                    onClick={() => void handleDiscordDisconnect()}
                  >
                    {t("discord.rpcDisconnect")}
                  </Button>
                </>
              ) : (
                <Button
                  variant="primary"
                  disabled={discordBusy !== null}
                  onClick={() => void handleDiscordConnectCentral()}
                >
                  {discordBusy === "connect"
                    ? t("discord.rpcConnecting")
                    : t("discord.rpcConnect")}
                </Button>
              )}
            </div>
          </SettingRow>

          {(discordError || discordStatus?.lastError) && (
            <div className="cc-sensor-error" role="alert">
              {discordError || discordStatus?.lastError}
            </div>
          )}
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
          <SettingRow
            title={t("settings.update")}
            help={updateHelp}
            meta={
              <span className={`cc-sensor-state is-${updateTone}`}>
                <span className="cc-sensor-dot" aria-hidden="true" />
                {updateLabel}
              </span>
            }
          >
            <div className="cc-sensor-actions">
              {updateState === "ready" ? (
                <Button
                  variant="primary"
                  disabled={updateBusy !== null}
                  onClick={() => void handleInstallUpdate()}
                >
                  {updateBusy === "install"
                    ? t("settings.updateInstalling")
                    : t("settings.updateRestart")}
                </Button>
              ) : downloading ? (
                <Button variant="secondary" disabled>
                  {downloadPct !== null
                    ? t("settings.updateDownloading", { pct: downloadPct })
                    : t("settings.updateStatusDownloading")}
                </Button>
              ) : updateState !== "dev" ? (
                <Button
                  variant="secondary"
                  disabled={updateBusy !== null}
                  onClick={() => void handleCheckUpdate()}
                >
                  {updateBusy === "check"
                    ? t("settings.updateChecking")
                    : updateState === "failed"
                      ? t("settings.updateRetry")
                      : t("settings.updateCheck")}
                </Button>
              ) : null}
            </div>
          </SettingRow>
          {updateError && (
            <div className="cc-sensor-error" role="alert">
              {updateError}
            </div>
          )}
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
