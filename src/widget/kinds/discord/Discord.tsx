import { Headset } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import { formatDuration } from "../../../shared/format";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { Skeleton, StateEmpty } from "../../components/States";
import "./discord.css";

interface DiscordProps {
  size: WidgetSize;
  theme?: ThemeId;
  onAriaChange?: (aria: string) => void;
}

/**
 * Discord-aktivitet. `inCall` er en lydbaseret heuristik (WASAPI-sessioner fra
 * Discord-processen) – ikke en officiel Discord-status. UI skal derfor altid
 * vise detektionsgrundlaget (t("discord.detection")) som micro-tekst.
 */
export function Discord({ size, onAriaChange }: DiscordProps) {
  const { data, showSkeleton } = useProvider("discord");
  const [nowMs, setNowMs] = useState(() => Date.now());

  const inCall = Boolean(data?.inCall);
  const running = Boolean(data?.running);

  // Call-varighed opdateres roligt (10 s) mens et call er aktivt.
  useEffect(() => {
    if (!inCall) return;
    const id = window.setInterval(() => setNowMs(Date.now()), 10_000);
    return () => window.clearInterval(id);
  }, [inCall]);

  const stateLabel = !running
    ? t("discord.closed")
    : inCall
      ? t("discord.inCall")
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
      ? Math.max(0, nowMs - Number(data.callStartedAt))
      : null;
  const subLabel =
    callMs !== null ? formatDuration(callMs) : t("discord.running");

  const icon = (
    <div className={`dc-icon${inCall ? " is-call" : ""}`} data-enter="pop">
      <Headset size="calc(16 * var(--u))" weight={inCall ? "fill" : "regular"} />
    </div>
  );

  if (size === "pill") {
    return (
      <div className="row g8 dc-root" style={{ height: "100%" }}>
        {icon}
        <div className="col grow" style={{ gap: "calc(1 * var(--u))", minWidth: 0 }} data-enter>
          <div className="title-s trunc">{stateLabel}</div>
          <div className="micro trunc ink3">{subLabel}</div>
        </div>
        <span className={`dc-dot${inCall ? " is-call" : ""}`} data-enter="pop" />
      </div>
    );
  }

  if (size === "s") {
    return (
      <div className="col between dc-root" style={{ height: "100%" }}>
        <div className="row between">
          {icon}
          <span className={`dc-dot${inCall ? " is-call" : ""}`} data-enter="pop" />
        </div>
        <div className="col" style={{ gap: "calc(4 * var(--u))" }} data-enter>
          <div className="dc-label trunc" style={{ fontSize: "calc(20 * var(--u))" }}>
            {stateLabel}
          </div>
          <div className="micro trunc ink3">{subLabel}</div>
        </div>
        <div className="micro trunc ink3" data-enter>
          {t("discord.detection")}
        </div>
      </div>
    );
  }

  // Size: "m" (276x132)
  return (
    <div className="row g12 dc-root" style={{ height: "100%" }}>
      <div className="col between" style={{ minWidth: 0, flex: 1 }}>
        <div className="row g8">
          {icon}
          <span className={`dc-dot${inCall ? " is-call" : ""}`} data-enter="pop" />
        </div>
        <div className="col" style={{ gap: "calc(4 * var(--u))" }} data-enter>
          <div className="dc-label trunc" style={{ fontSize: "calc(24 * var(--u))" }}>
            {stateLabel}
          </div>
          <div className="meta trunc ink3">{subLabel}</div>
        </div>
      </div>
      <div className="col between dc-side">
        <span className="od-chip" data-enter>
          Discord
        </span>
        <div className="micro ink3 dc-detection" data-enter>
          {t("discord.detection")}
        </div>
      </div>
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
      <div className="col between" style={{ width: "calc(96 * var(--u))" }}>
        <Skeleton width={64} height={19} radius={999} />
        <Skeleton width="90%" height={8} />
      </div>
    </div>
  );
}
