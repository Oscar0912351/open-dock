import { Check, Copy, Moon, Sun } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import type { ControlTheme } from "../../shared/bindings/ControlTheme";
import type { CpuSensorStatus } from "../../shared/bindings/CpuSensorStatus";
import type { DiscordRpcStatus } from "../../shared/bindings/DiscordRpcStatus";
import type { Language } from "../../shared/bindings/Language";
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
  const [discordFormOpen, setDiscordFormOpen] = useState(false);
  const [discordClientId, setDiscordClientId] = useState("");
  const [discordClientSecret, setDiscordClientSecret] = useState("");
  const [discordBusy, setDiscordBusy] = useState<"connect" | "disconnect" | null>(null);
  const [discordError, setDiscordError] = useState<string | null>(null);
  const [redirectCopied, setRedirectCopied] = useState(false);
  const copiedTimerRef = useRef<number | null>(null);

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
        if (st.clientId) setDiscordClientId(st.clientId);
      })
      .catch(() => {});

    let unlisten: (() => void) | null = null;
    void on
      .discordRpcStatus((st) => {
        if (cancelled) return;
        setDiscordStatus(st);
        if (st.clientId) {
          setDiscordClientId((prev) => prev || st.clientId || "");
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      });

    return () => {
      cancelled = true;
      unlisten?.();
      if (copiedTimerRef.current !== null) {
        window.clearTimeout(copiedTimerRef.current);
      }
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
      setDiscordFormOpen(false);
    } catch (err) {
      setDiscordError(formatError(err));
    } finally {
      setDiscordBusy(null);
    }
  };

  const handleDiscordConnect = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!discordClientId.trim() || discordBusy !== null) return;
    setDiscordBusy("connect");
    setDiscordError(null);
    try {
      const next = await ipc.discordRpcConnect(
        discordClientId.trim(),
        discordClientSecret.trim(),
        discordStatus?.redirectUri || "http://127.0.0.1",
      );
      setDiscordStatus(next);
      setDiscordClientSecret("");
      setDiscordFormOpen(false);
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
      setDiscordClientId("");
      setDiscordClientSecret("");
      setDiscordFormOpen(false);
    } catch (err) {
      setDiscordError(formatError(err));
    } finally {
      setDiscordBusy(null);
    }
  };

  const handleCopyRedirect = () => {
    const uri = discordStatus?.redirectUri || "http://127.0.0.1";
    void navigator.clipboard?.writeText(uri).catch(() => {});
    setRedirectCopied(true);
    if (copiedTimerRef.current !== null) {
      window.clearTimeout(copiedTimerRef.current);
    }
    copiedTimerRef.current = window.setTimeout(() => {
      copiedTimerRef.current = null;
      setRedirectCopied(false);
    }, 1500);
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
                    onClick={() => {
                      if (discordStatus.viaCentral) {
                        void handleDiscordConnectCentral();
                      } else {
                        setDiscordFormOpen((prev) => !prev);
                      }
                    }}
                  >
                    {discordBusy === "connect"
                      ? t("discord.rpcConnecting")
                      : t("discord.rpcReconnect")}
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={discordBusy !== null}
                    onClick={() => setDiscordFormOpen((prev) => !prev)}
                  >
                    {discordFormOpen
                      ? t("discord.rpcHideCustomApp")
                      : discordStatus.viaCentral
                        ? t("discord.rpcUseCustomApp")
                        : t("discord.rpcEdit")}
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
                <>
                  <Button
                    variant="primary"
                    disabled={discordBusy !== null}
                    onClick={() => void handleDiscordConnectCentral()}
                  >
                    {discordBusy === "connect"
                      ? t("discord.rpcConnecting")
                      : t("discord.rpcConnect")}
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={discordBusy !== null}
                    onClick={() => setDiscordFormOpen((prev) => !prev)}
                  >
                    {discordFormOpen
                      ? t("discord.rpcHideCustomApp")
                      : t("discord.rpcUseCustomApp")}
                  </Button>
                </>
              )}
            </div>
          </SettingRow>

          {discordFormOpen && (
            <form className="cc-discord-setup" onSubmit={(e) => void handleDiscordConnect(e)}>
              <div className="cc-discord-guide">
                <span>{t("discord.rpcGuideStep1")}</span>
                <div className="cc-discord-redirect-row">
                  <span>{t("discord.rpcGuideStep2")}</span>
                  <code className="cc-discord-uri">
                    {discordStatus?.redirectUri || "http://127.0.0.1"}
                  </code>
                  <button
                    type="button"
                    className="cc-discord-copy"
                    onClick={handleCopyRedirect}
                  >
                    {redirectCopied ? <Check size={12} /> : <Copy size={12} />}
                    <span>{redirectCopied ? t("discord.rpcCopied") : t("discord.rpcCopy")}</span>
                  </button>
                </div>
                <span>{t("discord.rpcGuideStep3")}</span>
              </div>

              <div className="cc-discord-fields">
                <label className="cc-discord-field">
                  <span className="cc-field-label">{t("discord.rpcClientId")}</span>
                  <input
                    type="text"
                    className="cc-input"
                    value={discordClientId}
                    onChange={(e) => setDiscordClientId(e.target.value)}
                    placeholder="1234567890123456789"
                    autoComplete="off"
                    disabled={discordBusy !== null}
                  />
                </label>
                <label className="cc-discord-field">
                  <span className="cc-field-label">{t("discord.rpcClientSecret")}</span>
                  <input
                    type="password"
                    className="cc-input"
                    value={discordClientSecret}
                    onChange={(e) => setDiscordClientSecret(e.target.value)}
                    placeholder={
                      discordStatus?.hasSecret ? t("discord.rpcClientSecretSaved") : "••••••••"
                    }
                    autoComplete="off"
                    disabled={discordBusy !== null}
                  />
                </label>
              </div>

              <div className="cc-discord-actions">
                <Button
                  type="submit"
                  variant="primary"
                  disabled={
                    discordBusy !== null ||
                    !discordClientId.trim() ||
                    (!discordClientSecret.trim() && !discordStatus?.hasSecret)
                  }
                >
                  {discordBusy === "connect"
                    ? t("discord.rpcConnecting")
                    : discordStatus?.configured
                      ? t("discord.rpcReconnect")
                      : t("discord.rpcConnect")}
                </Button>
              </div>
            </form>
          )}

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
