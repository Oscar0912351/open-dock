import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { CheckSquare, Square } from "@phosphor-icons/react";
import { memo, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import { t, useLocale } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import { fireIpc, reportFailure } from "../../../shared/logging";
import { parseChecklist, resolveNoteSettings, serializeChecklist, type ChecklistRow } from "./Note";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";

export const PREVIEW_CLUSTER_NOTE = "- [x] Havregryn og kaffe\n- [ ] Mælk og æbler\n- [ ] Rugbrød";

// Behold note-tekst på tværs af collapse/expand-remounts så feltet ikke
// blinker tomt mens disken genindlæses.
export const noteTextCache = new Map<string, string>();

const NoteClusterMod = memo(function NoteClusterMod({
  mod,
  mode,
  widgetId,
  editMode,
  preview,
  onModuleFocus,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  widgetId: string;
  editMode: boolean;
  preview?: boolean;
  onModuleFocus?: (moduleId: string, focused: boolean) => void;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveNoteSettings(mod.settings);
  const isPreviewSample = Boolean(preview && widgetId.startsWith("preview-"));
  const [text, setText] = useState(() =>
    isPreviewSample ? PREVIEW_CLUSTER_NOTE : (noteTextCache.get(mod.id) ?? ""),
  );
  const [loaded, setLoaded] = useState(false);
  const [saveFailed, setSaveFailed] = useState(false);
  const saveTimerRef = useRef<number | null>(null);
  const retryTimerRef = useRef<number | null>(null);
  const latestRef = useRef(
    isPreviewSample ? PREVIEW_CLUSTER_NOTE : (noteTextCache.get(mod.id) ?? ""),
  );
  const performSaveRef = useRef<(value: string) => Promise<void>>(async () => {});

  useEffect(() => {
    if (isPreviewSample) {
      return;
    }
    let active = true;
    ipc
      .noteLoad(mod.id)
      .then((content) => {
        if (!active) return;
        noteTextCache.set(mod.id, content);
        setText(content);
        latestRef.current = content;
        setLoaded(true);
      })
      .catch((reason: unknown) => {
        reportFailure(`note_load (${mod.id})`, reason);
        if (active) setLoaded(true);
      });
    return () => {
      active = false;
      if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
      if (retryTimerRef.current !== null) window.clearTimeout(retryTimerRef.current);
    };
  }, [mod.id, isPreviewSample]);

  const performSave = useCallback(
    async (value: string) => {
      if (isPreviewSample) return;
      const clipped = value.slice(0, 4000);
      try {
        await ipc.noteSave(mod.id, clipped);
        setSaveFailed(false);
        if (retryTimerRef.current !== null) {
          window.clearTimeout(retryTimerRef.current);
          retryTimerRef.current = null;
        }
      } catch (reason) {
        reportFailure(`note_save (${mod.id})`, reason);
        setSaveFailed(true);
        if (retryTimerRef.current === null) {
          retryTimerRef.current = window.setTimeout(() => {
            retryTimerRef.current = null;
            void performSaveRef.current(latestRef.current);
          }, 5000);
        }
      }
    },
    [mod.id, isPreviewSample],
  );

  useEffect(() => {
    performSaveRef.current = performSave;
  }, [performSave]);

  const scheduleSave = useCallback(
    (next: string) => {
      const clipped = next.slice(0, 4000);
      setText(clipped);
      latestRef.current = clipped;
      noteTextCache.set(mod.id, clipped);
      if (isPreviewSample) return;
      if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = window.setTimeout(() => {
        saveTimerRef.current = null;
        void performSave(clipped);
      }, 500);
    },
    [mod.id, isPreviewSample, performSave],
  );

  const flushSave = useCallback(() => {
    if (saveTimerRef.current !== null) {
      window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    if (isPreviewSample) return;
    void performSave(latestRef.current);
  }, [isPreviewSample, performSave]);

  const effectiveLoaded = isPreviewSample || loaded;
  const rows = useMemo(() => parseChecklist(text), [text]);
  const nonEmptyRows = rows.filter((r) => r.text.trim().length > 0);
  const doneCount = nonEmptyRows.filter((r) => r.done).length;

  const updateRow = (idx: number, patch: Partial<ChecklistRow>) => {
    const next = [...rows];
    const current = next[idx];
    if (!current) return;
    next[idx] = { ...current, ...patch };
    scheduleSave(serializeChecklist(next));
  };

  const enterEditing = () => {
    if (editMode || preview) return;
    fireIpc(ipc.widgetSetFocusable(widgetId, true), `widget_set_focusable (${widgetId})`);
    onModuleFocus?.(mod.id, true);
  };

  const exitEditing = () => {
    flushSave();
    if (!preview) {
      fireIpc(ipc.widgetSetFocusable(widgetId, false), `widget_set_focusable (${widgetId})`);
    }
    onModuleFocus?.(mod.id, false);
  };

  useEffect(() => () => onModuleFocus?.(mod.id, false), [onModuleFocus, mod.id]);

  const handleKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      (e.currentTarget as HTMLElement).blur();
      exitEditing();
    }
  };

  if (mode === "expanded") {
    return (
      <div
        className="c-mod is-exp c-mod-note-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <span className="label trunc">{settings.title}</span>
          <span className="micro tabular">
            {settings.mode === "checklist" ? `${doneCount}/${nonEmptyRows.length}` : `${text.length}/4000`}
          </span>
        </div>

        {!effectiveLoaded ? null : settings.mode === "checklist" ? (
          <div className="c-note-list" data-no-expand>
            {rows.slice(0, 3).map((row, idx) => (
              <div className={`c-note-row${row.done ? " is-done" : ""}`} key={idx}>
                <button
                  type="button"
                  className="c-note-check"
                  onClick={(e) => {
                    e.stopPropagation();
                    updateRow(idx, { done: !row.done });
                  }}
                >
                  {row.done ? (
                    <CheckSquare size="calc(13 * var(--u))" weight="fill" />
                  ) : (
                    <Square size="calc(13 * var(--u))" />
                  )}
                </button>
                <input
                  type="text"
                  value={row.text}
                  readOnly={editMode}
                  placeholder={idx === 0 ? t("note.placeholder") : ""}
                  onFocus={enterEditing}
                  onBlur={exitEditing}
                  onKeyDown={handleKeyDown}
                  onChange={(e) => updateRow(idx, { text: e.target.value })}
                />
              </div>
            ))}
          </div>
        ) : (
          <textarea
            className="c-note-area"
            data-no-expand
            value={text}
            readOnly={editMode}
            maxLength={4000}
            placeholder={t("note.placeholder")}
            onFocus={enterEditing}
            onBlur={exitEditing}
            onKeyDown={handleKeyDown}
            onChange={(e) => scheduleSave(e.target.value)}
          />
        )}
        {saveFailed && <div className="micro note-save-error">{t("note.saveError")}</div>}
      </div>
    );
  }

  const firstOpen = nonEmptyRows.find((r) => !r.done)?.text || "";
  const compactMeta =
    settings.mode === "checklist"
      ? nonEmptyRows.length > 0
        ? t("cluster.checked", { done: doneCount, total: nonEmptyRows.length })
        : t("note.placeholder")
      : firstOpen || t("note.placeholder");

  return (
    <div
      className="c-mod c-mod-note-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between g8">
        <span className="meta trunc" style={{ color: "var(--ink-2)" }}>
          Note · {settings.title}
        </span>
        <div className="row g6" style={{ flex: "0 1 auto", minWidth: 0 }}>
          <span className="micro trunc">{compactMeta}</span>
          <HintBadge label={t("cluster.more")} />
        </div>
      </div>
    </div>
  );
});

export function NoteClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  return (
    <NoteClusterMod
      mod={mod}
      mode={mode}
      widgetId={shared.widgetId}
      editMode={shared.editMode}
      preview={shared.preview}
      onModuleFocus={shared.onModuleFocus}
      onToggleExpand={onToggleExpand}
    />
  );
}
