import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { Robot } from "@phosphor-icons/react";
import { memo, type MouseEvent } from "react";
import type { AgentsSnapshot } from "../../../shared/bindings/AgentsSnapshot";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import { t, useLocale } from "../../../shared/i18n";
import { AgentIcon } from "./AgentIcon";
import { AgentRow, AgentStatusDot, agentStatusLine, filterAgentSessions, formatAgentSummary, overallAgentStatus, resolveAgentsSettings, statusToneClass, todoRatio } from "./Agents";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";

const AgentsClusterMod = memo(function AgentsClusterMod({
  mod,
  mode,
  data,
  nowMs,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  data: AgentsSnapshot | null;
  nowMs: number;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const resolved = resolveAgentsSettings(mod.settings);
  const allSessions = data?.sessions ?? [];
  const visible = filterAgentSessions(allSessions, resolved.recentWindowMin, nowMs);

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-agents-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="col ag-exp-inner">
          <div className="row between g8" style={{ flex: "none" }}>
            <span className="meta ag-trunc" style={{ color: "var(--ink-2)" }}>
              {visible.length === 0 ? t("agents.empty.title") : formatAgentSummary(visible)}
            </span>
            {visible.length > 0 && <AgentStatusDot status={overallAgentStatus(visible)} />}
          </div>
          {visible.length === 0 ? (
            <span className="micro ink3 ag-empty-copy">{t("agents.empty.body")}</span>
          ) : (
            <div className="ag-list">
              {visible.map((session) => (
                <AgentRow key={session.id} session={session} nowMs={nowMs} preview={preview} />
              ))}
            </div>
          )}
        </div>
      </div>
    );
  }

  if (visible.length === 0) {
    return (
      <div
        className="c-mod c-mod-agents-cmp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row g8">
          <span className="c-discord-ico">
            <Robot size="calc(13 * var(--u))" />
          </span>
          <span className="meta ag-trunc" style={{ color: "var(--ink-3)" }}>
            {t("agents.empty.title")}
          </span>
        </div>
      </div>
    );
  }

  if (visible.length === 1) {
    const only = visible[0]!;
    const tone = statusToneClass(only.status);
    return (
      <div
        className={`c-mod c-mod-agents-cmp${only.todo ? " has-todo" : ""}`}
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row g8">
          <AgentIcon agent={only.agent} variant="compact" />
          <div className="row g6 grow" style={{ minWidth: 0 }}>
            <span className="title-s ag-trunc" style={{ fontSize: "calc(12 * var(--u))" }}>
              {only.agentLabel}
            </span>
            <span
              className={`micro ag-trunc ${only.status === "waiting" ? "ag-status-wait" : "ink3"}`}
            >
              {agentStatusLine(only, nowMs)}
            </span>
          </div>
          {only.todo && (
            <span className="micro tabular ag-todo-chip">
              {t("agents.todoProgress", {
                done: only.todo.done,
                total: only.todo.total,
              })}
            </span>
          )}
          <AgentStatusDot status={only.status} />
          <HintBadge label={t("cluster.more")} />
        </div>
        {only.todo && (
          <div className="c-ag-prog-edge" aria-hidden="true">
            <i
              className={tone || undefined}
              style={{ transform: `scaleX(${todoRatio(only.todo).toFixed(4)})` }}
            />
          </div>
        )}
      </div>
    );
  }

  const distinctAgents = Array.from(new Set(visible.map((s) => s.agent))).slice(0, 3);
  return (
    <div
      className="c-mod c-mod-agents-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row g8">
        <div className="ag-stack">
          {distinctAgents.map((agentId) => (
            <AgentIcon key={agentId} agent={agentId} variant="compact" />
          ))}
        </div>
        <span className="title-s ag-trunc grow" style={{ fontSize: "calc(12 * var(--u))" }}>
          {formatAgentSummary(visible)}
        </span>
        <div className="ag-dots">
          {visible.slice(0, 4).map((session) => (
            <AgentStatusDot key={session.id} status={session.status} />
          ))}
        </div>
        <HintBadge label={t("cluster.more")} />
      </div>
    </div>
  );
});

export function AgentsClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  return (
    <AgentsClusterMod
      mod={mod}
      mode={mode}
      data={shared.agents.data}
      nowMs={shared.agentsNowMs}
      preview={shared.preview}
      onToggleExpand={onToggleExpand}
    />
  );
}
