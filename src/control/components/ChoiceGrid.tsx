import type { ReactNode } from "react";

export interface ChoiceOption<T extends string> {
  value: T;
  label: string;
  preview: ReactNode;
  badge?: string;
  disabled?: boolean;
  title?: string;
}

interface ChoiceGridProps<T extends string> {
  options: ReadonlyArray<ChoiceOption<T>>;
  value: T;
  onChange: (value: T) => void;
  ariaLabel?: string;
}

export function ChoiceGrid<T extends string>({
  options,
  value,
  onChange,
  ariaLabel,
}: ChoiceGridProps<T>) {
  return (
    <div className="cc-choice" role="radiogroup" aria-label={ariaLabel}>
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={opt.disabled}
            title={opt.title}
            className={`cc-choice-item${active ? " is-selected" : ""}`}
            onClick={() => {
              if (!opt.disabled && !active) onChange(opt.value);
            }}
          >
            <span className="cc-choice-prev" aria-hidden="true">
              {opt.preview}
            </span>
            <span className="cc-choice-foot">
              <span className="cc-choice-label">{opt.label}</span>
              {opt.badge && <span className="cc-choice-badge">{opt.badge}</span>}
            </span>
          </button>
        );
      })}
    </div>
  );
}
