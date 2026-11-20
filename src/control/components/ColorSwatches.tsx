import { Check } from "@phosphor-icons/react";
import type { ThemeId } from "../../shared/bindings/ThemeId";
import { t } from "../../shared/i18n";

export interface AccentSwatch {
  hex: string | null;
  labelKey: string;
  previewHex: string;
}

export const THEME_DEFAULT_ACCENT: Record<ThemeId, string> = {
  aurora: "#FFFFFF",
  laerk: "#F6B733",
  grafit: "#F4F5F6",
};

const SWATCHES: readonly AccentSwatch[] = [
  { hex: null, labelKey: "themes.accentTheme", previewHex: "#FFFFFF" },
  { hex: "#FFFFFF", labelKey: "themes.accentWhite", previewHex: "#FFFFFF" },
  { hex: "#F6B733", labelKey: "themes.accentRav", previewHex: "#F6B733" },
  { hex: "#FF8A70", labelKey: "themes.accentKoral", previewHex: "#FF8A70" },
  { hex: "#A1A1AA", labelKey: "themes.accentSolv", previewHex: "#A1A1AA" },
  { hex: "#8CCBFF", labelKey: "themes.accentIs", previewHex: "#8CCBFF" },
  { hex: "#B5C99A", labelKey: "themes.accentSalvie", previewHex: "#B5C99A" },
  { hex: "#F4A6B8", labelKey: "themes.accentRose", previewHex: "#F4A6B8" },
];

const DARK_CHECK_BG = new Set([
  "#FFFFFF",
  "#F4F5F6",
  "#A1A1AA",
  "#F6B733",
  "#8CCBFF",
  "#B5C99A",
  "#F4A6B8",
]);

export function ColorSwatches({
  value,
  themeId,
  onChange,
}: {
  value: string | null;
  themeId: ThemeId;
  onChange: (hex: string | null) => void;
}) {
  const list = SWATCHES.filter((s) => !(themeId === "laerk" && s.hex === "#FFFFFF"));

  return (
    <div className="cc-swatches" role="radiogroup" aria-label={t("themes.accent")}>
      {list.map((swatch) => {
        const selected =
          swatch.hex === null
            ? value === null
            : value?.toUpperCase() === swatch.hex.toUpperCase();
        const bg = swatch.hex === null ? THEME_DEFAULT_ACCENT[themeId] : swatch.previewHex;
        const darkCheck = DARK_CHECK_BG.has(bg.toUpperCase());

        return (
          <button
            key={swatch.labelKey}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={t(swatch.labelKey)}
            title={t(swatch.labelKey)}
            className={`cc-swatch${selected ? " is-selected" : ""}`}
            style={{ backgroundColor: bg }}
            onClick={() => onChange(swatch.hex)}
          >
            {selected && (
              <Check size={13} weight="bold" color={darkCheck ? "#09090B" : "#FFFFFF"} aria-hidden="true" />
            )}
          </button>
        );
      })}
    </div>
  );
}
