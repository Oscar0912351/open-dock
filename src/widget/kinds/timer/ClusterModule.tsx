import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { Hourglass } from "@phosphor-icons/react";
import { memo, useEffect, useMemo, useState, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import type { TimerListSnapshot } from "../../../shared/bindings/TimerListSnapshot";
import { t, useLocale } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import { formatCountdown, formatOvertime } from "./formatDuration";
import { sortTimers } from "./sortTimers";
import { chipLabel, Chips, errorMessage, releaseTimerFocus, TimerInput, TimerRow, timerItemTitle, timerRestMs } from "./Timer";
import { cleanTimerLabel, parseTimerInput } from "./parseTimerInput";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";
import "./timer.css";

const TimerClusterMod = memo(function TimerClusterMod({
  mod,
  mode,
  widgetId,
  editMode,
  preview,
  snapshot,
  nowMs,
  onModuleFocus,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  widgetId: string;
  editMode: boolean;
  preview?: boolean;
  snapshot: TimerListSnapshot | null;
  nowMs: number;
  onModuleFocus?: (moduleId: string, focused: boolean) => void;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const items = useMemo(() => snapshot?.items ?? [], [snapshot]);
  const sorted = useMemo(() => sortTimers(items), [items]);
  const next = sorted[0];
  const [inputText, setInputText] = useState("");
  const [inputEditing, setInputEditing] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);
  const interactive = !editMode && !preview;
  const full = items.length >= 8;

  // Fokus rapporteres til shell, så et showOnHover-modul ikke kollapser
  // mens brugeren skriver (og klyngen forbliver pinnet synlig).
  useEffect(() => {
    onModuleFocus?.(mod.id, inputEditing);
  }, [onModuleFocus, mod.id, inputEditing]);
  useEffect(() => () => onModuleFocus?.(mod.id, false), [onModuleFocus, mod.id]);

  const startPreset = (minutes: number) => {
    if (!interactive || full) return;
    const parsed = parseTimerInput(inputText, new Date());
    const label = parsed ? parsed.label : cleanTimerLabel(inputText);
    void ipc
      .timerAdd(mod.id, label, minutes * 60_000)
      .then(() => {
        setInputText("");
        releaseTimerFocus();
      })
      .catch((e) => setAddError(errorMessage(e)));
  };

  if (mode === "compact") {
    const finished = next?.status === "finished";
    const rest = next ? timerRestMs(next, nowMs) : 0;
    const overtimeMs = next?.finishedAt != null ? Math.max(0, nowMs - next.finishedAt) : 0;
    const timeText = !next
      ? ""
      : finished
        ? formatOvertime(overtimeMs)
        : formatCountdown(rest);
    return (
      <div
        className={`c-mod is-compact c-mod-timer${finished ? " is-finished" : ""}`}
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <div className="row g8">
            <Hourglass size="calc(15 * var(--u))" aria-hidden="true" />
            {!next ? (
              <>
                <span className="label">{t("timer.name")}</span>
                <span className="micro">{t("timer.empty")}</span>
              </>
            ) : (
              <>
                <span className="label trunc">{timerItemTitle(next)}</span>
                <span className={`value tabular${finished ? " tm-overtime-ink" : ""}`}>
                  {timeText}
                </span>
                {sorted.length > 1 && <span className="micro">+{sorted.length - 1}</span>}
              </>
            )}
          </div>
          <HintBadge label={t("cluster.more")} />
        </div>
      </div>
    );
  }

  // Expanded: input øverst, derefter op til 3 rækker + "+n flere" — eller
  // hurtig-preset-chips når der ingen timere er. Højden beregnes i shell
  // (computeTimerExpandedHeight) så intet klippes.
  const finished = next?.status === "finished";
  return (
    <div
      className={`c-mod is-exp c-mod-timer-exp${finished ? " is-finished" : ""}`}
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="col g6 tm-cluster-exp">
        <TimerInput
          value={inputText}
          onValueChange={(text) => {
            setInputText(text);
            setAddError(null);
          }}
          timerId={mod.id}
          focusId={widgetId}
          placeholder={full ? t("timer.full") : t("timer.placeholder")}
          disabled={!interactive || full}
          showHint
          error={addError}
          onEditingChange={setInputEditing}
        />
        {sorted.length > 0 ? (
          <>
            {sorted.slice(0, 3).map((item) => (
              <TimerRow
                key={item.id}
                timerId={mod.id}
                item={item}
                nowMs={nowMs}
                size="c"
                interactive={interactive}
              />
            ))}
            {sorted.length > 3 && (
              <span className="micro tm-more">{t("timer.more", { n: sorted.length - 3 })}</span>
            )}
          </>
        ) : (
          <Chips className="tm-suggest">
            {[1, 3, 5, 10, 15, 30].map((minutes) => (
              <button
                key={`p${minutes}`}
                type="button"
                className="tm-chipbtn"
                disabled={!interactive || full}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => startPreset(minutes)}
              >
                {chipLabel(minutes)}
              </button>
            ))}
          </Chips>
        )}
      </div>
    </div>
  );
});

export function TimerClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  return (
    <TimerClusterMod
      mod={mod}
      mode={mode}
      widgetId={shared.widgetId}
      editMode={shared.editMode}
      preview={shared.preview}
      snapshot={shared.timerSnapshots?.get(mod.id) ?? null}
      nowMs={shared.timerNowMs ?? Date.now()}
      onModuleFocus={shared.onModuleFocus}
      onToggleExpand={onToggleExpand}
    />
  );
}
