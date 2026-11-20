import { useEffect, useState } from "react";
import { t } from "../../shared/i18n";
import { Kbd } from "./Kbd";

const CODE_KEY_MAP: Record<string, string> = {
  Space: "Space",
  Minus: "-",
  Equal: "=",
  Comma: ",",
  Period: ".",
  Slash: "/",
  BracketLeft: "[",
  BracketRight: "]",
  Semicolon: ";",
  Quote: "'",
  Backslash: "\\",
  Backquote: "`",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
};

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph", "Dead", "Process"]);

function keyFromCode(event: KeyboardEvent): string | null {
  const { code } = event;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1\d|2[0-4])$/.test(code)) return code;
  if (code.startsWith("Arrow")) return code.slice(5);
  if (code in CODE_KEY_MAP) return CODE_KEY_MAP[code] ?? null;

  const k = event.key;
  if (MODIFIER_KEYS.has(k)) return null;
  if (k === " ") return "Space";
  if (k.length === 1 && /^[\x21-\x7e]$/.test(k)) return k.toUpperCase();
  if (k.startsWith("Arrow")) return k.slice(5);
  return null;
}

function formatKeyEvent(event: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(event.key)) return null;
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  if (modifiers.length === 0) return null;

  const main = keyFromCode(event);
  if (!main) return null;
  return [...modifiers, main].join("+");
}

function extractErrorMessage(err: unknown): string {
  if (
    err &&
    typeof err === "object" &&
    "message" in err &&
    typeof (err as { message: unknown }).message === "string"
  ) {
    return (err as { message: string }).message;
  }
  if (err instanceof Error) return err.message;
  return typeof err === "string" ? err : String(err);
}

export function ShortcutInput({
  value,
  conflictWith,
  onChange,
  ariaLabel,
}: {
  value: string;
  conflictWith?: string;
  onChange: (next: string) => void | Promise<void>;
  ariaLabel?: string;
}) {
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;

    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();

      if (event.key === "Escape") {
        setRecording(false);
        return;
      }

      const combo = formatKeyEvent(event);
      if (!combo) return;

      if (conflictWith && combo.toLowerCase() === conflictWith.toLowerCase()) {
        setError(t("settings.shortcutConflict"));
        return;
      }
      setError(null);
      setRecording(false);
      void Promise.resolve(onChange(combo)).catch((err: unknown) => {
        setError(extractErrorMessage(err));
      });
    };

    const onBlur = () => setRecording(false);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", onBlur);
    };
  }, [recording, conflictWith, onChange]);

  return (
    <div className="cc-shortcut-wrap">
      <button
        type="button"
        className={`cc-shortcut-btn${recording ? " is-recording" : ""}`}
        aria-label={ariaLabel}
        onClick={() => {
          setError(null);
          setRecording((prev) => !prev);
        }}
      >
        {recording ? t("settings.shortcutPrompt") : <Kbd keys={value} />}
      </button>
      {error && <span className="cc-shortcut-err">{error}</span>}
    </div>
  );
}
