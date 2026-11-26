import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { Headset, MicrophoneSlash, SpeakerSlash } from "@phosphor-icons/react";
import { memo, useEffect, useState, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import type { DiscordSnapshot } from "../../../shared/bindings/DiscordSnapshot";
import { formatCallTimer } from "../../../shared/format";
import { t, useLocale } from "../../../shared/i18n";
import { DiscordAvatarStack, DiscordParticipantRow, DiscordVoiceControls } from "./Discord";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";

const DiscordClusterMod = memo(function DiscordClusterMod({
  mod,
  mode,
  data,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  data: DiscordSnapshot | null;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const running = Boolean(data?.running);
  const inCall = Boolean(data?.inCall);
  const participants = data?.participants ?? [];
  const channelLabel =
    data?.channelName?.trim() ||
    (inCall && data?.mode === "rpc" ? t("discord.privateCall") : t("discord.inCall"));
  const label = !running ? t("discord.closed") : inCall ? channelLabel : t("discord.notInCall");

  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!inCall || preview) return;
    const id = window.setInterval(() => {
      if (!document.hidden) setNowMs(Date.now());
    }, 1_000);
    return () => window.clearInterval(id);
  }, [inCall, preview]);

  const callMs =
    inCall && data?.callStartedAt != null
      ? Math.max(0, nowMs - data.callStartedAt)
      : null;
  const timerStr = callMs !== null ? formatCallTimer(callMs) : null;

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-discord-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="col dc-exp-inner">
          <div className="row between g8" style={{ flex: "none", width: "100%" }}>
            <div className="row g6" style={{ minWidth: 0, flex: "1 1 0" }}>
              <span className={`dc-dot${inCall ? " is-call" : ""}`} />
              <span className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
                {label}
              </span>
              {inCall && participants.length > 0 && (
                <span className="micro tabular ink3" style={{ flex: "none" }}>
                  · {participants.length}
                </span>
              )}
            </div>
            {timerStr && (
              <span className="micro tabular ink2" style={{ flex: "none" }}>
                {timerStr}
              </span>
            )}
          </div>

          {inCall && participants.length > 0 ? (
            <>
              <div className="dc-list">
                {participants.map((p) => (
                  <DiscordParticipantRow key={p.id} participant={p} />
                ))}
              </div>
              {data?.canControlVoice && (
                <DiscordVoiceControls
                  selfMute={data.selfMute}
                  selfDeaf={data.selfDeaf}
                  preview={preview}
                />
              )}
            </>
          ) : inCall ? (
            <span className="micro ink3 dc-m-foot">{t("discord.fallbackHint")}</span>
          ) : (
            <span className="micro ink3 dc-m-foot">
              {!running
                ? t("discord.closedBody")
                : data?.mode === "rpc"
                  ? t("discord.running")
                  : t("discord.fallbackHint")}
            </span>
          )}
        </div>
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-discord-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row g8" style={{ width: "100%", minWidth: 0 }}>
        {inCall && participants.length > 0 ? (
          <DiscordAvatarStack participants={participants} max={3} sizeU={20} />
        ) : (
          <span className={`c-discord-ico${inCall ? " is-call" : ""}`}>
            <Headset size="calc(13 * var(--u))" weight={inCall ? "fill" : "regular"} />
          </span>
        )}
        <span className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))", flex: "1 1 0" }}>
          {label}
        </span>
        {timerStr && <span className="micro tabular ink2">{timerStr}</span>}
        {inCall && data?.selfDeaf ? (
          <SpeakerSlash size="calc(11 * var(--u))" className="dc-badge is-warn" weight="bold" />
        ) : inCall && data?.selfMute ? (
          <MicrophoneSlash size="calc(11 * var(--u))" className="dc-badge is-warn" weight="bold" />
        ) : null}
        <span className={`dc-dot${inCall ? " is-call" : ""}`} />
        <HintBadge label={t("cluster.more")} />
      </div>
    </div>
  );
});

export function DiscordClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  return (
    <DiscordClusterMod
      mod={mod}
      mode={mode}
      data={shared.discord.data}
      preview={shared.preview}
      onToggleExpand={onToggleExpand}
    />
  );
}
