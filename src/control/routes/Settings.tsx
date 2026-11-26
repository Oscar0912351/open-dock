import { Desktop, Moon, PushPin, Sun } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import type { ControlTheme } from "../../shared/bindings/ControlTheme";
import type { CpuSensorStatus } from "../../shared/bindings/CpuSensorStatus";
import type { DiscordRpcStatus } from "../../shared/bindings/DiscordRpcStatus";
import type { Language } from "../../shared/bindings/Language";
import type { Layer } from "../../shared/bindings/Layer";
import type { UpdateProgress } from "../../shared/bindings/UpdateProgress";
import type { UpdateStatus } from "../../shared/bindings/UpdateStatus";
import { getDateTimeFormatter } from "../../shared/format";
import { t } from "../../shared/i18n";
import { DockLogo } from "../../shared/icons/DockLogo";
import { ipc, on, toAssetUrl } from "../../shared/ipc";
import { Button } from "../components/Button";
import { Card, SettingRow } from "../components/Card";
import { ConfirmDialog } from "../components/Drawer";
import { Input } from "../components/Input";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Switch } from "../components/Switch";
import { useControlStore } from "../store";

function formatError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    return String((err as { message: unknown }).message);
  }
  return String(err);
}

function DiscordGlyph({ size = 20 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028 14.09 14.09 0 0 0 1.226-1.994.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.864-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z" />
    </svg>
  );
}

export function Settings() {
  useControlStore((s) => s.locale);
  const config = useControlStore((s) => s.config);
  const appVersion = useControlStore((s) => s.appVersion);

  const widgets = config?.widgets ?? [];
  const widgetLayer: Layer | null = widgets.at(0)?.layer ?? null;
  const layersMixed = widgets.some((w) => w.layer !== widgetLayer);
  const LAYER_HELP: Record<Layer, string> = {
    desktop: t("layer.desktopHelp"),
    top: t("layer.topHelp"),
  };

  const [resetConfirmOpen, setResetConfirmOpen] = useState(false);
  const [sensorStatus, setSensorStatus] = useState<CpuSensorStatus | null>(null);
  const [sensorBusy, setSensorBusy] = useState<"install" | "uninstall" | null>(null);
  const [sensorError, setSensorError] = useState<string | null>(null);

  const [discordStatus, setDiscordStatus] = useState<DiscordRpcStatus | null>(null);
  const [discordBusy, setDiscordBusy] = useState<"connect" | "disconnect" | null>(null);
  const [discordError, setDiscordError] = useState<string | null>(null);
  const [failedDiscordAvatar, setFailedDiscordAvatar] = useState<string | null>(null);
  const [showCustomApp, setShowCustomApp] = useState(false);
  const [customClientId, setCustomClientId] = useState("");
  const [prevDiscordErr, setPrevDiscordErr] = useState("");
  const [prefilledClientId, setPrefilledClientId] = useState<string | null>(null);
  const [customSecret, setCustomSecret] = useState("");
  const [copiedRedirect, setCopiedRedirect] = useState(false);

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
        if (st.connected || st.lastError === null) {
          setDiscordError(null);
        }
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

  // Fold egen-app-formularen ud når Discord afviser scopes (invalid_scope rammer
  // alle der ikke er testere på den ikke-godkendte Central-app).
  const discordErrText = discordError ?? discordStatus?.lastError ?? "";
  if (discordErrText !== prevDiscordErr) {
    setPrevDiscordErr(discordErrText);
    if (discordErrText.toLowerCase().includes("scope")) setShowCustomApp(true);
  }

  if (
    showCustomApp &&
    !customClientId &&
    discordStatus &&
    !discordStatus.viaCentral &&
    discordStatus.clientId &&
    discordStatus.clientId !== prefilledClientId
  ) {
    setPrefilledClientId(discordStatus.clientId);
    setCustomClientId(discordStatus.clientId);
  }

  if (!config) return null;

  const { general } = config;
  const discordRedirectUri = discordStatus?.redirectUri?.trim() || "http://127.0.0.1";
  const customSecretSaved = Boolean(
    discordStatus?.configured && !discordStatus.viaCentral && discordStatus.hasSecret,
  );

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

  const runDiscordConnect = async (connect: () => Promise<DiscordRpcStatus>) => {
    if (discordBusy !== null) return;
    setDiscordBusy("connect");
    setDiscordError(null);
    setFailedDiscordAvatar(null);
    try {
      const next = await connect();
      setDiscordStatus(next);
    } catch (err) {
      setDiscordError(formatError(err));
    } finally {
      setDiscordBusy(null);
    }
  };

  const handleDiscordConnectCentral = () =>
    runDiscordConnect(() => ipc.discordRpcConnectCentral());

  // Egen app: tom secret = behold gemt (Rust-siden genbruger stored creds).
  const handleDiscordConnectCustom = () =>
    runDiscordConnect(async () => {
      const next = await ipc.discordRpcConnect(customClientId, customSecret);
      setCustomSecret("");
      setShowCustomApp(false);
      return next;
    });

  const handleDiscordReconnect = () =>
    runDiscordConnect(() =>
      discordStatus?.viaCentral
        ? ipc.discordRpcConnectCentral()
        : ipc.discordRpcConnect(discordStatus?.clientId ?? "", ""),
    );

  const handleCopyRedirect = async () => {
    try {
      await navigator.clipboard.writeText(discordRedirectUri);
      setCopiedRedirect(true);
      window.setTimeout(() => setCopiedRedirect(false), 1600);
    } catch {
      // Uden clipboard-adgang: brugeren kan stadig markere og kopiere teksten.
    }
  };

  const handleDiscordDisconnect = async () => {
    if (discordBusy !== null) return;
    setDiscordBusy("disconnect");
    setDiscordError(null);
    setFailedDiscordAvatar(null);
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
    : sensorStatus.state === "unsupported"
      ? t("settings.cpuSensorStatusUnsupported")
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
  const needsDriver =
    sensorStatus !== null && sensorStatus.state === "running" && !sensorStatus.driverInstalled;
  const needsInstallOrRepair =
    sensorStatus !== null &&
    !isUnsupported &&
    (needsDriver ||
      sensorStatus.state === "notInstalled" ||
      sensorStatus.state === "stopped" ||
      sensorStatus.outdated);

  const primaryActionLabel =
    sensorBusy === "install"
      ? t("settings.cpuSensorInstalling")
      : needsDriver
        ? t("settings.cpuSensorInstallDriver")
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
    downloading
      ? t("settings.updateStatusDownloading")
      : updateBusy === "check"
        ? t("settings.updateChecking")
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
    downloading && updateInfo?.latest
      ? t("settings.updateHelpDownloading", {
          version: updateInfo.latest.version,
        })
      : updateBusy === "check"
        ? t("settings.updateChecking")
        : updateState === "dev"
          ? t("settings.updateHelpDev")
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

          {widgetLayer !== null && (
            <SettingRow
              title={t("settings.widgetLayer")}
              help={
                layersMixed
                  ? t("settings.widgetLayerMixed")
                  : LAYER_HELP[widgetLayer]
              }
              stacked
            >
              <Segmented<Layer>
                value={widgetLayer}
                ariaLabel={t("settings.widgetLayer")}
                fullWidth
                options={[
                  {
                    value: "desktop",
                    label: t("layer.desktop"),
                    icon: <Desktop size={13} aria-hidden="true" />,
                  },
                  {
                    value: "top",
                    label: t("layer.top"),
                    icon: <PushPin size={13} aria-hidden="true" />,
                  },
                ]}
                onChange={(layer) => {
                  for (const w of widgets) {
                    if (w.layer !== layer) void ipc.widgetUpdate(w.id, { layer });
                  }
                }}
              />
            </SettingRow>
          )}
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
          {(() => {
            const discordLinked = Boolean(discordStatus?.configured);
            const discordConnected = Boolean(discordStatus?.connected);
            const activeDiscordError = discordError || discordStatus?.lastError || null;
            const hasDiscordError = Boolean(activeDiscordError);
            const showDiscordReconnect =
              discordLinked && !discordConnected && (hasDiscordError || discordBusy === "connect");
            const discordTone: "live" | "warn" | "muted" = discordConnected
              ? "live"
              : hasDiscordError
                ? "warn"
                : "muted";
            const discordMetaLabel = discordConnected
              ? `${t("discord.rpcTitle")} · ${t("discord.rpcStatusConnectedShort")}`
              : hasDiscordError
                ? `${t("discord.rpcTitle")} · ${t("discord.rpcStatusErrorShort")}`
                : t("discord.rpcStatusLinkedIdle");
            const discordDisplayName =
              discordStatus?.displayName?.trim() ||
              discordStatus?.username?.trim() ||
              t("discord.rpcTitle");
            const rawHandle = discordStatus?.username?.trim() || "";
            const discordHandle =
              rawHandle && rawHandle.toLowerCase() !== discordDisplayName.toLowerCase()
                ? `@${rawHandle}`
                : null;
            const avatarPath = discordStatus?.avatarPath ?? null;
            const avatarSrc =
              avatarPath && avatarPath !== failedDiscordAvatar ? toAssetUrl(avatarPath) : "";
            const avatarInitial = discordDisplayName.charAt(0).toLocaleUpperCase("da-DK") || "D";

            return (
              <div className="cc-discord-row">
                {discordLinked ? (
                  <div className={`cc-discord-avatar-wrap${avatarSrc ? "" : " is-fallback"}`}>
                    {avatarSrc ? (
                      <img
                        className="cc-discord-avatar"
                        src={avatarSrc}
                        alt={discordDisplayName}
                        draggable={false}
                        onError={() => setFailedDiscordAvatar(avatarPath)}
                      />
                    ) : (
                      <span className="cc-discord-avatar-fallback" aria-hidden="true">
                        {avatarInitial}
                      </span>
                    )}
                  </div>
                ) : (
                  <div className="cc-discord-mark" aria-hidden="true">
                    <DiscordGlyph size={18} />
                  </div>
                )}

                <div className="cc-discord-copy">
                  <div className="cc-discord-name-row">
                    <span className="cc-discord-name">
                      {discordLinked ? discordDisplayName : t("discord.rpcTitle")}
                    </span>
                    {discordLinked && discordHandle && (
                      <span className="cc-discord-handle">{discordHandle}</span>
                    )}
                  </div>

                  {discordLinked ? (
                    <div className={`cc-discord-meta is-${discordTone}`}>
                      <span className="cc-discord-dot" aria-hidden="true" />
                      <span>{discordMetaLabel}</span>
                    </div>
                  ) : (
                    <p className="cc-discord-desc">{t("discord.rpcViaCentralHelp")}</p>
                  )}
                </div>

                <div className="cc-discord-actions">
                  {discordLinked ? (
                    <>
                      {showDiscordReconnect && (
                        <Button
                          variant="ghost"
                          disabled={discordBusy !== null}
                          onClick={() => void handleDiscordReconnect()}
                        >
                          {discordBusy === "connect"
                            ? t("discord.rpcConnecting")
                            : t("discord.rpcReconnect")}
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        disabled={discordBusy !== null}
                        onClick={() => setShowCustomApp((v) => !v)}
                      >
                        {showCustomApp ? t("discord.rpcHideCustomApp") : t("discord.rpcEdit")}
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
                        disabled={discordBusy !== null || discordStatus === null}
                        onClick={() => void handleDiscordConnectCentral()}
                      >
                        {discordBusy === "connect"
                          ? t("discord.rpcConnecting")
                          : t("discord.rpcConnect")}
                      </Button>
                      <Button
                        variant="ghost"
                        disabled={discordBusy !== null}
                        onClick={() => setShowCustomApp((v) => !v)}
                      >
                        {showCustomApp
                          ? t("discord.rpcHideCustomApp")
                          : t("discord.rpcUseCustomApp")}
                      </Button>
                    </>
                  )}
                </div>
              </div>
            );
          })()}

          {showCustomApp && (
            <div className="cc-discord-custom">
              <p className="cc-discord-desc">{t("discord.rpcHelpDisconnected")}</p>
              <div className="cc-discord-guide">
                <span>{t("discord.rpcGuideStep1")}</span>
                <div>
                  <span>{t("discord.rpcGuideStep2")}</span>
                  <span className="cc-discord-redirect">
                    <code className="cc-mono">{discordRedirectUri}</code>
                    <button
                      type="button"
                      className="cc-link-btn"
                      onClick={() => void handleCopyRedirect()}
                    >
                      {copiedRedirect ? t("discord.rpcCopied") : t("discord.rpcCopy")}
                    </button>
                  </span>
                </div>
                <span>{t("discord.rpcGuideStep3")}</span>
              </div>
              <div className="cc-discord-fields">
                <label className="cc-discord-field">
                  <span className="cc-field-label">{t("discord.rpcClientId")}</span>
                  <Input
                    value={customClientId}
                    onChange={(e) => setCustomClientId(e.target.value)}
                    inputMode="numeric"
                    autoComplete="off"
                    spellCheck={false}
                  />
                </label>
                <label className="cc-discord-field">
                  <span className="cc-field-label">{t("discord.rpcClientSecret")}</span>
                  <Input
                    type="password"
                    value={customSecret}
                    onChange={(e) => setCustomSecret(e.target.value)}
                    autoComplete="off"
                    spellCheck={false}
                  />
                  {customSecretSaved && (
                    <span className="cc-field-help">{t("discord.rpcClientSecretSaved")}</span>
                  )}
                </label>
              </div>
              <div className="cc-discord-custom-actions">
                <Button
                  variant="primary"
                  disabled={
                    discordBusy !== null ||
                    !customClientId.trim() ||
                    (!customSecretSaved && !customSecret.trim())
                  }
                  onClick={() => void handleDiscordConnectCustom()}
                >
                  {discordBusy === "connect"
                    ? t("discord.rpcConnecting")
                    : t("discord.rpcConnect")}
                </Button>
              </div>
            </div>
          )}

          {(discordError || discordStatus?.lastError) && (
            <div className="cc-sensor-error" role="alert">
              {discordError || discordStatus?.lastError}
            </div>
          )}
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
