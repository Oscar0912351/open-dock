import { da } from "./da";
import { en } from "./en";

export type Messages = typeof da;
export type Locale = "da" | "en";
export type Interpolation = Record<string, string | number>;

const messages: Record<Locale, Messages> = { da, en };
let currentLocale: Locale = "da";

export function setLocale(locale: Locale) {
  currentLocale = locale;
}

export function getLocale(): Locale {
  return currentLocale;
}

export function translate(locale: Locale, key: string, values: Interpolation = {}): string {
  const source = key.split(".").reduce<unknown>((current, part) => {
    if (typeof current !== "object" || current === null || !(part in current)) return undefined;
    return (current as Record<string, unknown>)[part];
  }, messages[locale]);

  if (typeof source !== "string") return key;
  return source.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = values[name];
    return value === undefined ? match : String(value);
  });
}

export function t(key: string, values: Interpolation = {}): string {
  return translate(currentLocale, key, values);
}
