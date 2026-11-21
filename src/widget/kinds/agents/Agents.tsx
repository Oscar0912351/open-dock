import { Robot } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import type { AgentSession } from "../../../shared/bindings/AgentSession";
import type { AgentsSnapshot } from "../../../shared/bindings/AgentsSnapshot";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import { formatDuration } from "../../../shared/format";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { Skeleton, StateEmpty } from "../../components/States";
import "./agents.css";

interface AgentsProps {
  size: WidgetSize;
  theme?: ThemeId;
  onAriaChange?: (aria: string) => void;
}

/** Millisekunder før vi igen tæller "siden sidst" for en inaktiv session. */
const TICK_MS = 10_000;

function statusLabel(session: AgentSession, nowMs: number): string {
  if (session.status === "working") {
    return session.detail ?? t("agents.working");
  }
  if (session.detail) return session.detail;
  const ago = Math.max(0, nowMs - Number(session.lastActiveMs));
  return ago < 60_000
    ? t("agents.running")
    : t("agents.idleFor", { dur: formatDuration(ago) });
}

function workingCount(data: AgentsSnapshot | null): number {
  return data?.sessions.filter((s) => s.status === "working").length ?? 0;
}

export function Agents({ size, onAriaChange }: AgentsProps) {
  const { data, showSkeleton } = useProvider("agents");
  const [nowMs, setNowMs] = useState(() => Date.now());

  const hasSessions = Boolean(data && data.sessions.length > 0);

  // Genudregn "siden sidst"-tekster roligt mens der er sessioner at vise.
  useEffect(() => {
    if (!hasSessions) return;
    const id = window.setInterval(() => setNowMs(Date.now()), TICK_MS);
    return () => window.clearInterval(id);
  }, [hasSessions]);

  const count = data?.sessions.length ?? 0;
  const working = workingCount(data);
  const ariaLabel =
    count === 0
      ? t("agents.ariaEmpty")
      : t("agents.aria", { total: count, working });

  useEffect(() => {
    if (!onAriaChange) return;
    onAriaChange(ariaLabel);
  }, [ariaLabel, onAriaChange]);

  if (!data) {
    if (!showSkeleton) return null;
    return <AgentsSkeleton size={size} />;
  }

  if (data.sessions.length === 0) {
    return (
      <StateEmpty
        icon={
          <span className="ag-icon">
            <Robot size={15} />
          </span>
        }
        title={t("agents.empty.title")}
        subtitle={size === "s" ? undefined : t("agents.empty.body")}
        compact={size === "s"}
      />
    );
  }

  if (size === "s") {
    return (
      <div className="col between ag-root" style={{ height: "100%" }}>
        <div className="row between">
          <div className="ag-icon" data-enter="pop">
            <Robot size="calc(15 * var(--u))" />
          </div>
          <span className={`ag-dot${working > 0 ? " is-live" : ""}`} data-enter="pop" />
        </div>
        <div className="col" style={{ gap: "calc(2 * var(--u))" }} data-enter>
          <div className="ag-count">
            <span className="display-s tabular" style={{ fontSize: "calc(26 * var(--u))" }}>
              {count}
            </span>
            <span className="meta ink3">
              {count === 1 ? t("agents.sessionOne") : t("agents.sessionMany")}
            </span>
          </div>
          <div className="micro trunc ink3" data-enter>
            {working > 0 ? t("agents.workingN", { n: working }) : t("agents.allIdle")}
          </div>
        </div>
      </div>
    );
  }

  const maxRows = size === "l" ? 6 : 3;
  const rows = data.sessions.slice(0, maxRows);
  const more = data.sessions.length - rows.length;

  return (
    <div className="col ag-root" style={{ height: "100%", gap: "calc(6 * var(--u))" }}>
      <div className="row between" data-enter>
        <span className="meta ink3">
          {t("agents.summary", { total: count, working })}
        </span>
        <span className={`ag-dot${working > 0 ? " is-live" : ""}`} />
      </div>
      <div className="col grow" style={{ gap: "calc(7 * var(--u))", minHeight: 0 }}>
        {rows.map((session) => (
          <AgentRow key={session.id} session={session} nowMs={nowMs} />
        ))}
        {more > 0 && (
          <span className="micro ink3" data-enter>
            {t("agents.more", { n: more })}
          </span>
        )}
      </div>
    </div>
  );
}

export function AgentRow({
  session,
  nowMs,
}: {
  session: AgentSession;
  nowMs: number;
}) {
  return (
    <div className="row g8 ag-row" data-enter>
      <span className={`ag-dot${session.status === "working" ? " is-live" : ""}`} />
      <div className="col grow" style={{ gap: "calc(1 * var(--u))", minWidth: 0 }}>
        <div className="row g6" style={{ minWidth: 0 }}>
          <span className="title-s trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
            {session.agentLabel}
          </span>
          <span className="micro trunc ink3">{session.project}</span>
        </div>
        <span className="micro trunc ink3">{statusLabel(session, nowMs)}</span>
      </div>
    </div>
  );
}

function AgentsSkeleton({ size }: { size: WidgetSize }) {
  if (size === "s") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <Skeleton width={28} height={28} radius={9} />
        <div className="col g4">
          <Skeleton width="55%" height={20} />
          <Skeleton width="75%" height={9} />
        </div>
      </div>
    );
  }
  return (
    <div className="col g8" style={{ height: "100%" }}>
      <Skeleton width="45%" height={10} />
      {[0, 1, 2].map((i) => (
        <div className="row g8" key={i}>
          <Skeleton width={8} height={8} radius={999} />
          <div className="col grow g4">
            <Skeleton width="60%" height={10} />
            <Skeleton width="40%" height={8} />
          </div>
        </div>
      ))}
    </div>
  );
}
