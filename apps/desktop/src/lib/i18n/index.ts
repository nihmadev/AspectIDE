// i18n runtime: dictionary registry + message formatter.
//
// Supported message syntax:
//   - Simple placeholder:  "Rename {name}"            → params.name
//   - ICU-lite plural:     "{count, plural, one {# item} other {# items}}"
//        * branch selectors: zero|one|two|few|many|other and exact "=N"
//        * "#" inside a branch is replaced with the numeric value
//        * branches may contain nested placeholders, e.g. "other {{count} items}"
//        * plural category is resolved per-locale via Intl.PluralRules (RU gets one/few/many/other)
//
// `translate` never throws: a missing key falls back to the English string, then to the key itself.

import { DEFAULT_LOCALE, type Locale } from "./config";
import { messagesEn, type Messages, type MessageKey } from "./messages-en";

export type { Locale } from "./config";
export { LOCALES, DEFAULT_LOCALE, UI_LOCALE_KEY, isLocale, normalizeLocale } from "./config";
export type { MessageKey } from "./messages-en";

export type MessageParams = Record<string, string | number>;

// Only the default (English) dictionary is bundled eagerly. Non-default locale
// dictionaries (e.g. the ~139 KB Russian one) are code-split and fetched on
// demand via `loadDictionary`, so an English session never ships or parses them.
// Until a locale loads, `translate` falls back per-key to English — the same
// behavior as a missing key — so callers stay synchronous.
const DICTIONARIES: Partial<Record<Locale, Messages>> = {
  en: messagesEn,
};

// Loaders for the lazily-bundled dictionaries. `en` is always resident, so it has
// no loader. Each returns the chunk's `messages*` export.
const DICTIONARY_LOADERS: Partial<Record<Locale, () => Promise<Messages>>> = {
  ru: () => import("./messages-ru").then((module) => module.messagesRu),
};

const dictionaryLoads = new Map<Locale, Promise<void>>();

/**
 * Ensure `locale`'s dictionary is resident, fetching its split chunk once if
 * needed. Resolves immediately for already-loaded locales (incl. `en`). Callers
 * should `await`/chain this before switching the active locale so the first
 * render after the switch already has the strings (no English flash). A failed
 * load is swallowed — `translate` keeps falling back to English.
 */
export function loadDictionary(locale: Locale): Promise<void> {
  if (DICTIONARIES[locale]) return Promise.resolve();
  const existing = dictionaryLoads.get(locale);
  if (existing) return existing;
  const loader = DICTIONARY_LOADERS[locale];
  if (!loader) return Promise.resolve();
  const load = loader()
    .then((messages) => {
      DICTIONARIES[locale] = messages;
    })
    .catch(() => {
      // Leave it unloaded; per-key English fallback covers the gap. Drop the
      // cached rejection so a later attempt can retry.
      dictionaryLoads.delete(locale);
    });
  dictionaryLoads.set(locale, load);
  return load;
}

// Intl.PluralRules construction is ~18µs each; plural strings build one per
// placeholder per render. Memoize per locale (≤2 today) — pure, behavior-stable.
const pluralRulesByLocale = new Map<Locale, Intl.PluralRules>();
function pluralRulesFor(locale: Locale): Intl.PluralRules {
  let rules = pluralRulesByLocale.get(locale);
  if (!rules) {
    rules = new Intl.PluralRules(locale);
    pluralRulesByLocale.set(locale, rules);
  }
  return rules;
}

export function translate(locale: Locale, key: MessageKey, params?: MessageParams): string {
  const dictionary = DICTIONARIES[locale] ?? messagesEn;
  const template = dictionary[key] ?? messagesEn[key] ?? key;
  if (!params) return formatMessage(template, EMPTY_PARAMS, locale);
  return formatMessage(template, params, locale);
}

const EMPTY_PARAMS: MessageParams = {};

export function formatMessage(template: string, params: MessageParams, locale: Locale): string {
  return renderSegment(template, params, locale, undefined);
}

// `pluralValue` carries the active count so "#" resolves inside a plural branch.
function renderSegment(input: string, params: MessageParams, locale: Locale, pluralValue: number | undefined): string {
  let result = "";
  let index = 0;
  while (index < input.length) {
    const char = input[index];
    if (char === "#" && pluralValue !== undefined) {
      result += String(pluralValue);
      index += 1;
      continue;
    }
    if (char === "{") {
      const closingIndex = findMatchingBrace(input, index);
      const inner = input.slice(index + 1, closingIndex);
      result += renderPlaceholder(inner, params, locale);
      index = closingIndex + 1;
      continue;
    }
    result += char;
    index += 1;
  }
  return result;
}

function renderPlaceholder(inner: string, params: MessageParams, locale: Locale): string {
  const commaIndex = inner.indexOf(",");
  if (commaIndex === -1) {
    const name = inner.trim();
    const value = params[name];
    return value === undefined ? `{${name}}` : String(value);
  }

  const name = inner.slice(0, commaIndex).trim();
  const rest = inner.slice(commaIndex + 1).trim();
  if (rest.startsWith("plural")) {
    const body = rest.slice("plural".length).replace(/^\s*,/, "").trim();
    const branches = parseBranches(body);
    const count = toNumber(params[name]);
    const chosen = branches[`=${count}`]
      ?? branches[pluralRulesFor(locale).select(count)]
      ?? branches.other
      ?? "";
    return renderSegment(chosen, params, locale, count);
  }

  const value = params[name];
  return value === undefined ? `{${name}}` : String(value);
}

function parseBranches(body: string): Record<string, string> {
  const branches: Record<string, string> = {};
  let index = 0;
  while (index < body.length) {
    while (index < body.length && /\s/.test(body[index])) index += 1;
    if (index >= body.length) break;

    let selector = "";
    while (index < body.length && body[index] !== "{") {
      selector += body[index];
      index += 1;
    }
    if (body[index] !== "{") break;

    const closingIndex = findMatchingBrace(body, index);
    branches[selector.trim()] = body.slice(index + 1, closingIndex);
    index = closingIndex + 1;
  }
  return branches;
}

function findMatchingBrace(input: string, openIndex: number): number {
  let depth = 0;
  for (let index = openIndex; index < input.length; index += 1) {
    if (input[index] === "{") depth += 1;
    else if (input[index] === "}") {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return input.length;
}

function toNumber(value: string | number | undefined): number {
  const numberValue = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numberValue) ? numberValue : 0;
}
