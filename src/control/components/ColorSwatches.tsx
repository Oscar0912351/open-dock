import { Eyedropper } from "@phosphor-icons/react";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import { t } from "../../shared/i18n";

export interface AccentPreset {
  hex: string;
  labelKey: string;
}

export const THEME_DEFAULT_ACCENT: Record<ThemeId, string> = {
  aurora: "#FFFFFF",
  laerk: "#F6B733",
  grafit: "#F4F5F6",
};

const CHROMATIC_PRESETS: readonly AccentPreset[] = [
  { hex: "#5CA9FF", labelKey: "themes.accentBlue" },
  { hex: "#34D399", labelKey: "themes.accentGreen" },
  { hex: "#F6B733", labelKey: "themes.accentYellow" },
  { hex: "#FB7A54", labelKey: "themes.accentOrange" },
  { hex: "#F25C69", labelKey: "themes.accentRed" },
];

const WHITE_PRESET: AccentPreset = { hex: "#FFFFFF", labelKey: "themes.accentWhite" };
const DARK_PRESET: AccentPreset = { hex: "#2E2D2A", labelKey: "themes.accentDark" };

const ALL_PRESETS: readonly AccentPreset[] = [WHITE_PRESET, DARK_PRESET, ...CHROMATIC_PRESETS];

function presetsForTheme(themeId: ThemeId): readonly AccentPreset[] {
  return [themeId === "laerk" ? DARK_PRESET : WHITE_PRESET, ...CHROMATIC_PRESETS];
}

function isLightHex(hex: string): boolean {
  const clean = hex.replace("#", "");
  if (clean.length !== 6) return true;
  const r = parseInt(clean.slice(0, 2), 16);
  const g = parseInt(clean.slice(2, 4), 16);
  const b = parseInt(clean.slice(4, 6), 16);
  if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) return true;
  return (r * 299 + g * 587 + b * 114) / 1000 > 150;
}

export function accentLabel(value: string | null): string {
  if (value === null) return t("themes.accentAuto");
  const match = ALL_PRESETS.find((p) => p.hex.toUpperCase() === value.toUpperCase());
  return match ? t(match.labelKey) : value.toUpperCase();
}

export function ColorSwatches({
  value,
  themeId,
  onChange,
}: {
  value: string | null;
  themeId: ThemeId;
  onChange: (hex: string | null) => void;
}) {
  const presets = presetsForTheme(themeId);
  const upper = value?.toUpperCase() ?? null;
  const isCustom = upper !== null && !presets.some((p) => p.hex.toUpperCase() === upper);
  const pickerHex = upper && /^#[0-9A-F]{6}$/.test(upper) ? upper : THEME_DEFAULT_ACCENT[themeId];

  return (
    <div className="cc-accent-tray" role="radiogroup" aria-label={t("themes.accent")}>
      <button
        type="button"
        role="radio"
        aria-checked={value === null}
        className={`cc-accent-auto${value === null ? " is-selected" : ""}`}
        onClick={() => onChange(null)}
      >
        <span
          className="cc-accent-auto-dot"
          aria-hidden="true"
          style={{ backgroundColor: THEME_DEFAULT_ACCENT[themeId] }}
        />
        <span>{t("themes.accentTheme")}</span>
      </button>

      <span className="cc-accent-sep" aria-hidden="true" />

      <div className="cc-accent-swatches">
        {presets.map((preset) => {
          const selected = upper === preset.hex.toUpperCase();
          return (
            <button
              key={preset.hex}
              type="button"
              role="radio"
              aria-checked={selected}
              aria-label={t(preset.labelKey)}
              title={t(preset.labelKey)}
              className={`cc-accent-chip${selected ? " is-selected" : ""}`}
              onClick={() => onChange(preset.hex)}
            >
              <span
                className="cc-accent-chip-fill"
                style={{ backgroundColor: preset.hex }}
              />
            </button>
          );
        })}
      </div>

      <span className="cc-accent-sep" aria-hidden="true" />

      <label
        className={`cc-accent-custom${isCustom ? " is-selected" : ""}`}
        title={t("themes.accentCustom")}
      >
        <input
          type="color"
          className="cc-accent-color-input"
          value={pickerHex}
          aria-label={t("themes.accentCustom")}
          onChange={(e) => onChange(e.target.value.toUpperCase())}
        />
        {isCustom && value ? (
          <span
            className="cc-accent-chip-fill cc-accent-custom-fill"
            style={{ backgroundColor: value }}
          >
            <Eyedropper
              size={12}
              weight="bold"
              color={isLightHex(value) ? "#09090B" : "#FFFFFF"}
              aria-hidden="true"
            />
          </span>
        ) : (
          <Eyedropper size={14} aria-hidden="true" />
        )}
      </label>
    </div>
  );
}
