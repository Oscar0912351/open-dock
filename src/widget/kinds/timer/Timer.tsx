import { Check, Hourglass, Pause, Play, Plus, X } from "@phosphor-icons/react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import type { RecentTimer } from "../../../shared/bindings/RecentTimer";
import type { TimerItem } from "../../../shared/bindings/TimerItem";
import { t } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import { Ring } from "../system/Ring";
import { formatCountdown, formatDurationShort, formatOvertime } from "./formatDuration";
import { cleanTimerLabel, parseTimerInput } from "./parseTimerInput";
import { sortTimers } from "./sortTimers";
import { useTimers } from "./useTimers";
import "./timer.css";
import "../system/system.css";

export interface TimerSettings {
  sound: "chime" | "marimba" | "crystal";
  volume: number;
  notify: boolean;
}

export const defaultTimerSettings: TimerSettings = {
  sound: "chime",
  volume: 80,
  notify: true,
};

export function resolveTimerSettings(raw: Record<string, unknown> | undefined): TimerSettings {
  if (!raw) return defaultTimerSettings;
  return {
    sound:
      raw.sound === "marimba" || raw.sound === "crystal" || raw.sound === "chime"
        ? raw.sound
        : "chime",
    volume: typeof raw.volume === "number" ? Math.min(100, Math.max(0, raw.volume)) : 80,
    notify: raw.notify === undefined ? true : Boolean(raw.notify),
  };
}

interface TimerProps {
  id: string;
  settings: TimerSettings;
  size: "pill" | "s" | "m" | "l";
  editMode: boolean;
  preview?: boolean;
  onAriaChange?: (aria: string) => void;
  /** Rapporterer om widgetten skal forblive synlig (finished timer eller
      fokuseret input) — bruges af "vis kun når musen er over". */
  onPinVisibleChange?: (pinned: boolean) => void;
}

const MAX_TIMERS = 8;
const FIXED_PRESETS = [1, 3, 5, 10, 15, 30, 60];
const S_PRESETS = [1, 3, 5, 10, 15, 30];
const S_EMPTY_PRESETS = [5, 10, 15, 30];
const ADJUST_WHEEL_MS = 80;

// Række-budget i u — tallene spejler timer.css + WidgetShell-målene
// (indholdshøjde: s/m 104, l 244). Ved ændringer i CSS'ens rækkehøjder,
// input- eller fodhøjde skal de justeres her.
const ROW_H: Record<"s" | "m" | "l" | "c", number> = { s: 32, m: 22, l: 28, c: 26 };
const ROW_H_FINISHED = 40;
const ROW_GAP: Record<"s" | "m" | "l", number> = { s: 4, m: 3, l: 3 };

function formatEndTime(ms: number): string {
  return new Intl.DateTimeFormat("da-DK", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })
    .format(new Date(ms))
    .replaceAll(".", ":");
}

function errorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = (error as { message: unknown }).message;
    if (typeof message === "string" && message) return message;
  }
  return String(error);
}

function chipLabel(minutes: number): string {
  return minutes === 60 ? "1t" : `${minutes}m`;
}

function recentChipLabel(recent: RecentTimer): string {
  return `${recent.label} · ${formatDurationShort(recent.durationMs)}`;
}

export function timerItemTitle(item: TimerItem): string {
  return item.label.trim() || formatDurationShort(item.initialMs);
}

export function timerRestMs(item: TimerItem, nowMs: number): number {
  if (item.status === "running" && item.endsAt !== null) {
    return Math.max(0, item.endsAt - nowMs);
  }
  if (item.status === "paused") return Math.max(0, item.remainingMs);
  return 0;
}

export function timerProgressPct(item: TimerItem, nowMs: number): number {
  if (item.status === "finished") return 0;
  return Math.min(1, Math.max(0, timerRestMs(item, nowMs) / Math.max(1, item.totalMs)));
}

const ICON_PX = 12;

function icon(kind: "play" | "pause" | "check" | "x" | "plus", px = ICON_PX): ReactNode {
  const size = `calc(${px} * var(--u))`;
  switch (kind) {
    case "play":
      return <Play size={size} weight="fill" aria-hidden="true" />;
    case "pause":
      return <Pause size={size} weight="fill" aria-hidden="true" />;
    case "check":
      return <Check size={size} weight="bold" aria-hidden="true" />;
    case "x":
      return <X size={size} weight="bold" aria-hidden="true" />;
    case "plus":
      return <Plus size={size} weight="bold" aria-hidden="true" />;
  }
}

/* ── Inputfelt: readOnly indtil klik, så_focusable + editing (Note-mønster) */

interface TimerInputProps {
  value: string;
  onValueChange: (text: string) => void;
  /** Id som timer-kommandoerne bruger (widget-id eller klynge-modul-id). */
  timerId: string;
  /** Id som widgetSetFocusable bruger ( altid widget-vinduets id). */
  focusId: string;
  placeholder: string;
  disabled?: boolean;
  autoFocus?: boolean;
  showHint?: boolean;
  className?: string;
  error?: string | null;
  onAdded?: () => void;
  onBlur?: () => void;
  onEditingChange?: (editing: boolean) => void;
}

export function TimerInput({
  value,
  onValueChange,
  timerId,
  focusId,
  placeholder,
  disabled = false,
  autoFocus = false,
  showHint = true,
  className = "",
  error: externalError = null,
  onAdded,
  onBlur,
  onEditingChange,
}: TimerInputProps) {
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const didAutoFocus = useRef(false);

  const setEditingState = useCallback(
    (next: boolean) => {
      setEditing(next);
      onEditingChange?.(next);
    },
    [onEditingChange],
  );

  const enterEditing = useCallback(() => {
    if (disabled) return;
    if (!editing) {
      void ipc.widgetSetFocusable(focusId, true).catch(() => {});
      setEditingState(true);
    }
    window.requestAnimationFrame(() => inputRef.current?.focus());
  }, [disabled, editing, focusId, setEditingState]);

  const leaveEditing = useCallback(() => {
    setEditingState(false);
    setError(null);
    void ipc.widgetSetFocusable(focusId, false).catch(() => {});
    onBlur?.();
  }, [focusId, onBlur, setEditingState]);

  // autoFocus bruges af pillens "+"-knap: inputtet mountes og åbnes med det samme.
  useEffect(() => {
    if (autoFocus && !didAutoFocus.current && !disabled) {
      didAutoFocus.current = true;
      enterEditing();
    }
  }, [autoFocus, disabled, enterEditing]);

  const submit = useCallback(async () => {
    const parsed = parseTimerInput(value, new Date());
    if (!parsed) return; // needTime-hint vises allerede
    try {
      await ipc.timerAdd(timerId, parsed.label, parsed.durationMs);
      onValueChange("");
      setError(null);
      inputRef.current?.blur();
      onAdded?.();
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [value, timerId, onValueChange, onAdded]);

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      void submit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      onValueChange("");
      setError(null);
      inputRef.current?.blur();
    }
  };

  const hint = useMemo(() => {
    const shown = error ?? externalError;
    if (shown) return shown;
    if (!value.trim()) return null;
    const parsed = parseTimerInput(value, new Date());
    if (!parsed) return t("timer.needTime");
    const end = formatEndTime(Date.now() + parsed.durationMs);
    return parsed.label
      ? t("timer.preview", {
          label: parsed.label,
          duration: formatDurationShort(parsed.durationMs),
          time: end,
        })
      : t("timer.previewNoLabel", {
          duration: formatDurationShort(parsed.durationMs),
          time: end,
        });
  }, [value, error, externalError]);

  return (
    <span className={`tm-inputwrap${className ? ` ${className}` : ""}`}>
      <input
        ref={inputRef}
        className="tm-input"
        type="text"
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        readOnly={disabled || !editing}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        spellCheck={false}
        aria-label={t("timer.add")}
        onPointerDown={enterEditing}
        onFocus={() => {
          if (!disabled && !editing) {
            void ipc.widgetSetFocusable(focusId, true).catch(() => {});
            setEditingState(true);
          }
        }}
        onBlur={leaveEditing}
        onChange={(event) => {
          onValueChange(event.target.value);
          setError(null);
        }}
        onKeyDown={onKeyDown}
      />
      {showHint && hint !== null && <span className="tm-hint trunc">{hint}</span>}
    </span>
  );
}

/* ── Chips: én linje, højre kant fader ud når der er mere indhold ──── */

function Chips({ className = "", children }: { className?: string; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const [fade, setFade] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () => setFade(el.scrollWidth > el.clientWidth + 1);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  });
  return (
    <div
      ref={ref}
      className={`tm-chips${className ? ` ${className}` : ""}${fade ? " is-fade" : ""}`}
    >
      {children}
    </div>
  );
}

/* ── TimerRow: én timer med titel, tid, progress og handlinger ──────── */

interface TimerRowProps {
  timerId: string;
  item: TimerItem;
  nowMs: number;
  size?: "s" | "m" | "l" | "c";
  interactive?: boolean;
}

export function TimerRow({ timerId, item, nowMs, size = "m", interactive = true }: TimerRowProps) {
  const finished = item.status === "finished";
  const paused = item.status === "paused";
  const rest = timerRestMs(item, nowMs);
  const livePct = timerProgressPct(item, nowMs);
  const overtimeMs = item.finishedAt !== null ? Math.max(0, nowMs - item.finishedAt) : 0;
  const timeText = finished ? formatOvertime(overtimeMs) : formatCountdown(rest);
  const title = timerItemTitle(item);

  // Drain-animationen fryses pr. endsAt: genstarter først ved adjust/resume,
  // ikke ved hvert sekund-tick. `key` på <i> remounter den ved ændring.
  const drain = useMemo(() => {
    if (item.status !== "running" || item.endsAt === null) return null;
    const remaining = Math.max(1, item.endsAt - Date.now());
    return {
      pct: Math.min(1, Math.max(0, remaining / Math.max(1, item.totalMs))),
      durationMs: remaining,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item.id, item.status, item.endsAt, item.totalMs]);

  const wheelAt = useRef(0);
  const onWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (!interactive || finished) return;
    const now = Date.now();
    if (now - wheelAt.current < ADJUST_WHEEL_MS) return;
    wheelAt.current = now;
    const delta = (event.deltaY < 0 ? 1 : -1) * (event.shiftKey ? 300_000 : 60_000);
    void ipc.timerAdjust(timerId, item.id, delta).catch(() => {});
  };

  const adjust = (deltaMs: number) => {
    void ipc.timerAdjust(timerId, item.id, deltaMs).catch(() => {});
  };
  const togglePause = () => {
    const call = paused ? ipc.timerResume(timerId, item.id) : ipc.timerPause(timerId, item.id);
    void call.catch(() => {});
  };
  const remove = () => {
    void ipc.timerRemove(timerId, item.id).catch(() => {});
  };

  const rowTitle = finished
    ? t("timer.overtime", { time: formatCountdown(overtimeMs) })
    : t("timer.adjustHint");

  const barStyle = {
    "--p-live": livePct.toFixed(5),
    ...(drain
      ? {
          "--p0": drain.pct.toFixed(5),
          animation: `tm-drain ${drain.durationMs}ms linear forwards`,
        }
      : {}),
  } as CSSProperties;

  return (
    <div
      className={`tm-row tm-row-${size}${finished ? " is-finished" : ""}${paused ? " is-paused" : ""}`}
      onWheel={onWheel}
      title={interactive ? rowTitle : undefined}
    >
      <div className="tm-row-top">
        <span className="tm-row-title label trunc">{title}</span>
        {paused && <span className="tm-row-pause">{icon("pause", 9)}</span>}
        <span className={`tm-row-time value tabular${finished ? " is-overtime" : ""}`}>
          {timeText}
        </span>
      </div>
      {finished ? (
        /* Færdig: ingen progress-bar — i stedet en handlingslinje med
           +1/+5 og ✓. Rækken er højere end en almindelig (se .is-finished). */
        <div className="tm-row-done">
          <span className="tm-row-done-chips">
            <button
              type="button"
              className="tm-chipbtn tm-chipbtn-mini"
              disabled={!interactive}
              onClick={() => adjust(60_000)}
            >
              {t("timer.plus1")}
            </button>
            <button
              type="button"
              className="tm-chipbtn tm-chipbtn-mini"
              disabled={!interactive}
              onClick={() => adjust(300_000)}
            >
              {t("timer.plus5")}
            </button>
          </span>
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 20 } as CSSProperties}
            aria-label={t("timer.done")}
            disabled={!interactive}
            onClick={remove}
          >
            {icon("check")}
          </button>
        </div>
      ) : (
        <div className="tm-bar" aria-hidden="true">
          <i
            key={drain ? `${item.id}:${item.endsAt}` : `${item.id}:static`}
            className={drain ? "is-running" : undefined}
            style={barStyle}
          />
        </div>
      )}
      {!finished && interactive && (
        <div className="tm-row-actions">
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 22 } as CSSProperties}
            aria-label={paused ? t("timer.resume") : t("timer.pause")}
            onClick={togglePause}
          >
            {paused ? icon("play") : icon("pause")}
          </button>
          <button
            type="button"
            className="tm-chipbtn tm-chipbtn-mini"
            onClick={() => adjust(60_000)}
          >
            {t("timer.plus1")}
          </button>
          <button
            type="button"
            className="od-btn"
            style={{ "--b": 22 } as CSSProperties}
            aria-label={t("timer.remove")}
            onClick={remove}
          >
            {icon("x")}
          </button>
        </div>
      )}
    </div>
  );
}

/* ── Widget ─────────────────────────────────────────────────────────── */

export function Timer({
  id,
  size,
  editMode,
  preview = false,
  onAriaChange,
  onPinVisibleChange,
}: TimerProps) {
  const isPreview = preview || id.startsWith("preview-");
  const { snapshot, nowMs } = useTimers(isPreview ? null : id, !isPreview);
  const items = useMemo(() => snapshot?.items ?? [], [snapshot]);
  const recents = useMemo(() => snapshot?.recents ?? [], [snapshot]);
  const sorted = useMemo(() => sortTimers(items), [items]);
  const full = items.length >= MAX_TIMERS;
  const interactive = !editMode && !isPreview;

  const [inputText, setInputText] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [pillOpen, setPillOpen] = useState(false);
  const [entryActive, setEntryActive] = useState(false);

  // Næste timers drain-animation (pill-ring) — fryses pr. endsAt så den
  // genstarter ved adjust/resume, ikke ved hvert sekund-tick.
  const nextItem = sorted[0];
  const nextDrain = useMemo(() => {
    if (!nextItem || nextItem.status !== "running" || nextItem.endsAt === null) return null;
    const remaining = Math.max(1, nextItem.endsAt - Date.now());
    return {
      pct: Math.min(100, Math.max(0, (remaining / Math.max(1, nextItem.totalMs)) * 100)),
      durationMs: remaining,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nextItem?.id, nextItem?.status, nextItem?.endsAt, nextItem?.totalMs]);

  const inputDisabled = !interactive || full;
  const placeholder = full
    ? t("timer.full")
    : size === "pill" || size === "s"
      ? t("timer.placeholderShort")
      : t("timer.placeholder");

  const inputEl = (extraClass = "", autoFocus = false, onBlurExtra?: () => void) => (
    <TimerInput
      value={inputText}
      onValueChange={(text) => {
        setInputText(text);
        setAddError(null);
      }}
      timerId={id}
      focusId={id}
      placeholder={placeholder}
      disabled={inputDisabled}
      autoFocus={autoFocus}
      showHint
      className={extraClass}
      error={addError}
      onBlur={onBlurExtra}
      onEditingChange={setEntryActive}
    />
  );

  // ── Forslag-chips ──────────────────────────────────────────────────
  // Efter et vellykket chip-klik frigives input-fokusset som ved Enter
  // (blur → leaveEditing → widgetSetFocusable(false)), så tastaturet
  // igen går til brugerens andre apps.
  const releaseFocus = () => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active !== document.body) active.blur();
  };
  const startPreset = (minutes: number) => {
    if (!interactive || full) return;
    const parsed = parseTimerInput(inputText, new Date());
    const label = parsed ? parsed.label : cleanTimerLabel(inputText);
    void ipc
      .timerAdd(id, label, minutes * 60_000)
      .then(() => {
        setInputText("");
        releaseFocus();
      })
      .catch((e) => setAddError(errorMessage(e)));
  };
  const startRecent = (recent: RecentTimer) => {
    if (!interactive || full) return;
    void ipc
      .timerAdd(id, recent.label, recent.durationMs)
      .then(() => {
        setInputText("");
        releaseFocus();
      })
      .catch((e) => setAddError(errorMessage(e)));
  };

  const suggestionChips = (presets: number[], recentCount: number) => (
    <Chips className="tm-suggest">
      {presets.map((minutes) => (
        <button
          key={`p${minutes}`}
          type="button"
          className="tm-chipbtn"
          disabled={inputDisabled}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => startPreset(minutes)}
        >
          {chipLabel(minutes)}
        </button>
      ))}
      {recents.slice(0, recentCount).map((recent, index) => (
        <button
          key={`r${index}`}
          type="button"
          className="tm-chipbtn"
          disabled={inputDisabled}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => startRecent(recent)}
        >
          {recentChipLabel(recent)}
        </button>
      ))}
    </Chips>
  );

  const presetRow = (presets: number[]) => (
    <Chips className="tm-suggest">
      {presets.map((minutes) => (
        <button
          key={`p${minutes}`}
          type="button"
          className="tm-chipbtn"
          disabled={inputDisabled}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => startPreset(minutes)}
        >
          {chipLabel(minutes)}
        </button>
      ))}
    </Chips>
  );

  const recentRow = (count?: number) => {
    const shown = count === undefined ? recents : recents.slice(0, count);
    if (shown.length === 0) return null;
    return (
      <Chips className="tm-suggest tm-recents">
        <span className="micro">{t("timer.recents")}</span>
        {shown.map((recent, index) => (
          <button
            key={`r${index}`}
            type="button"
            className="tm-chipbtn"
            disabled={inputDisabled}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => startRecent(recent)}
          >
            {recentChipLabel(recent)}
          </button>
        ))}
      </Chips>
    );
  };

  // Højdebudget (u) for rækkeområdet —: inputblokkens evt. hint-linje
  // (14u), m's margin-top (8u) og l's fod (23u når der er recents) trækkes
  // fra indholdshøjden. Rækker renderes i sorteret rækkefølge så længe de
  // passer; resten samles i "+{n} flere".
  const hintVisible = addError !== null || inputText.trim().length > 0;
  const rowBudget = (rowSize: "s" | "m" | "l"): number => {
    const hint = hintVisible ? 14 : 0;
    if (rowSize === "s") return 104 - 22 - hint;
    if (rowSize === "m") return 104 - 22 - 8 - hint;
    return 244 - 24 - hint - (recents.length > 0 ? 23 : 0);
  };
  const fitRowCount = (rowSize: "s" | "m" | "l", budget: number): number => {
    const gap = ROW_GAP[rowSize];
    let used = 0;
    let count = 0;
    for (const item of sorted) {
      const height = item.status === "finished" ? ROW_H_FINISHED : ROW_H[rowSize];
      const need = count === 0 ? height : used + gap + height;
      if (count > 0 && need > budget) break;
      used = need;
      count += 1;
    }
    return count;
  };
  const rows = (rowSize: "s" | "m" | "l") => {
    const gap = ROW_GAP[rowSize];
    let count = fitRowCount(rowSize, rowBudget(rowSize));
    if (sorted.length - count > 0) {
      // "+{n} flere"-linjen (12u) + række-gap skal også være plads til.
      count = Math.max(1, fitRowCount(rowSize, rowBudget(rowSize) - 12 - gap));
    }
    const visible = sorted.slice(0, count);
    const hidden = sorted.length - visible.length;
    return (
      <>
        {visible.map((item) => (
          <TimerRow
            key={item.id}
            timerId={id}
            item={item}
            nowMs={nowMs}
            size={rowSize}
            interactive={interactive}
          />
        ))}
        {hidden > 0 && <span className="micro tm-more">{t("timer.more", { n: hidden })}</span>}
      </>
    );
  };

  // ── Aria-label til widget-chrome ───────────────────────────────────
  useEffect(() => {
    if (!onAriaChange) return;
    const next = sorted[0];
    if (!next) {
      onAriaChange(`${t("timer.name")}: ${t("timer.ariaEmpty")}`);
      return;
    }
    const label = timerItemTitle(next);
    if (next.status === "finished") {
      onAriaChange(`${t("timer.name")}: ${t("timer.ariaFinished", { label })}`);
      return;
    }
    onAriaChange(
      t("timer.ariaSummary", {
        n: items.length,
        label,
        time: formatCountdown(timerRestMs(next, nowMs)),
      }),
    );
  }, [sorted, items.length, nowMs, onAriaChange]);

  // Pin til "vis kun når musen er over": en finished timer (indtil den er
  // klaret) eller et fokuseret input holder widgetten fremme.
  const pinnedVisible = sorted.some((item) => item.status === "finished") || entryActive;
  useEffect(() => {
    onPinVisibleChange?.(pinnedVisible);
  }, [pinnedVisible, onPinVisibleChange]);
  useEffect(() => () => onPinVisibleChange?.(false), [onPinVisibleChange]);

  // ── pill ───────────────────────────────────────────────────────────
  if (size === "pill") {
    const next = sorted[0];
    if (!next || pillOpen) {
      return (
        <section className="tm tm-pill">
          <Hourglass size="calc(16 * var(--u))" className="tm-pill-icon" aria-hidden="true" />
          {inputEl("", pillOpen && Boolean(next), () => setPillOpen(false))}
        </section>
      );
    }

    const finished = next.status === "finished";
    const paused = next.status === "paused";
    const rest = timerRestMs(next, nowMs);
    const overtimeMs = next.finishedAt !== null ? Math.max(0, nowMs - next.finishedAt) : 0;
    const timeText = finished ? formatOvertime(overtimeMs) : formatCountdown(rest);
    const drain = nextDrain;

    const ringPct = finished
      ? 0
      : drain
        ? drain.pct
        : Math.min(100, Math.max(0, (rest / Math.max(1, next.totalMs)) * 100));
    const endEstimate =
      next.status === "running" && next.endsAt !== null
        ? next.endsAt
        : nowMs + next.remainingMs;

    const ringAction = () => {
      if (!interactive) return;
      if (finished) void ipc.timerRemove(id, next.id).catch(() => {});
      else if (paused) void ipc.timerResume(id, next.id).catch(() => {});
      else void ipc.timerPause(id, next.id).catch(() => {});
    };

    return (
      <section className="tm tm-pill">
        <button
          type="button"
          className="tm-ringbtn"
          onClick={ringAction}
          disabled={!interactive}
          aria-label={timerItemTitle(next)}
        >
          <Ring
            key={`${next.id}:${next.endsAt ?? "x"}`}
            size={32}
            stroke={2.8}
            pct={ringPct}
            animateDurationMs={finished ? undefined : (drain?.durationMs ?? Math.max(1, rest))}
            animateRunning={next.status === "running"}
          >
            {finished ? icon("check") : paused ? icon("play") : icon("pause")}
          </Ring>
        </button>
        <span className={`tm-time display-s${finished ? " is-overtime" : ""}`}>{timeText}</span>
        <div className="col tm-pill-meta">
          <span className="label trunc">{timerItemTitle(next)}</span>
          {finished ? (
            <button
              type="button"
              className="tm-chipbtn tm-chipbtn-mini"
              disabled={!interactive}
              onClick={() => void ipc.timerAdjust(id, next.id, 300_000).catch(() => {})}
            >
              {t("timer.plus5")}
            </button>
          ) : sorted.length > 1 ? (
            <span className="micro">{t("timer.more", { n: sorted.length - 1 })}</span>
          ) : (
            <span className="micro">{t("timer.doneAt", { time: formatEndTime(endEstimate) })}</span>
          )}
        </div>
        <button
          type="button"
          className="od-btn tm-pill-add"
          style={{ "--b": 22 } as CSSProperties}
          aria-label={t("timer.add")}
          disabled={!interactive}
          onClick={() => setPillOpen(true)}
        >
          {icon("plus")}
        </button>
      </section>
    );
  }

  // ── s ──────────────────────────────────────────────────────────────
  if (size === "s") {
    return (
      <section className="tm tm-s">
        {inputEl()}
        {entryActive ? (
          <div className="tm-fill">
            {suggestionChips(S_PRESETS, 2)}
          </div>
        ) : sorted.length > 0 ? (
          <div className="tm-fill tm-rows">{rows("s")}</div>
        ) : (
          <div className="tm-fill tm-center">
            <div className="tm-quick tm-quick-s">
              {S_EMPTY_PRESETS.map((minutes) => (
                <button
                  key={minutes}
                  type="button"
                  className="tm-chipbtn"
                  disabled={inputDisabled}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => startPreset(minutes)}
                >
                  {chipLabel(minutes)}
                </button>
              ))}
            </div>
          </div>
        )}
      </section>
    );
  }

  // ── m ──────────────────────────────────────────────────────────────
  if (size === "m") {
    return (
      <section className="tm tm-m">
        {inputEl()}
        <div className="tm-m-body">
          {entryActive ? (
            <div className="tm-fill tm-col">
              {suggestionChips(FIXED_PRESETS, 3)}
            </div>
          ) : sorted.length > 0 ? (
            <div className="tm-fill tm-rows tm-rows-m">{rows("m")}</div>
          ) : (
            <div className="tm-fill tm-col tm-center">
              {presetRow(FIXED_PRESETS)}
              {recentRow()}
            </div>
          )}
        </div>
      </section>
    );
  }

  // ── l ──────────────────────────────────────────────────────────────
  return (
    <section className="tm tm-l">
      {inputEl("tm-input-l")}
      <div className="tm-l-body">
        {sorted.length > 0 ? (
          <div className="tm-fill tm-rows tm-rows-l">{rows("l")}</div>
        ) : (
          <div className="tm-fill tm-col tm-center tm-l-empty">
            <span className="micro">{t("timer.empty")}</span>
            {presetRow(FIXED_PRESETS)}
          </div>
        )}
      </div>
      {recents.length > 0 && <div className="tm-l-foot">{recentRow()}</div>}
    </section>
  );
}
