import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from "react";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  disabled?: boolean;
  title?: string;
}

interface SegmentedProps<T extends string> {
  options: ReadonlyArray<SegmentedOption<T>>;
  value: T;
  onChange: (value: T) => void;
  fullWidth?: boolean;
  ariaLabel?: string;
}

export function Segmented<T extends string>({
  options,
  value,
  onChange,
  fullWidth = false,
  ariaLabel,
}: SegmentedProps<T>) {
  const rootRef = useRef<HTMLDivElement>(null);
  const btnRefs = useRef<Map<T, HTMLButtonElement>>(new Map());
  const [thumb, setThumb] = useState<{ x: number; w: number; visible: boolean }>({
    x: 0,
    w: 0,
    visible: false,
  });

  const measure = useCallback(() => {
    const root = rootRef.current;
    const btn = btnRefs.current.get(value);
    if (!root || !btn) {
      setThumb((prev) => (prev.visible ? { x: 0, w: 0, visible: false } : prev));
      return;
    }
    const x = btn.offsetLeft - root.clientLeft;
    const w = btn.offsetWidth;
    setThumb({ x, w, visible: w > 0 });
  }, [value]);

  useLayoutEffect(() => {
    measure();
    const root = rootRef.current;
    if (!root || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => measure());
    ro.observe(root);
    return () => ro.disconnect();
  }, [measure, options]);

  return (
    <div
      ref={rootRef}
      className={`cc-seg${fullWidth ? " is-full" : ""}`}
      role="tablist"
      aria-label={ariaLabel}
    >
      <div
        className="cc-seg-thumb"
        aria-hidden="true"
        style={{
          width: `${thumb.w}px`,
          transform: `translateX(${thumb.x}px)`,
          opacity: thumb.visible ? 1 : 0,
        }}
      />
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            ref={(el) => {
              if (el) btnRefs.current.set(opt.value, el);
              else btnRefs.current.delete(opt.value);
            }}
            type="button"
            role="tab"
            aria-selected={active}
            disabled={opt.disabled}
            title={opt.title}
            className={`cc-seg-btn${active ? " is-active" : ""}`}
            onClick={() => {
              if (!opt.disabled && !active) onChange(opt.value);
            }}
          >
            {opt.icon}
            <span>{opt.label}</span>
          </button>
        );
      })}
    </div>
  );
}
