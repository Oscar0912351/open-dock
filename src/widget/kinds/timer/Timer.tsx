import {
  ArrowCounterClockwise,
  Bell,
  Pause,
  Play,
  SkipForward,
  X,
} from "@phosphor-icons/react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import { t } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import { RollingNumber } from "../clock/RollingNumber";
import { Ring } from "../system/Ring";
import { adjustDuration, formatCountdown, formatOvertime } from "./formatDuration";
import { useTimer } from "./useTimer";
import "./timer.css";

export interface TimerSettings {
  mode: "countdown" | "pomodoro";
  sound: "chime" | "marimba" | "crystal";
  volume: number;
  notify: boolean;
  durationMin: number;
  focusMin: number;
  breakMin: number;
  longBreakMin: number;
  rounds: number;
  autoStartBreak: boolean;
}

export const defaultTimerSettings: TimerSettings = {
  mode: "countdown",
  sound: "chime",
  volume: 80,
  notify: true,
  durationMin: 25,
  focusMin: 25,
  breakMin: 5,
  longBreakMin: 15,
  rounds: 4,
  autoStartBreak: true,
};

export function resolveTimerSettings(raw: Record<string, unknown> | undefined): TimerSettings {
  if (!raw) return defaultTimerSettings;
  const minutes = (key: keyof TimerSettings, fallback: number) =>
    typeof raw[key] === "number" && (raw[key] as number) > 0 ? (raw[key] as number) : fallback;
  return {
    mode: raw.mode === "pomodoro" ? "pomodoro" : "countdown",
    sound:
      raw.sound === "marimba" || raw.sound === "crystal" || raw.sound === "chime"
        ? raw.sound
        : "chime",
    volume: typeof raw.volume === "number" ? Math.min(100, Math.max(0, raw.volume)) : 80,
    notify: raw.notify === undefined ? true : Boolean(raw.notify),
    durationMin: minutes("durationMin", 25),
    focusMin: minutes("focusMin", 25),
    breakMin: minutes("breakMin", 5),
    longBreakMin: minutes("longBreakMin", 15),
    rounds: minutes("rounds", 4),
    autoStartBreak: raw.autoStartBreak === undefined ? true : Boolean(raw.autoStartBreak),
  };
}

interface TimerProps {
  id: string;
  settings: TimerSettings;
  size: "pill" | "s" | "m";
  editMode: boolean;
  preview?: boolean;
  onAriaChange?: (aria: string) => void;
}

const PREVIEW_NOTE = "Hente vasketøj";
const PRESETS_MIN = [5, 10, 25, 45];

function formatEndTime(endsAtMs: number): string {
  return new Intl.DateTimeFormat("da-DK", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })
    .format(new Date(endsAtMs))
    .replaceAll(".", ":");
}

function ic(kind: "play" | "pause" | "reset" | "skip" | "stop" | "bell", px: number): ReactNode {
  const size = `calc(${px} * var(--u))`;
  switch (kind) {
    case "play":
      return <Play size={size} weight="fill" />;
    case "pause":
      return <Pause size={size} weight="fill" />;
    case "reset":
      return <ArrowCounterClockwise size={size} />;
    case "skip":
      return <SkipForward size={size} />;
    case "stop":
      return <X size={size} weight="bold" />;
    case "bell":
      return <Bell size={size} weight="fill" />;
  }
}

export function Timer({ id, settings, size, editMode, preview = false, onAriaChange }: TimerProps) {
  const isPreview = preview || id.startsWith("preview-");
  const { snapshot, nowMs } = useTimer(isPreview ? null : id, !isPreview);

  // ── Note (samme fokus-mønster som note-widgetten) ─────────────────────
  const [note, setNote] = useState("");
  const [noteLoaded, setNoteLoaded] = useState(false);
  const [noteEditing, setNoteEditing] = useState(false);
  const noteInputRef = useRef<HTMLInputElement>(null);
  const noteDebounceRef = useRef<number | null>(null);
  const latestNoteRef = useRef("");

  useEffect(() => {
    if (isPreview) {
      setNoteLoaded(true);
      return;
    }
    let active = true;
    ipc
      .noteLoad(id)
      .then((content) => {
        if (!active) return;
        const firstLine = content.split(/\r?\n/)[0] ?? "";
        setNote(firstLine);
        latestNoteRef.current = firstLine;
        setNoteLoaded(true);
      })
      .catch(() => {
        if (active) setNoteLoaded(true);
      });
    return () => {
      active = false;
      if (noteDebounceRef.current !== null) window.clearTimeout(noteDebounceRef.current);
    };
  }, [id, isPreview]);

  const persistNote = useCallback(
    (value: string) => {
      if (isPreview) return;
      if (noteDebounceRef.current !== null) window.clearTimeout(noteDebounceRef.current);
      noteDebounceRef.current = window.setTimeout(() => {
        noteDebounceRef.current = null;
        void ipc.noteSave(id, value).catch(() => {});
      }, 500);
    },
    [id, isPreview],
  );

  const flushNote = useCallback(() => {
    if (noteDebounceRef.current !== null) {
      window.clearTimeout(noteDebounceRef.current);
      noteDebounceRef.current = null;
      if (!isPreview) void ipc.noteSave(id, latestNoteRef.current).catch(() => {});
    }
  }, [id, isPreview]);

  const updateNote = useCallback(
    (value: string) => {
      const clipped = value.replace(/\r?\n/g, " ").slice(0, 120);
      setNote(clipped);
      latestNoteRef.current = clipped;
      persistNote(clipped);
    },
    [persistNote],
  );

  const enterNoteEditing = useCallback(() => {
    if (editMode || isPreview || noteEditing) return;
    void ipc.widgetSetFocusable(id, true).catch(() => {});
    setNoteEditing(true);
    window.requestAnimationFrame(() => noteInputRef.current?.focus());
  }, [editMode, isPreview, noteEditing, id]);

  const leaveNoteEditing = useCallback(() => {
    setNoteEditing(false);
    flushNote();
    if (!isPreview) void ipc.widgetSetFocusable(id, false).catch(() => {});
  }, [flushNote, id, isPreview]);

  // ── Countdown-varighed i idle ─────────────────────────────────────────
  const [editMs, setEditMs] = useState(settings.durationMin * 60_000);
  const editMsRef = useRef(editMs);
  editMsRef.current = editMs;

  useEffect(() => {
    if (!snapshot || snapshot.status === "idle") {
      const fromSettings = settings.durationMin * 60_000;
      if (editMsRef.current !== fromSettings) setEditMs(fromSettings);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.durationMin]);

  const persistTimerRef = useRef<number | null>(null);
  const changeDuration = useCallback(
    (nextMs: number) => {
      setEditMs(nextMs);
      if (isPreview) return;
      if (persistTimerRef.current !== null) window.clearTimeout(persistTimerRef.current);
      persistTimerRef.current = window.setTimeout(() => {
        persistTimerRef.current = null;
        void ipc
          .widgetUpdate(id, { settings: { ...settings, durationMin: Math.round(nextMs / 60_000) } })
          .catch(() => {});
      }, 900);
    },
    [id, isPreview, settings],
  );
  useEffect(
    () => () => {
      if (persistTimerRef.current !== null) window.clearTimeout(persistTimerRef.current);
    },
    [],
  );

  // ── Afledt visningstilstand ───────────────────────────────────────────
  const status = snapshot?.status ?? "idle";
  const segment = snapshot?.segment ?? "countdown";
  const isPomodoro = settings.mode === "pomodoro";
  const endsAt = snapshot?.endsAt != null ? Number(snapshot.endsAt) : null;
  const finishedAt = snapshot?.finishedAt != null ? Number(snapshot.finishedAt) : null;

  let displayMs: number;
  let overtimeMs = 0;
  if (status === "running" && endsAt !== null) {
    displayMs = Math.max(0, endsAt - nowMs);
  } else if (status === "paused") {
    displayMs = Math.max(0, snapshot ? Number(snapshot.remainingMs) : 0);
  } else if (status === "finished" && finishedAt !== null) {
    displayMs = 0;
    overtimeMs = Math.max(0, nowMs - finishedAt);
  } else if (isPomodoro) {
    displayMs = settings.focusMin * 60_000;
  } else {
    displayMs = editMs;
  }

  const finished = status === "finished";
  const timeText = finished ? formatOvertime(overtimeMs) : formatCountdown(displayMs);
  const endLabel = status === "running" && endsAt !== null ? formatEndTime(endsAt) : null;

  // Ring-parametre fryses på snapshot-tilstanden, så sekund-ticks ikke genstarter
  // CSS-drain-animationen (den kører selvstændigt med varighed = resterende tid).
  const ringProps = useMemo(() => {
    if (!snapshot || status === "idle" || status === "finished") {
      return { pct: 0, durationMs: 0, running: false };
    }
    const total = Math.max(1, Number(snapshot.totalMs));
    if (status === "paused") {
      const remaining = Math.max(0, Number(snapshot.remainingMs));
      return { pct: (remaining / total) * 100, durationMs: 0, running: false };
    }
    const end = snapshot.endsAt != null ? Number(snapshot.endsAt) : null;
    if (end === null) return { pct: 0, durationMs: 0, running: false };
    const remaining = Math.max(0, end - Date.now());
    return { pct: Math.min(100, (remaining / total) * 100), durationMs: remaining, running: true };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot, status]);

  // ── Kommandoer ────────────────────────────────────────────────────────
  const start = useCallback(() => {
    if (isPreview) return;
    const duration = isPomodoro ? undefined : Math.round(editMsRef.current);
    void ipc.timerStart(id, duration).catch(() => {});
  }, [id, isPreview, isPomodoro]);

  const pauseToggle = useCallback(() => {
    if (isPreview) return;
    if (status === "running") void ipc.timerPause(id).catch(() => {});
    else if (status === "paused") void ipc.timerResume(id).catch(() => {});
  }, [id, isPreview, status]);

  const reset = useCallback(() => {
    if (!isPreview) void ipc.timerReset(id).catch(() => {});
  }, [id, isPreview]);

  const dismiss = useCallback(() => {
    if (!isPreview) void ipc.timerDismiss(id).catch(() => {});
  }, [id, isPreview]);

  const snooze = useCallback(() => {
    if (!isPreview) void ipc.timerSnooze(id, 5).catch(() => {});
  }, [id, isPreview]);

  const skip = useCallback(() => {
    if (!isPreview) void ipc.timerSkip(id).catch(() => {});
  }, [id, isPreview]);

  // ── Tekst og aria ─────────────────────────────────────────────────────
  const segmentLabel = isPomodoro
    ? segment === "break"
      ? t("timer.break")
      : segment === "longBreak"
        ? t("timer.longBreak")
        : t("timer.focus")
    : null;

  const effectiveNote = isPreview ? PREVIEW_NOTE : note;
  const title =
    effectiveNote.trim() || (isPomodoro ? (segmentLabel ?? t("timer.pomodoro")) : t("timer.name"));

  useEffect(() => {
    if (!onAriaChange) return;
    const state =
      status === "running"
        ? t("timer.ariaRunning", { time: timeText })
        : finished
          ? t("timer.ariaFinished")
          : status === "paused"
            ? t("timer.ariaPaused", { time: timeText })
            : t("timer.ariaIdle");
    onAriaChange(`${title}: ${state}`);
  }, [onAriaChange, title, status, timeText, finished]);

  const round = snapshot?.round ?? 0;
  const roundPos = segment === "focus" ? Math.max(1, round) : Math.min(settings.rounds, round + 1);
  const doneRounds = segment === "focus" ? Math.max(0, round - 1) : round;

  // ── Scroll-justering af varighed (idle countdown) ─────────────────────
  const canAdjust = status === "idle" && !isPomodoro && !editMode && !isPreview;
  const onWheelAdjust = useCallback(
    (event: WheelEvent) => {
      if (!canAdjust) return;
      const direction = event.deltaY < 0 ? 1 : -1;
      changeDuration(adjustDuration(editMsRef.current, "minutes", direction, event.shiftKey));
    },
    [canAdjust, changeDuration],
  );

  // ── Del-elementer ─────────────────────────────────────────────────────
  const ringClick = () => {
    if (editMode || isPreview) return;
    if (status === "idle") start();
    else if (finished) dismiss();
    else pauseToggle();
  };

  const ringButtonLabel =
    status === "running" ? t("timer.pause") : finished ? t("timer.stop") : t("timer.start");

  /** Ringen ER start/pause-knappen — ikon i centrum (pill) eller tid (s/m). */
  const ringButton = (
    ringSize: number,
    stroke: number,
    center: "icon" | "time",
    iconPx: number,
    timePx: number,
  ) => (
    <button
      type="button"
      className={`tm-ringbtn${finished ? " is-done" : ""}`}
      style={{ width: `calc(${ringSize} * var(--u))`, height: `calc(${ringSize} * var(--u))` }}
      aria-label={ringButtonLabel}
      title={canAdjust && center === "time" ? t("timer.adjustMinutes") : undefined}
      onClick={ringClick}
      onWheel={center === "time" ? onWheelAdjust : undefined}
      data-enter="pop"
    >
      <Ring
        size={ringSize}
        stroke={stroke}
        pct={ringProps.pct}
        animateDurationMs={ringProps.running ? ringProps.durationMs : undefined}
        animateRunning={ringProps.running}
      >
        {center === "time" ? (
          <span
            className={`tm-time${finished ? " is-overtime" : ""}`}
            style={{ fontSize: `calc(${timePx} * var(--u))` }}
          >
            <RollingNumber value={timeText} />
          </span>
        ) : finished ? (
          ic("bell", iconPx)
        ) : status === "running" ? (
          ic("pause", iconPx)
        ) : (
          ic("play", iconPx)
        )}
      </Ring>
    </button>
  );

  const ctlBtn = (
    label: string,
    boxU: number,
    icon: ReactNode,
    onClick: () => void,
    primary = false,
  ) => (
    <button
      type="button"
      className={`od-btn${primary ? " primary" : ""}`}
      style={{ "--b": boxU } as CSSProperties}
      aria-label={label}
      onClick={onClick}
    >
      {icon}
    </button>
  );

  /** Kontrolrække efter reference: reset 24 · pause/start 28|30 primary · skip 24. */
  const controlsRow = (primaryU: number, centerIdle = false) => {
    if (finished) {
      return (
        <div className="row tm-controls" data-enter>
          <button type="button" className="tm-chipbtn tm-snooze" onClick={snooze}>
            {t("timer.snooze")}
          </button>
          {ctlBtn(t("timer.stop"), 24, ic("stop", 11), dismiss)}
        </div>
      );
    }
    if (status === "idle") {
      return (
        <div className={`row tm-controls${centerIdle ? " is-center" : ""}`} data-enter>
          {ctlBtn(t("timer.start"), primaryU, ic("play", Math.round(primaryU * 0.42)), start, true)}
        </div>
      );
    }
    return (
      <div className="row tm-controls" data-enter>
        {ctlBtn(t("timer.reset"), 24, ic("reset", 12), reset)}
        {ctlBtn(
          status === "running" ? t("timer.pause") : t("timer.resume"),
          primaryU,
          ic(status === "running" ? "pause" : "play", Math.round(primaryU * 0.42)),
          pauseToggle,
          true,
        )}
        {isPomodoro ? (
          ctlBtn(t("timer.skip"), 24, ic("skip", 12), skip)
        ) : (
          <span className="tm-ctl-end" />
        )}
      </div>
    );
  };

  const roundDots = isPomodoro ? (
    <span className="tm-rounds" aria-hidden="true">
      {Array.from({ length: settings.rounds }, (_, i) => (
        <span
          key={i}
          className={`tm-dot${i < doneRounds ? " is-done" : ""}${
            i === doneRounds && status !== "idle" && !finished ? " is-active" : ""
          }`}
        />
      ))}
    </span>
  ) : null;

  const noteField = (compact: boolean) => {
    if (!noteLoaded) return null;
    return (
      <input
        ref={noteInputRef}
        type="text"
        className={`tm-note${noteEditing ? " is-editing" : ""}${compact ? " tm-note-compact" : ""}`}
        value={effectiveNote}
        maxLength={120}
        placeholder={t("timer.notePlaceholder")}
        readOnly={!noteEditing}
        data-enter
        onPointerDown={() => {
          if (!noteEditing) enterNoteEditing();
        }}
        onChange={(event) => updateNote(event.target.value)}
        onBlur={() => leaveNoteEditing()}
        onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
          if (event.key === "Escape" || event.key === "Enter") {
            event.preventDefault();
            event.currentTarget.blur();
          }
        }}
      />
    );
  };

  const metaLine = (withPrefix: boolean) => {
    if (finished) return t("timer.finished");
    if (status === "paused") return t("timer.paused");
    if (status === "running" && endLabel) {
      return withPrefix ? t("timer.doneAt", { time: endLabel }) : endLabel;
    }
    if (isPomodoro) {
      return t("timer.pomodoroMeta", { focus: settings.focusMin, break: settings.breakMin });
    }
    return "";
  };

  // ── Layouts ───────────────────────────────────────────────────────────
  const rootClass = `tm tm-${size}${finished ? " is-finished" : ""}${status === "paused" ? " is-paused" : ""}`;

  if (size === "pill") {
    return (
      <section className={rootClass} data-size="pill">
        {ringButton(32, 2.8, "icon", 12, 0)}
        <span
          className={`tm-time display-s tm-pill-time${finished ? " is-overtime" : ""}`}
          onWheel={onWheelAdjust}
          title={canAdjust ? t("timer.adjustMinutes") : undefined}
          data-enter
        >
          <RollingNumber value={timeText} />
        </span>
        <div className="col tm-pill-meta" data-enter>
          <span className="label trunc">{title}</span>
          {finished ? (
            <button type="button" className="tm-chipbtn tm-chipbtn-mini tm-snooze" onClick={snooze}>
              {t("timer.snooze")}
            </button>
          ) : (
            <span className="micro trunc">
              {isPomodoro && status !== "idle"
                ? t("timer.roundOf", { round: roundPos, rounds: settings.rounds })
                : metaLine(false)}
            </span>
          )}
        </div>
      </section>
    );
  }

  if (size === "s") {
    return (
      <section className={rootClass} data-size="s">
        <div className="row between tm-head" data-enter>
          {noteField(true)}
          {isPomodoro && status !== "idle" && (
            <span className="micro tm-roundcount">
              {roundPos}/{settings.rounds}
            </span>
          )}
        </div>
        <div className="grow tm-center">
          {ringButton(60, 5, "time", 0, timeText.length > 5 ? 14 : 18)}
        </div>
        {controlsRow(28, true)}
      </section>
    );
  }

  // m — reference-layout: ring 104u til venstre, kontekst + kontroller til højre.
  return (
    <section className={rootClass} data-size="m">
      <div className="tm-m-left">{ringButton(104, 6, "time", 0, timeText.length > 5 ? 17 : 24)}</div>
      <div className="col tm-m-right">
        <div className="row between" data-enter>
          <span className="title-s trunc">
            {isPomodoro ? (segmentLabel ?? t("timer.pomodoro")) : title}
          </span>
          {isPomodoro && status !== "idle" && !finished ? roundDots : null}
        </div>
        {noteField(false)}
        <div className="micro tm-m-meta" data-enter>
          {metaLine(true) || " "}
        </div>
        <div className="grow" />
        {status === "idle" && !isPomodoro ? (
          <div className="row tm-chips" data-enter>
            {PRESETS_MIN.map((min) => (
              <button
                key={min}
                type="button"
                className={`tm-chipbtn${editMs === min * 60_000 ? " is-active" : ""}`}
                onClick={() => changeDuration(min * 60_000)}
              >
                {min}
              </button>
            ))}
          </div>
        ) : null}
        {controlsRow(30)}
      </div>
    </section>
  );
}
