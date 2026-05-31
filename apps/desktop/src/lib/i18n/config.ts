// Locale configuration for the Lux IDE interface.
//
// Adding a new language:
//   1. Create `messages-<id>.ts` exporting a dictionary that satisfies `Messages`
//      (TypeScript enforces that every key from the English source of truth exists).
//   2. Register it in the `DICTIONARIES` map in `./index.ts`.
//   3. Add an entry to `LOCALES` below.
// No other code changes are required — every `t(...)` call picks it up automatically.

export type Locale = "en" | "ru";

export type LocaleDescriptor = {
  id: Locale;
  // Name shown in the language picker, written in the language itself.
  nativeLabel: string;
  // Name in English, used for search/keywords and accessibility.
  englishLabel: string;
};

export const LOCALES: readonly LocaleDescriptor[] = [
  { id: "en", nativeLabel: "English", englishLabel: "English" },
  { id: "ru", nativeLabel: "Русский", englishLabel: "Russian" },
];

export const DEFAULT_LOCALE: Locale = "en";

export const UI_LOCALE_KEY = "ui.locale";

export function isLocale(value: unknown): value is Locale {
  return typeof value === "string" && LOCALES.some((locale) => locale.id === value);
}

export function normalizeLocale(value: unknown): Locale {
  return isLocale(value) ? value : DEFAULT_LOCALE;
}
