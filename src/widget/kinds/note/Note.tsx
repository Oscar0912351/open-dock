import { CheckSquare, Square } from "@phosphor-icons/react";
import { useCallback, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { t } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import "./note.css";

export interface NoteSettings {
  title: string;
  mode: "text" | "checklist";
  fontSize: "s" | "m" | "l";
}

export const defaultNoteSettings: NoteSettings = {
  title: "Note",
  mode: "text",
  fontSize: "m",
};

export function resolveNoteSettings(raw: Record<string, unknown> | undefined): NoteSettings {
  if (!raw) return defaultNoteSettings;
  const title = typeof raw.title === "string" && raw.title.trim() ? raw.title : "Note";
  const mode = raw.mode === "checklist" ? "checklist" : "text";
  const fontSize = raw.fontSize === "s" || raw.fontSize === "l" ? raw.fontSize : "m";
  return { title, mode, fontSize };
}

export interface ChecklistRow {
  done: boolean;
  text: string;
}

const FONT_UNITS: Record<NoteSettings["fontSize"], number> = {
  s: 13,
  m: 15,
  l: 18,
};

export function parseChecklist(raw: string): ChecklistRow[] {
  if (!raw) return [{ done: false, text: "" }];
  return raw.split(/\r?\n/).map((line) => {
    if (line.startsWith("- [x] ") || line.startsWith("- [X] ")) {
      return { done: true, text: line.slice(6) };
    }
    if (line.startsWith("- [ ] ")) {
      return { done: false, text: line.slice(6) };
    }
    return { done: false, text: line };
  });
}

export function serializeChecklist(rows: ChecklistRow[]): string {
  return rows
    .map((row) => `- [${row.done ? "x" : " "}] ${row.text}`)
    .join("\n")
    .slice(0, 4000);
}

interface NoteProps {
  id: string;
  settings: NoteSettings;
  editMode: boolean;
  preview?: boolean;
  onAriaChange?: (aria: string) => void;
}

const PREVIEW_TEXT = "Klargør præsentation til fredag\nSend status til holdet";
const PREVIEW_CHECKLIST = "- [x] Hent morgenkaffe\n- [ ] Gennemgå widgets\n- [ ] Opdatér tema";

export function Note({ id, settings, editMode, preview = false, onAriaChange }: NoteProps) {
  const isPreviewSample = preview && id.startsWith("preview-");
  const [text, setText] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [focused, setFocused] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);
  const [scrollable, setScrollable] = useState(false);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const rowRefs = useRef<Array<HTMLInputElement | null>>([]);
  const saveTimerRef = useRef<number | null>(null);
  const retryTimerRef = useRef<number | null>(null);
  const latestTextRef = useRef("");
  const performSaveRef = useRef<(value: string) => Promise<void>>(async () => {});

  const effectiveText = isPreviewSample
    ? settings.mode === "checklist"
      ? PREVIEW_CHECKLIST
      : PREVIEW_TEXT
    : text;
  const effectiveLoaded = isPreviewSample || loaded;

  useEffect(() => {
    onAriaChange?.(settings.mode === "checklist" ? `${settings.title} (tjekliste)` : settings.title);
  }, [settings.title, settings.mode, onAriaChange]);

  useEffect(() => {
    if (isPreviewSample) {
      return;
    }
    let active = true;
    ipc
      .noteLoad(id)
      .then((content) => {
        if (!active) return;
        setText(content);
        latestTextRef.current = content;
        setLoaded(true);
      })
      .catch(() => {
        if (active) setLoaded(true);
      });
    return () => {
      active = false;
      if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
      if (retryTimerRef.current !== null) window.clearTimeout(retryTimerRef.current);
    };
  }, [id, isPreviewSample]);

  const performSave = useCallback(
    async (value: string) => {
      if (isPreviewSample) return;
      const clipped = value.slice(0, 4000);
      try {
        await ipc.noteSave(id, clipped);
        setSaveFailed(false);
        if (retryTimerRef.current !== null) {
          window.clearTimeout(retryTimerRef.current);
          retryTimerRef.current = null;
        }
      } catch {
        setSaveFailed(true);
        if (retryTimerRef.current === null) {
          retryTimerRef.current = window.setTimeout(() => {
            retryTimerRef.current = null;
            void performSaveRef.current(latestTextRef.current);
          }, 5000);
        }
      }
    },
    [id, isPreviewSample],
  );

  useEffect(() => {
    performSaveRef.current = performSave;
  }, [performSave]);

  const scheduleSave = useCallback(
    (nextValue: string) => {
      const clipped = nextValue.slice(0, 4000);
      setText(clipped);
      latestTextRef.current = clipped;
      if (isPreviewSample) return;
      if (saveTimerRef.current !== null) {
        window.clearTimeout(saveTimerRef.current);
      }
      saveTimerRef.current = window.setTimeout(() => {
        saveTimerRef.current = null;
        void performSave(clipped);
      }, 500);
    },
    [isPreviewSample, performSave],
  );

  const flushSave = useCallback(() => {
    if (saveTimerRef.current !== null) {
      window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    if (isPreviewSample) return;
    void performSave(latestTextRef.current);
  }, [isPreviewSample, performSave]);

  const enterEditing = useCallback(
    async (focusTarget: () => void) => {
      if (editMode) return;
      if (!focused) {
        if (!preview) {
          try {
            await ipc.widgetSetFocusable(id, true);
          } catch {
            // Ignore focusable toggle failure in non-window preview context.
          }
        }
        setFocused(true);
      }
      window.requestAnimationFrame(focusTarget);
    },
    [editMode, focused, id, preview],
  );

  const leaveEditing = useCallback(() => {
    setFocused(false);
    flushSave();
    if (!preview) {
      void ipc.widgetSetFocusable(id, false).catch(() => {});
    }
  }, [flushSave, id, preview]);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    setScrollable(el.scrollHeight > el.clientHeight + 2);
  }, [effectiveText, settings.fontSize, settings.mode]);

  const fontSizeStyle: CSSProperties = {
    fontSize: `calc(${FONT_UNITS[settings.fontSize]} * var(--u))`,
  };

  const rows = settings.mode === "checklist" ? parseChecklist(effectiveText) : [];
  const doneCount = rows.filter((r) => r.done && r.text.trim().length > 0).length;
  const totalCount = rows.filter((r) => r.text.trim().length > 0).length;
  const headerMeta =
    settings.mode === "checklist" ? `${doneCount}/${totalCount}` : `${effectiveText.length} tegn`;

  const updateRow = (index: number, next: ChecklistRow) => {
    const updated = rows.map((row, idx) => (idx === index ? next : row));
    scheduleSave(serializeChecklist(updated));
  };

  const toggleRow = (index: number) => {
    if (editMode) return;
    const row = rows[index];
    if (!row) return;
    updateRow(index, { ...row, done: !row.done });
  };

  const handleItemKeyDown = (index: number, event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.currentTarget.blur();
      leaveEditing();
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const updated = [
        ...rows.slice(0, index + 1),
        { done: false, text: "" },
        ...rows.slice(index + 1),
      ];
      scheduleSave(serializeChecklist(updated));
      window.requestAnimationFrame(() => {
        rowRefs.current[index + 1]?.focus();
      });
      return;
    }
    if (event.key === "Backspace" && rows[index]?.text === "" && rows.length > 1) {
      event.preventDefault();
      const updated = rows.filter((_, idx) => idx !== index);
      scheduleSave(serializeChecklist(updated));
      const targetIdx = Math.max(0, index - 1);
      window.requestAnimationFrame(() => {
        rowRefs.current[targetIdx]?.focus();
      });
    }
  };

  return (
    <div className="col note-root">
      <div className="row between" data-enter>
        <span className="label trunc">{settings.title}</span>
        <span className="micro ink4">{headerMeta}</span>
      </div>

      <div className={`note-body${focused ? " is-focused" : ""}`} data-enter>
        {!effectiveLoaded ? null : settings.mode === "text" ? (
          <textarea
            ref={textareaRef}
            className={`note-textarea${scrollable ? " is-scrollable" : ""}`}
            style={fontSizeStyle}
            value={effectiveText}
            maxLength={4000}
            placeholder={t("note.placeholder")}
            readOnly={!focused}
            onPointerDown={() => {
              if (!focused) {
                void enterEditing(() => textareaRef.current?.focus());
              }
            }}
            onChange={(event) => scheduleSave(event.target.value)}
            onBlur={() => leaveEditing()}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                event.currentTarget.blur();
                leaveEditing();
              }
            }}
          />
        ) : (
          <div className="col note-list">
            {rows.map((row, idx) => (
              <div key={idx} className="row note-row">
                <button
                  type="button"
                  className={`note-check${row.done ? " is-done" : ""}`}
                  aria-label={t(row.done ? "note.uncheckItem" : "note.checkItem")}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => toggleRow(idx)}
                >
                  {row.done ? (
                    <CheckSquare size="calc(16 * var(--u))" weight="regular" />
                  ) : (
                    <Square size="calc(16 * var(--u))" weight="regular" />
                  )}
                </button>
                <input
                  ref={(el) => {
                    rowRefs.current[idx] = el;
                  }}
                  type="text"
                  className={`note-item-input${row.done ? " is-done" : ""}`}
                  style={fontSizeStyle}
                  value={row.text}
                  placeholder={idx === 0 ? t("note.placeholder") : ""}
                  readOnly={!focused}
                  onPointerDown={() => {
                    if (!focused) {
                      void enterEditing(() => rowRefs.current[idx]?.focus());
                    }
                  }}
                  onChange={(event) => updateRow(idx, { ...row, text: event.target.value })}
                  onKeyDown={(event) => handleItemKeyDown(idx, event)}
                  onBlur={(event) => {
                    const nextTarget = event.relatedTarget as Node | null;
                    if (nextTarget && event.currentTarget.closest(".note-list")?.contains(nextTarget)) {
                      return;
                    }
                    leaveEditing();
                  }}
                />
              </div>
            ))}
          </div>
        )}
      </div>

      {saveFailed && <div className="micro note-save-error">{t("note.saveError")}</div>}
    </div>
  );
}
