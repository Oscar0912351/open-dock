import { useCallback, useSyncExternalStore } from "react";
import { da } from "./da";
import { en } from "./en";

export type Messages = typeof da;
export type Locale = "da" | "en";
export type Interpolation = Record<string, string | number>;

const messages: Record<Locale, Messages> = { da, en };
let currentLocale: Locale = "da";
const listeners = new Set<() => void>();

export function setLocale(locale: Locale) {
  if (locale === currentLocale) return;
  currentLocale = locale;
  listeners.forEach((listener) => listener());
}

export function getLocale(): Locale {
  return currentLocale;
}

export function subscribeLocale(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useLocale(): Locale {
  return useSyncExternalStore(subscribeLocale, getLocale, getLocale);
}

export function useT(): (key: string, values?: Interpolation) => string {
  const locale = useLocale();
  return useCallback(
    (key: string, values?: Interpolation) => translate(locale, key, values),
    [locale],
  );
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
