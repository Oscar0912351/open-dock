import { Robot } from "@phosphor-icons/react";
import { useEffect, useMemo, useState } from "react";
import type { AgentSession } from "../../../shared/bindings/AgentSession";
import type { AgentStatus } from "../../../shared/bindings/AgentStatus";
import type { AgentTodoProgress } from "../../../shared/bindings/AgentTodoProgress";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import { formatDuration } from "../../../shared/format";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import { Skeleton, StateEmpty } from "../../components/States";
import { AgentIcon } from "./AgentIcon";
import "./agents.css";

export interface AgentsSettings {
  recentWindowMin: 0 | 15 | 60;
}

export const defaultAgentsSettings: AgentsSettings = {
  recentWindowMin: 15,
};

export function resolveAgentsSettings(raw?: Record<string, unknown>): AgentsSettings {
  if (!raw) return defaultAgentsSettings;
  const rawWin = raw.recentWindowMin;
  const num =
    typeof rawWin === "number"
      ? rawWin
      : typeof rawWin === "string"
        ? Number(rawWin)
        : NaN;
  const recentWindowMin: 0 | 15 | 60 =
    num === 0 ? 0 : num === 60 ? 60 : 15;
  return { recentWindowMin };
}

interface AgentsProps {
  size: WidgetSize;
  theme?: ThemeId;
  settings?: Partial<AgentsSettings>;
  preview?: boolean;
  onAriaChange?: (aria: string) => void;
}

/** Opdaterer "Klar · X min" / "Pause · X min" roligt mens sessioner er synlige. */
const TICK_MS = 10_000;

export function filterAgentSessions(
  sessions: readonly AgentSession[],
  recentWindowMin: 0 | 15 | 60,
  nowMs: number,
): AgentSession[] {
  const windowMs = recentWindowMin * 60_000;
  return sessions.filter((session) => {
    if (session.status === "working" || session.status === "waiting") {
      return true;
    }
    if (windowMs <= 0) return false;
    return Math.max(0, nowMs - session.lastActiveMs) <= windowMs;
  });
}

export function formatAgentSummary(sessions: readonly AgentSession[]): string {
  let working = 0;
  let waiting = 0;
  let ready = 0;
  let idle = 0;
  for (const s of sessions) {
    if (s.status === "working") working += 1;
    else if (s.status === "waiting") waiting += 1;
    else if (s.status === "ready") ready += 1;
    else idle += 1;
  }

  const parts: string[] = [];
  if (working > 0) parts.push(t("agents.workingCount", { n: working }));
  if (waiting > 0) parts.push(t("agents.waitingCount", { n: waiting }));
  if (ready > 0) parts.push(t("agents.readyCount", { n: ready }));
  if (parts.length > 0) return parts.join(" · ");
  if (idle > 0) return t("agents.idle");
  return t("agents.empty.title");
}

/** Opmærksomheds-hierarki: mangler svar > arbejder > klar > pause. */
export function overallAgentStatus(sessions: readonly AgentSession[]): AgentStatus {
  if (sessions.some((s) => s.status === "waiting")) return "waiting";
  if (sessions.some((s) => s.status === "working")) return "working";
  if (sessions.some((s) => s.status === "ready")) return "ready";
  return "idle";
}

export function statusToneClass(status: AgentStatus): string {
  switch (status) {
    case "working":
      return "is-working";
    case "waiting":
      return "is-waiting";
    case "ready":
      return "is-ready";
    default:
      return "";
  }
}

export function statusTooltip(status: AgentStatus): string {
  switch (status) {
    case "working":
      return t("agents.statusWorking");
    case "waiting":
      return t("agents.statusWaiting");
    case "ready":
      return t("agents.statusReady");
    default:
      return t("agents.statusIdle");
  }
}

export function todoRatio(todo: AgentTodoProgress): number {
  return Math.min(1, Math.max(0, todo.done / Math.max(1, todo.total)));
}

export const AGENTS_EXPANDED_MAX_INLINE_ROWS = 6;

export function computeAgentsExpandedHeight(sessions: readonly AgentSession[]): number {
  if (sessions.length === 0) return 76;
  const inline = sessions.slice(0, AGENTS_EXPANDED_MAX_INLINE_ROWS);
  let rowsH = 0;
  for (const s of inline) {
    rowsH += s.todo ? 38 : 33;
  }
  const gaps = Math.max(0, inline.length - 1) * 2;
  return Math.max(76, 40 + rowsH + gaps);
}

export function agentStatusLine(session: AgentSession, nowMs: number): string {
  if (session.status === "working") {
    return session.todo?.current ?? session.detail ?? t("agents.working");
  }
  if (session.status === "waiting") {
    return session.detail ?? t("agents.waiting");
  }
  const ago = Math.max(0, nowMs - session.lastActiveMs);
  if (session.status === "ready") {
    if (session.todo && ago < 60_000) {
      return t("agents.readyTodos", {
        done: session.todo.done,
        total: session.todo.total,
      });
    }
    return ago < 60_000
      ? t("agents.ready")
      : t("agents.readyAgo", { dur: formatDuration(ago) });
  }
  return ago < 60_000
    ? t("agents.idle")
    : t("agents.idleAgo", { dur: formatDuration(ago) });
}

export function AgentStatusDot({
  status,
  label,
}: {
  status: AgentStatus;
  label?: string;
}) {
  const tone = statusToneClass(status);
  const tip = label ?? statusTooltip(status);
  return (
    <span
      className={tone ? `ag-dot ${tone}` : "ag-dot"}
      title={tip}
      aria-label={tip}
    />
  );
}

export function AgentTodoBar({
  todo,
  status,
}: {
  todo: AgentTodoProgress;
  status: AgentStatus;
}) {
  const tone = statusToneClass(status);
  return (
    <div className="ag-todo-bar" aria-hidden="true">
      <i
        className={tone || undefined}
        style={{ transform: `scaleX(${todoRatio(todo).toFixed(4)})` }}
      />
    </div>
  );
}

export function Agents({
  size,
  settings,
  preview = false,
  onAriaChange,
}: AgentsProps) {
  const { data, showSkeleton } = useProvider("agents");
  const resolved = resolveAgentsSettings(settings);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const rawSessions = data?.sessions;
  const hasTimedSessions = Boolean(
    rawSessions?.some((s) => s.status === "ready" || s.status === "idle"),
  );

  useEffect(() => {
    if (!hasTimedSessions || preview) return;
    const id = window.setInterval(() => {
      if (!document.hidden) setNowMs(Date.now());
    }, TICK_MS);
    return () => window.clearInterval(id);
  }, [hasTimedSessions, preview]);

  const visible = useMemo(
    () =>
      rawSessions
        ? filterAgentSessions(rawSessions, resolved.recentWindowMin, nowMs)
        : [],
    [rawSessions, resolved.recentWindowMin, nowMs],
  );

  const ariaLabel =
    visible.length === 0
      ? t("agents.ariaEmpty")
      : t("agents.aria", { summary: formatAgentSummary(visible) });

  useEffect(() => {
    if (!onAriaChange) return;
    onAriaChange(ariaLabel);
  }, [ariaLabel, onAriaChange]);

  if (!data) {
    if (!showSkeleton) return null;
    return <AgentsSkeleton size={size} />;
  }

  if (visible.length === 0) {
    return (
      <StateEmpty
        icon={
          <span className="ag-icon">
            <Robot size="calc(14 * var(--u))" />
          </span>
        }
        title={t("agents.empty.title")}
        subtitle={size === "s" ? undefined : t("agents.empty.body")}
        compact={size === "s"}
      />
    );
  }

  const topStatus = overallAgentStatus(visible);

  if (size === "s") {
    const activeCount = visible.filter(
      (s) => s.status === "working" || s.status === "waiting",
    ).length;
    const lead = visible[0];
    const heroNum = activeCount > 0 ? activeCount : visible.length;
    const heroUnit =
      activeCount > 0
        ? activeCount === 1
          ? t("agents.activeOne")
          : t("agents.activeMany")
        : visible.length === 1
          ? t("agents.readyOne")
          : t("agents.readyMany");

    const distinctAgents = Array.from(new Set(visible.map((s) => s.agent))).slice(0, 2);
    const subtitle =
      visible.length === 1 && lead
        ? `${lead.agentLabel} · ${agentStatusLine(lead, nowMs)}`
        : formatAgentSummary(visible);

    const inner = (
      <>
        <div className="row between" style={{ width: "100%" }}>
          <div className="ag-stack" data-enter="pop">
            {distinctAgents.map((agentId) => (
              <AgentIcon key={agentId} agent={agentId} size={13} />
            ))}
          </div>
          {visible.length <= 3 ? (
            <div className="ag-dots" data-enter="pop">
              {visible.map((s) => (
                <AgentStatusDot key={s.id} status={s.status} />
              ))}
            </div>
          ) : (
            <AgentStatusDot status={topStatus} />
          )}
        </div>
        <div
          className="col"
          style={{ gap: "calc(2 * var(--u))", minWidth: 0, width: "100%" }}
          data-enter
        >
          <div className="ag-count">
            <span className="display-s tabular" style={{ fontSize: "calc(26 * var(--u))" }}>
              {heroNum}
            </span>
            <span className="meta ink3">{heroUnit}</span>
            {visible.length === 1 && lead?.todo && (
              <span className="micro tabular ag-todo-chip" style={{ marginLeft: "auto" }}>
                {t("agents.todoProgress", {
                  done: lead.todo.done,
                  total: lead.todo.total,
                })}
              </span>
            )}
          </div>
          <div className="micro ag-trunc ink3" data-enter>
            {subtitle}
          </div>
          {visible.length === 1 && lead?.todo && (
            <AgentTodoBar todo={lead.todo} status={lead.status} />
          )}
        </div>
      </>
    );

    if (visible.length === 1 && lead && !preview) {
      return (
        <button
          type="button"
          className="col between ag-root ag-s-btn"
          title={t("agents.focusHint", { agent: lead.agentLabel })}
          onClick={() => {
            void ipc.agentFocus(lead.agent, lead.pid, lead.project).catch(() => {});
          }}
        >
          {inner}
        </button>
      );
    }

    return (
      <div className="col between ag-root" style={{ height: "100%" }}>
        {inner}
      </div>
    );
  }

  return (
    <div className="col ag-root" style={{ height: "100%", minHeight: 0, gap: "calc(5 * var(--u))" }}>
      <div className="row between" style={{ flex: "none" }} data-enter>
        <span className="meta ink3 ag-trunc">{formatAgentSummary(visible)}</span>
        <AgentStatusDot status={topStatus} />
      </div>
      <div className="ag-list" style={{ gap: "calc(3 * var(--u))" }}>
        {visible.map((session) => (
          <AgentRow key={session.id} session={session} nowMs={nowMs} preview={preview} />
        ))}
      </div>
    </div>
  );
}

export function AgentRow({
  session,
  nowMs,
  preview = false,
}: {
  session: AgentSession;
  nowMs: number;
  preview?: boolean;
}) {
  const titleTrimmed = session.title?.trim() ?? "";
  const projectTrimmed = session.project?.trim() ?? "";
  const primary = titleTrimmed || projectTrimmed || session.agentLabel;
  const secondaryCandidate = titleTrimmed
    ? projectTrimmed || session.agentLabel
    : projectTrimmed
      ? session.agentLabel
      : null;
  const secondary =
    secondaryCandidate && secondaryCandidate.toLowerCase() !== primary.toLowerCase()
      ? secondaryCandidate
      : null;

  const line2 = agentStatusLine(session, nowMs);
  const statusClass =
    session.status === "waiting"
      ? "ag-status-wait"
      : session.status === "working"
        ? "ink2"
        : "ink3";

  return (
    <button
      type="button"
      className="row g8 ag-row"
      data-no-expand
      data-enter
      title={preview ? undefined : t("agents.focusHint", { agent: session.agentLabel })}
      onClick={(e) => {
        e.stopPropagation();
        if (preview) return;
        void ipc.agentFocus(session.agent, session.pid, session.project).catch(() => {});
      }}
    >
      <AgentIcon agent={session.agent} size={12} />
      <div className="col grow" style={{ gap: "calc(1 * var(--u))", minWidth: 0 }}>
        <div className="row g6" style={{ minWidth: 0 }}>
          <span className="title-s ag-trunc ag-row-title">{primary}</span>
          {session.todo && (
            <span className="micro tabular ag-todo-chip">
              {t("agents.todoProgress", {
                done: session.todo.done,
                total: session.todo.total,
              })}
            </span>
          )}
        </div>
        <div className="row g4" style={{ minWidth: 0 }}>
          <span className={`micro ag-trunc ag-row-status ${statusClass}`}>{line2}</span>
          {secondary && (
            <>
              <span className="micro ag-row-sep" aria-hidden="true">·</span>
              <span className="micro ag-trunc ink3 ag-row-ctx">{secondary}</span>
            </>
          )}
          {session.subagentCount > 0 && (
            <span className="micro tabular ag-sub-badge">
              {t("agents.subagents", { n: session.subagentCount })}
            </span>
          )}
        </div>
        {session.todo && <AgentTodoBar todo={session.todo} status={session.status} />}
      </div>
      <AgentStatusDot status={session.status} />
    </button>
  );
}

function AgentsSkeleton({ size }: { size: WidgetSize }) {
  if (size === "s") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <Skeleton width={24} height={24} radius={7} />
        <div className="col g4">
          <Skeleton width="55%" height={20} />
          <Skeleton width="75%" height={9} />
        </div>
      </div>
    );
  }
  return (
    <div className="col g6" style={{ height: "100%" }}>
      <Skeleton width="45%" height={10} />
      {[0, 1].map((i) => (
        <div className="row g8" key={i}>
          <Skeleton width={24} height={24} radius={7} />
          <div className="col grow g4">
            <Skeleton width="62%" height={10} />
            <Skeleton width="42%" height={8} />
          </div>
        </div>
      ))}
    </div>
  );
}
