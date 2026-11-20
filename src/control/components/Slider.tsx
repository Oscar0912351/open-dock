import type { CSSProperties } from "react";

interface SliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  formatValue?: (value: number) => string;
  onChange: (value: number) => void;
  ariaLabel?: string;
}

export function Slider({
  value,
  min,
  max,
  step = 1,
  formatValue,
  onChange,
  ariaLabel,
}: SliderProps) {
  const clamped = Math.min(max, Math.max(min, value));
  const pct = max > min ? ((clamped - min) / (max - min)) * 100 : 0;
  const chip = formatValue ? formatValue(clamped) : String(clamped);

  return (
    <div className="cc-slider-wrap">
      <input
        type="range"
        className="cc-slider"
        min={min}
        max={max}
        step={step}
        value={clamped}
        aria-label={ariaLabel}
        style={{ "--pct": `${pct}%` } as CSSProperties}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      <span className="cc-slider-chip">{chip}</span>
    </div>
  );
}
