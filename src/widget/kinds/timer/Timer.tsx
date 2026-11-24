import {
  ArrowCounterClockwise,
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

function formatEndTime(endsAtMs: number): string {
  return new Intl.DateTimeFormat("da-DK", { hour: "2-digit", minute: "2-digit", hourCycle: "h23" })
    .format(new Date(endsAtMs))
    .replaceAll(".", ":");
}

export function Timer({ id, settings, size, editMode, preview = false, onAriaChange }: TimerProps) {
  const isPreview = preview || id.startsWith("preview-");
  const { snapshot, nowMs } = useTimer(isPreview ? null : id, !isPreview);

  // ── Note ────────────────────────────────────────────────────────────────
  const [note, setNote] = useState("");
  const [noteLoaded, setNoteLoaded] = useState(false);
  const [noteFocused, setNoteFocused] = useState(false);
  const noteInputRef = useRef<HTMLInputElement>(null);
  const noteSaveTimerRef = useRef<number | null>(null);
  const latestNoteRef = useRef("");

  useEffect(() => {
    if (isPreview) return;
    let active = true;
    ipc
      .noteLoad(id)
      .then((content) => {
        if (!active) return;
        // Noten deles fil-navneplads med note-widgetten, men timeren bruger
        // kun første linje — resten er ikke meningsfuldt i en titel.
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
      if (noteSaveTimerRef.current !== null) window.clearTimeout(noteSaveTimerRef.current);
    };
  }, [id, isPreview]);

  const saveNote = useCallback(
    (value: string) => {
      if (isPreview) return;
      if (noteSaveTimerRef.current !== null) window.clearTimeout(noteSaveTimerRef.current);
      noteSaveTimerRef.current = window.setTimeout(() => {
        noteSaveTimerRef.current = null;
        void ipc.noteSave(id, value.slice(0, 4000)).catch(() => {});
      }, 500);
    },
    [id, isPreview],
  );

  const updateNote = useCallback(
    (value: string) => {
      const clipped = value.replace(/\r?\n/g, " ").slice(0, 120);
      setNote(clipped);
      latestNoteRef.current = clipped;
      saveNote(clipped);
    },
    [saveNote],
  );

  const flushNote = useCallback(() => {
    if (noteSaveTimerRef.current !== null) {
      window.clearTimeout(noteSaveTimerRef.current);
      noteSaveTimerRef.current = null;
    }
    if (!isPreview) void ipc.noteSave(id, latestNoteRef.current.slice(0, 4000)).catch(() => {});
  }, [id, isPreview]);

  const enterNoteEditing = useCallback(async () => {
    if (editMode || isPreview) return;
    if (!noteFocused) {
      try {
        await ipc.widgetSetFocusable(id, true);
      } catch {
        // Ignorér — focusable-tilstand er en forbedring, ikke en forudsætning.
      }
      setNoteFocused(true);
    }
    window.requestAnimationFrame(() => noteInputRef.current?.focus());
  }, [editMode, isPreview, noteFocused, id]);

  const leaveNoteEditing = useCallback(() => {
    setNoteFocused(false);
    flushNote();
    if (!isPreview) void ipc.widgetSetFocusable(id, false).catch(() => {});
  }, [flushNote, id, isPreview]);

  // ── Varigheds-opsætning (idle, nedtælling) ──────────────────────────────
  const totalFromSnapshot = snapshot ? Number(snapshot.totalMs) : 0;
  const initialMs = (totalFromSnapshot >= 5000 ? totalFromSnapshot : settings.durationMin * 60_000);
  const [editMs, setEditMs] = useState(initialMs);
  const editMsRef = useRef(editMs);
  editMsRef.current = editMs;

  // Hvis indstillingerne ændres udefra (kontrolcenter) mens vi er i idle, følg med.
  useEffect(() => {
    if (!snapshot || snapshot.status === "idle") {
      const fromSettings = settings.durationMin * 60_000;
      if (totalFromSnapshot < 5000 && editMsRef.current !== fromSettings) {
        setEditMs(fromSettings);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.durationMin]);

  const persistTimerRef = useRef<number | null>(null);
  const changeDuration = useCallback(
    (nextMs: number) => {
      setEditMs(nextMs);
      if (isPreview) return;
      // Husk seneste varighed i indstillingerne — debounced, så scroll ikke spammer config.
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

  const onWheelAdjust = useCallback(
    (part: "minutes" | "seconds") => (event: WheelEvent) => {
      if (editMode || isPreview) return;
      if (!snapshot || snapshot.status !== "idle" || settings.mode !== "countdown") return;
      const direction = event.deltaY < 0 ? 1 : -1;
      changeDuration(adjustDuration(editMsRef.current, part, direction, event.shiftKey));
    },
    [editMode, isPreview, snapshot, settings.mode, changeDuration],
  );

  // ── Afledt visningstilstand ─────────────────────────────────────────────
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

  const timeText = status === "finished" ? formatOvertime(overtimeMs) : formatCountdown(displayMs);
  const timeLong = timeText.length > 5 ? " is-long" : "";
  const endLabel = status === "running" && endsAt !== null ? formatEndTime(endsAt) : null;

  // Ring-progress: CSS-animationen kører selvstændigt ud fra endsAt — den må ikke
  // genstartes ved hvert sekund-tick, så parametrene fryses på snapshot-tilstanden.
  const ringProps = useMemo(() => {
    if (!snapshot || status === "idle") {
      return { pct: 100, durationMs: undefined as number | undefined, running: false };
    }
    if (status === "finished") {
      return { pct: 0, durationMs: undefined, running: false };
    }
    const total = Math.max(1, Number(snapshot.totalMs));
    if (status === "paused") {
      const remaining = Math.max(0, Number(snapshot.remainingMs));
      return { pct: (remaining / total) * 100, durationMs: undefined, running: false };
    }
    const end = snapshot.endsAt != null ? Number(snapshot.endsAt) : null;
    if (end === null) return { pct: 100, durationMs: undefined, running: false };
    const remaining = Math.max(0, end - Date.now());
    return {
      pct: Math.min(100, (remaining / total) * 100),
      durationMs: remaining,
      running: true,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapshot, status]);

  // ── Kommandoer ──────────────────────────────────────────────────────────
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

  // ── Labels og aria ──────────────────────────────────────────────────────
  const segmentLabel = isPomodoro
    ? segment === "focus"
      ? t("timer.focus")
      : segment === "break"
        ? t("timer.break")
        : segment === "longBreak"
          ? t("timer.longBreak")
          : t("timer.focus")
    : null;

  const effectiveNote = isPreview ? PREVIEW_NOTE : note;
  const title = effectiveNote.trim() || (isPomodoro ? (segmentLabel ?? t("timer.pomodoro")) : t("timer.name"));

  useEffect(() => {
    if (!onAriaChange) return;
    const state =
      status === "running"
        ? t("timer.ariaRunning", { time: timeText })
        : status === "finished"
          ? t("timer.ariaFinished")
          : status === "paused"
            ? t("timer.ariaPaused", { time: timeText })
            : t("timer.ariaIdle");
    onAriaChange(`${title}: ${state}`);
  }, [onAriaChange, title, status, timeText]);

  const round = snapshot?.round ?? 0;
  const completedRounds = segment === "focus" ? Math.max(0, round - 1) : round;

  // ── Del-elementer ───────────────────────────────────────────────────────
  const ringIsButton = true;
  const ringButtonLabel =
    status === "running"
      ? t("timer.pause")
      : status === "finished"
        ? t("timer.stop")
        : t("timer.start");

  const onRingClick = () => {
    if (editMode) return;
    if (status === "idle") start();
    else if (status === "finished") dismiss();
    else pauseToggle();
  };

  const ringIcon =
    status === "running" ? (
      <Pause size="calc(11 * var(--u))" weight="fill" />
    ) : status === "finished" ? (
      <X size="calc(11 * var(--u))" weight="bold" />
    ) : (
      <Play size="calc(11 * var(--u))" weight="fill" />
    );

  const ring = (ringSize: number, stroke: number, showTime: boolean, timeClass: string) => (
    <button
      type="button"
      className={`tm-ringbtn${ringIsButton ? "" : ""}`}
      aria-label={ringButtonLabel}
      onClick={onRingClick}
      data-enter="pop"
    >
      <Ring
        size={ringSize}
        stroke={stroke}
        pct={ringProps.pct}
        animateDurationMs={ringProps.durationMs}
        animateRunning={ringProps.running}
      >
        {showTime ? (
          <span className={`tm-time ${timeClass}${timeLong}${status === "finished" ? " is-overtime" : ""}`}>
            <RollingNumber value={timeText} />
          </span>
        ) : (
          ringIcon
        )}
      </Ring>
    </button>
  );

  const noteField = (compact: boolean) =>
    size === "pill" ? null : (
      <input
        ref={noteInputRef}
        type="text"
        className={`tm-note${noteFocused ? " is-focused" : ""}${compact ? " tm-note-compact" : ""}`}
        value={effectiveNote}
        maxLength={120}
        placeholder={t("timer.notePlaceholder")}
        readOnly={!noteFocused}
        data-enter
        onPointerDown={() => {
          if (!noteFocused) void enterNoteEditing();
        }}
        onChange={(event) => updateNote(event.target.value)}
        onBlur={() => leaveNoteEditing()}
        onKeyDown={(event: KeyboardEvent<HTMLInputElement>) => {
          if (event.key === "Escape" || event.key === "Enter") {
            event.preventDefault();
            event.currentTarget.blur();
            leaveNoteEditing();
          }
        }}
      />
    );

  const roundDots = isPomodoro && (status !== "idle" || isPomodoro) ? (
    <span className="tm-rounds" aria-hidden="true">
      {Array.from({ length: settings.rounds }, (_, i) => (
        <span
          key={i}
          className={`tm-dot${i < completedRounds ? " is-done" : ""}${
            i === completedRounds && status !== "idle" ? " is-active" : ""
          }`}
        />
      ))}
    </span>
  ) : null;

  const controlsRow = (big: number, small: number) => {
    if (status === "finished") {
      return (
        <div className="row tm-controls" data-enter>
          <button type="button" className="od-chip tm-snooze" onClick={snooze}>
            {t("timer.snooze")}
          </button>
          <button type="button" className="od-btn" aria-label={t("timer.stop")} onClick={dismiss}>
            <X size="calc(12 * var(--u))" weight="bold" />
          </button>
        </div>
      );
    }
    if (status === "idle") {
      return (
        <div className="row tm-controls" data-enter>
          <button type="button" className="od-btn primary" aria-label={t("timer.start")} onClick={start}>
            <Play size={`calc(${Math.round(big * 0.45)} * var(--u))`} weight="fill" />
          </button>
        </div>
      );
    }
    return (
      <div className="row tm-controls" data-enter>
        <button type="button" className="od-btn" aria-label={t("timer.reset")} onClick={reset}>
          <ArrowCounterClockwise size={`calc(${Math.round(small * 0.5)} * var(--u))`} weight="regular" />
        </button>
        <button
          type="button"
          className="od-btn primary"
          aria-label={status === "running" ? t("timer.pause") : t("timer.resume")}
          onClick={pauseToggle}
        >
          {status === "running" ? (
            <Pause size={`calc(${Math.round(big * 0.45)} * var(--u))`} weight="fill" />
          ) : (
            <Play size={`calc(${Math.round(big * 0.45)} * var(--u))`} weight="fill" />
          )}
        </button>
        {isPomodoro ? (
          <button type="button" className="od-btn" aria-label={t("timer.skip")} onClick={skip}>
            <SkipForward size={`calc(${Math.round(small * 0.5)} * var(--u))`} weight="regular" />
          </button>
        ) : (
          <span className="tm-spacer" />
        )}
      </div>
    );
  };

  const presetChips =
    status === "idle" && !isPomodoro ? (
      <div className="row tm-chips" data-enter>
        {[5, 10, 25, 45].map((min) => (
          <button
            key={min}
            type="button"
            className={`od-chip${editMs === min * 60_000 ? " is-active" : ""}`}
            onClick={() => changeDuration(min * 60_000)}
          >
            {min}
          </button>
        ))}
      </div>
    ) : null;

  // Tiden med scroll-zoner i idle (nedtælling): minutdel og sekunddel justeres hver for sig.
  const editableTime = (className: string) => {
    const text = timeText;
    const colonIdx = text.indexOf(":");
    if (status !== "idle" || isPomodoro || colonIdx < 0) {
      return (
        <span className={`tm-time ${className}${timeLong}${status === "finished" ? " is-overtime" : ""}`}>
          <RollingNumber value={text} />
        </span>
      );
    }
    // Ved h:mm:ss er scroll kun på minut-delen — ellers både min og sek.
    const parts = text.split(":");
    const minutePart = parts.length === 3 ? `${parts[0]}:${parts[1]}` : parts[0];
    const secondPart = parts[parts.length - 1];
    return (
      <span className={`tm-time tm-time-edit ${className}${timeLong}`} role="group" aria-label={t("timer.adjustHint")}>
        <span className="tm-zone" onWheel={onWheelAdjust("minutes")} title={t("timer.adjustMinutes")}>
          {minutePart}
        </span>
        <span className="tm-colon">:</span>
        <span className="tm-zone" onWheel={onWheelAdjust("seconds")} title={t("timer.adjustSeconds")}>
          {secondPart}
        </span>
      </span>
    );
  };

  // ── Layouts ─────────────────────────────────────────────────────────────
  const rootClass = `tm tm-${size}${status === "finished" ? " is-finished" : ""}${status === "paused" ? " is-paused" : ""}`;
  const shellStyle: CSSProperties = {};

  if (size === "pill") {
    return (
      <section className={rootClass} style={shellStyle} data-size="pill">
        {ring(32, 2.8, false, "")}
        <div className="tm-pill-mid" data-enter>
          {status === "idle" && !isPomodoro ? (
            editableTime("display-s")
          ) : (
            <span className={`tm-time display-s${status === "finished" ? " is-overtime" : ""}`}>
              <RollingNumber value={timeText} />
            </span>
          )}
        </div>
        <div className="col tm-pill-meta" data-enter>
          <span className="title-s trunc">{title}</span>
          {status === "finished" ? (
            <button type="button" className="od-chip tm-chip-mini tm-snooze" onClick={snooze}>
              {t("timer.snooze")}
            </button>
          ) : (
            <span className="micro ink3 trunc">
              {isPomodoro && status !== "idle"
                ? t("timer.roundOf", { round: Math.max(1, round), rounds: settings.rounds })
                : endLabel
                  ? t("timer.doneAt", { time: endLabel })
                  : segmentLabel ?? ""}
            </span>
          )}
        </div>
      </section>
    );
  }

  if (size === "s") {
    return (
      <section className={rootClass} style={shellStyle} data-size="s">
        <div className="row between tm-head" data-enter>
          {noteLoaded || isPreview ? (
            noteField(true) ?? <span className="label trunc">{title}</span>
          ) : (
            <span className="label trunc">{title}</span>
          )}
          {isPomodoro && status !== "idle" && (
            <span className="micro ink4">
              {Math.max(1, round)}/{settings.rounds}
            </span>
          )}
        </div>
        <div className="grow tm-center">{ring(64, 5.5, true, "tm-time-s")}</div>
        {status === "idle" && !isPomodoro ? (
          <div className="tm-footnote micro ink4" data-enter>
            {t("timer.scrollHint")}
          </div>
        ) : null}
        {controlsRow(28, 24)}
        {roundDots}
      </section>
    );
  }

  // m — hovedstørrelsen: ring til venstre, note + kontekst + kontroller til højre.
  return (
    <section className={rootClass} style={shellStyle} data-size="m">
      <div className="tm-m-left">
        {status === "idle" && !isPomodoro ? (
          <div className="tm-m-idle-ring">
            {ring(96, 6, false, "")}
            <div className="tm-m-idle-time">{editableTime("tm-time-m")}</div>
          </div>
        ) : (
          ring(96, 6, true, "tm-time-m")
        )}
      </div>
      <div className="col tm-m-right">
        <div className="row between" data-enter>
          {isPomodoro ? (
            <span className="label trunc">{segmentLabel ?? t("timer.pomodoro")}</span>
          ) : (
            <span className="label trunc">{t("timer.name")}</span>
          )}
          {isPomodoro && status !== "idle" ? roundDots : null}
        </div>
        {noteLoaded || isPreview ? noteField(false) : null}
        <div className="micro ink3" data-enter>
          {status === "running" && endLabel
            ? t("timer.doneAt", { time: endLabel })
            : status === "finished"
              ? t("timer.finished")
              : status === "paused"
                ? t("timer.paused")
                : isPomodoro
                  ? t("timer.pomodoroMeta", { focus: settings.focusMin, break: settings.breakMin })
                  : " "}
        </div>
        <div className="grow" />
        {presetChips}
        {controlsRow(30, 24)}
      </div>
    </section>
  );
}
