import { CaretDown, Check } from "@phosphor-icons/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  help?: string;
}

interface SelectProps<T extends string> {
  options: ReadonlyArray<SelectOption<T>>;
  value: T;
  onChange: (value: T) => void;
  searchable?: boolean;
  searchPlaceholder?: string;
  placeholder?: string;
  ariaLabel?: string;
  disabled?: boolean;
}

export function Select<T extends string>({
  options,
  value,
  onChange,
  searchable = false,
  searchPlaceholder = "Søg…",
  placeholder = "",
  ariaLabel,
  disabled = false,
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const wrapRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const selected = options.find((opt) => opt.value === value);

  const filtered = useMemo(() => {
    const q = query.trim().toLocaleLowerCase("da-DK");
    if (!q) return options;
    return options.filter(
      (opt) =>
        opt.label.toLocaleLowerCase("da-DK").includes(q) ||
        opt.value.toLocaleLowerCase("da-DK").includes(q),
    );
  }, [options, query]);

  const close = useCallback(() => {
    setOpen(false);
    setQuery("");
  }, []);

  useEffect(() => {
    if (!open) return;
    if (searchable) {
      window.requestAnimationFrame(() => searchRef.current?.focus());
    }
    const onPointerDown = (event: MouseEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) {
        close();
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        close();
      }
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open, searchable, close]);

  return (
    <div ref={wrapRef} className="cc-select">
      <button
        type="button"
        className="cc-select-trigger"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => {
          if (open) close();
          else setOpen(true);
        }}
      >
        <span>{selected?.label ?? placeholder}</span>
        <CaretDown size={14} aria-hidden="true" />
      </button>

      {open && (
        <div className="cc-select-popover" role="listbox" aria-label={ariaLabel}>
          {searchable && (
            <input
              ref={searchRef}
              type="text"
              className="cc-select-search"
              placeholder={searchPlaceholder}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          )}
          <div className="cc-select-list">
            {filtered.map((opt) => {
              const isSelected = opt.value === value;
              return (
                <button
                  key={opt.value}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  className={`cc-select-opt${isSelected ? " is-selected" : ""}`}
                  onClick={() => {
                    onChange(opt.value);
                    close();
                  }}
                >
                  <div className="cc-select-opt-copy">
                    <span className="cc-select-opt-label">{opt.label}</span>
                    {opt.help && <span className="cc-select-opt-help">{opt.help}</span>}
                  </div>
                  {isSelected && (
                    <Check size={13} weight="bold" color="var(--c-accent)" aria-hidden="true" />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
