import {
  Headset,
  Microphone,
  MicrophoneSlash,
  SpeakerHigh,
  SpeakerSlash,
} from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import type { DiscordParticipant } from "../../../shared/bindings/DiscordParticipant";
import type { DiscordSnapshot } from "../../../shared/bindings/DiscordSnapshot";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import { formatCallTimer } from "../../../shared/format";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { ipc, toAssetUrl } from "../../../shared/ipc";
import { Skeleton, StateEmpty } from "../../components/States";
import "./discord.css";

interface DiscordProps {
  size: WidgetSize;
  theme?: ThemeId;
  preview?: boolean;
  onAriaChange?: (aria: string) => void;
}

export const DISCORD_EXPANDED_MAX_INLINE_ROWS = 5;

export function computeDiscordExpandedHeight(data: DiscordSnapshot | null): number {
  if (!data || !data.inCall) return 76;
  if (data.participants.length === 0) return 86;
  const inlineCount = Math.min(DISCORD_EXPANDED_MAX_INLINE_ROWS, data.participants.length);
  const base = 46;
  const rows = inlineCount * 26 + Math.max(0, inlineCount - 1) * 3;
  const controls = data.canControlVoice ? 30 : 0;
  return Math.max(88, base + rows + controls);
}

function monogramHue(seed: string): number {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i += 1) {
    h ^= seed.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h) % 360;
}

function initialsOf(participant: DiscordParticipant): string {
  const src = (participant.displayName || participant.username || "D").trim();
  return src.charAt(0).toLocaleUpperCase("da-DK") || "D";
}

export function DiscordAvatar({
  participant,
  sizeU = 22,
}: {
  participant: DiscordParticipant;
  sizeU?: number;
}) {
  const [failedPath, setFailedPath] = useState<string | null>(null);
  const path = participant.avatarPath;
  const src = path && path !== failedPath ? toAssetUrl(path) : "";
  const classes = [
    "dc-avatar",
    participant.speaking ? "is-speaking" : "",
    participant.muted || participant.deafened ? "is-muted" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const sizeStyle = {
    width: `calc(${sizeU} * var(--u))`,
    height: `calc(${sizeU} * var(--u))`,
  };

  if (src) {
    return (
      <span className={classes} style={sizeStyle} title={participant.displayName}>
        <img
          src={src}
          alt={participant.displayName}
          draggable={false}
          onError={() => setFailedPath(path)}
        />
      </span>
    );
  }

  const hue = monogramHue(participant.id || participant.username);
  return (
    <span
      className={`${classes} is-monogram`}
      style={{
        ...sizeStyle,
        backgroundColor: `hsl(${hue} 44% 38%)`,
      }}
      title={participant.displayName}
    >
      {initialsOf(participant)}
    </span>
  );
}

export function DiscordAvatarStack({
  participants,
  max = 4,
  sizeU = 20,
}: {
  participants: readonly DiscordParticipant[];
  max?: number;
  sizeU?: number;
}) {
  const shown = participants.slice(0, max);
  const extra = participants.length - shown.length;
  return (
    <div className="dc-avatar-stack" aria-hidden="true">
      {shown.map((p) => (
        <DiscordAvatar key={p.id} participant={p} sizeU={sizeU} />
      ))}
      {extra > 0 && <span className="micro tabular dc-avatar-extra">+{extra}</span>}
    </div>
  );
}

export function DiscordParticipantRow({
  participant,
}: {
  participant: DiscordParticipant;
}) {
  return (
    <div className={`row g8 dc-p-row${participant.speaking ? " is-speaking" : ""}`}>
      <DiscordAvatar participant={participant} sizeU={22} />
      <div className="row g4 grow" style={{ minWidth: 0 }}>
        <span className="dc-p-name trunc">{participant.displayName}</span>
        {participant.isSelf && <span className="micro dc-p-you">{t("discord.you")}</span>}
      </div>
      <div className="dc-p-state">
        {participant.deafened ? (
          <span className="dc-badge is-warn" title={t("discord.deafened")}>
            <SpeakerSlash size="calc(11 * var(--u))" weight="bold" aria-hidden="true" />
          </span>
        ) : participant.muted ? (
          <span className="dc-badge is-warn" title={t("discord.muted")}>
            <MicrophoneSlash size="calc(11 * var(--u))" weight="bold" aria-hidden="true" />
          </span>
        ) : participant.speaking ? (
          <span className="dc-speak-bars" aria-label={t("discord.speaking")}>
            <i />
            <i />
            <i />
          </span>
        ) : null}
      </div>
    </div>
  );
}

export function DiscordVoiceControls({
  selfMute,
  selfDeaf,
  preview = false,
}: {
  selfMute: boolean;
  selfDeaf: boolean;
  preview?: boolean;
}) {
  return (
    <div className="row g6 dc-controls" data-no-expand>
      <button
        type="button"
        className={`dc-ctrl-btn${selfMute ? " is-active" : ""}`}
        aria-pressed={selfMute}
        aria-label={selfMute ? t("discord.unmute") : t("discord.mute")}
        title={selfMute ? t("discord.unmute") : t("discord.mute")}
        onClick={(e) => {
          e.stopPropagation();
          if (preview) return;
          void ipc.discordVoiceAction("toggleMute").catch(() => {});
        }}
      >
        {selfMute ? (
          <MicrophoneSlash size="calc(12 * var(--u))" weight="bold" aria-hidden="true" />
        ) : (
          <Microphone size="calc(12 * var(--u))" weight="bold" aria-hidden="true" />
        )}
        <span className="micro trunc">{t("discord.muted")}</span>
      </button>
      <button
        type="button"
        className={`dc-ctrl-btn${selfDeaf ? " is-active" : ""}`}
        aria-pressed={selfDeaf}
        aria-label={selfDeaf ? t("discord.undeafen") : t("discord.deafen")}
        title={selfDeaf ? t("discord.undeafen") : t("discord.deafen")}
        onClick={(e) => {
          e.stopPropagation();
          if (preview) return;
          void ipc.discordVoiceAction("toggleDeafen").catch(() => {});
        }}
      >
        {selfDeaf ? (
          <SpeakerSlash size="calc(12 * var(--u))" weight="bold" aria-hidden="true" />
        ) : (
          <SpeakerHigh size="calc(12 * var(--u))" weight="bold" aria-hidden="true" />
        )}
        <span className="micro trunc">{t("discord.deafened")}</span>
      </button>
    </div>
  );
}

export function Discord({ size, preview = false, onAriaChange }: DiscordProps) {
  const { data, showSkeleton } = useProvider("discord");
  const [nowMs, setNowMs] = useState(() => Date.now());

  const inCall = Boolean(data?.inCall);
  const running = Boolean(data?.running);

  useEffect(() => {
    if (!inCall || preview) return;
    const id = window.setInterval(() => {
      if (!document.hidden) setNowMs(Date.now());
    }, 1_000);
    return () => window.clearInterval(id);
  }, [inCall, preview]);

  const channelLabel =
    data?.channelName?.trim() ||
    (inCall && data?.mode === "rpc" ? t("discord.privateCall") : t("discord.inCall"));

  const stateLabel = !running
    ? t("discord.closed")
    : inCall
      ? channelLabel
      : t("discord.notInCall");

  useEffect(() => {
    if (!onAriaChange) return;
    onAriaChange(t("discord.aria", { state: stateLabel }));
  }, [stateLabel, onAriaChange]);

  if (!data) {
    if (!showSkeleton) return null;
    return <DiscordSkeleton size={size} />;
  }

  if (!running) {
    return (
      <StateEmpty
        icon={<Headset size="calc(18 * var(--u))" />}
        title={t("discord.closed")}
        subtitle={size === "pill" ? undefined : t("discord.closedBody")}
        compact={size === "pill"}
      />
    );
  }

  const callMs =
    inCall && data.callStartedAt !== null
      ? Math.max(0, nowMs - data.callStartedAt)
      : null;
  const timerStr = callMs !== null ? formatCallTimer(callMs) : null;
  const participants = data.participants ?? [];
  const subLabel =
    timerStr ?? (data.mode === "rpc" ? t("discord.running") : t("discord.fallbackHint"));

  const icon = (
    <div className={`dc-icon${inCall ? " is-call" : ""}`} data-enter="pop">
      <Headset size="calc(16 * var(--u))" weight={inCall ? "fill" : "regular"} />
    </div>
  );

  if (size === "pill") {
    return (
      <div className="row g8 dc-root" style={{ height: "100%" }}>
        {inCall && participants.length > 0 ? (
          <DiscordAvatarStack participants={participants} max={3} sizeU={22} />
        ) : (
          icon
        )}
        <div className="col grow" style={{ gap: "calc(1 * var(--u))", minWidth: 0 }} data-enter>
          <div className="title-s trunc">{stateLabel}</div>
          <div className="micro tabular trunc ink3">{subLabel}</div>
        </div>
        <span className={`dc-dot${inCall ? " is-call" : ""}`} data-enter="pop" />
      </div>
    );
  }

  if (size === "s") {
    return (
      <div className="col between dc-root" style={{ height: "100%" }}>
        <div className="row between g6">
          {inCall && participants.length > 0 ? (
            <DiscordAvatarStack participants={participants} max={3} sizeU={22} />
          ) : (
            icon
          )}
          <span className={`dc-dot${inCall ? " is-call" : ""}`} data-enter="pop" />
        </div>
        <div className="col" style={{ gap: "calc(3 * var(--u))", minWidth: 0 }} data-enter>
          <div className="dc-label trunc" style={{ fontSize: "calc(18 * var(--u))" }}>
            {stateLabel}
          </div>
          <div className="micro tabular trunc ink3">{timerStr ?? t("discord.running")}</div>
        </div>
        <div className="micro trunc ink3" data-enter>
          {inCall && participants.length > 0
            ? t("discord.participantsCount", { n: participants.length })
            : data.mode === "rpc"
              ? t("discord.running")
              : t("discord.fallbackHint")}
        </div>
      </div>
    );
  }

  // Size: "m" (276x132)
  return (
    <div className="row g12 dc-root" style={{ height: "100%", minHeight: 0 }}>
      <div className="col between" style={{ minWidth: 0, flex: participants.length > 0 ? "0 0 44%" : "1 1 0", height: "100%" }}>
        <div className="row g8">
          <span className={`dc-dot${inCall ? " is-call" : ""}`} />
          <span className="meta trunc">{stateLabel}</span>
        </div>
        <div className="col" style={{ gap: "calc(2 * var(--u))", minWidth: 0 }}>
          {timerStr ? (
            <div className="dc-label tabular trunc" style={{ fontSize: "calc(22 * var(--u))" }}>
              {timerStr}
            </div>
          ) : (
            <div className="title-s trunc">{stateLabel}</div>
          )}
          {participants.length > 0 && (
            <div className="micro ink3">
              {t("discord.participantsCount", { n: participants.length })}
            </div>
          )}
        </div>
        {inCall && data.canControlVoice ? (
          <DiscordVoiceControls
            selfMute={data.selfMute}
            selfDeaf={data.selfDeaf}
            preview={preview}
          />
        ) : (
          <div className="micro ink3 dc-m-foot">
            {t(data.mode === "rpc" ? "discord.rpcStatusConnectedShort" : "discord.fallbackHint")}
          </div>
        )}
      </div>

      {participants.length > 0 && (
        <div className="dc-list" style={{ flex: "1 1 0", height: "100%" }}>
          {participants.map((p) => (
            <DiscordParticipantRow key={p.id} participant={p} />
          ))}
        </div>
      )}
    </div>
  );
}

function DiscordSkeleton({ size }: { size: WidgetSize }) {
  if (size === "pill") {
    return (
      <div className="row g8" style={{ height: "100%" }}>
        <Skeleton width={32} height={32} radius={10} />
        <div className="col grow g4">
          <Skeleton width="55%" height={11} />
          <Skeleton width="35%" height={8} />
        </div>
      </div>
    );
  }
  if (size === "s") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <Skeleton width={32} height={32} radius={10} />
        <div className="col g4">
          <Skeleton width="70%" height={16} />
          <Skeleton width="45%" height={9} />
        </div>
        <Skeleton width="85%" height={8} />
      </div>
    );
  }
  return (
    <div className="row g12" style={{ height: "100%" }}>
      <div className="col between grow">
        <Skeleton width={32} height={32} radius={10} />
        <div className="col g4">
          <Skeleton width="60%" height={18} />
          <Skeleton width="40%" height={10} />
        </div>
      </div>
      <div className="col g6" style={{ width: "calc(112 * var(--u))" }}>
        <Skeleton width="100%" height={24} radius={8} />
        <Skeleton width="100%" height={24} radius={8} />
      </div>
    </div>
  );
}
