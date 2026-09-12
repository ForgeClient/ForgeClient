// Message lookup. One entry point for the whole app: `t()` outside React,
// `useTranslation()` inside it.
//
// Three properties matter more than features here:
//
// 1. **A missing translation renders English, never a key.** Users of a partly
//    translated language see a mixed but readable UI, which is strictly better
//    than `settings.general.language.label` on screen.
// 2. **A missing *key* is a compile error.** `MessageKey` is derived from the
//    English catalogue, so `t()` cannot be called with something that does not
//    exist.
// 3. **Formatting follows the language.** Dates and numbers go through the same
//    locale, so a German UI does not print `Aug 10, 2026`.

import { CATALOGUES as CATALOGUE_REGISTRY } from "./catalog";
import { en, type Message, type MessageKey, type PluralMessage } from "./catalog/en";
import { intlTag, type Locale } from "./locales";
import { getLocale } from "./store";

const CATALOGUES: Record<Locale, Partial<Record<MessageKey, Message>>> = CATALOGUE_REGISTRY;

/** Values substituted into `{placeholder}` slots. */
export type MessageValues = Record<string, string | number>;

function isPlural(message: Message): message is PluralMessage {
  return typeof message !== "string";
}

/**
 * English wins over nothing, but never over a present translation. Looking the
 * key up in the active catalogue first and falling through to English is what
 * makes a partial catalogue safe to ship.
 */
function resolve(key: MessageKey, locale: Locale): Message {
  return CATALOGUES[locale][key] ?? en[key];
}

function selectPlural(message: PluralMessage, locale: Locale, values?: MessageValues): string {
  const count = typeof values?.count === "number" ? values.count : 0;
  const category = new Intl.PluralRules(intlTag(locale)).select(count);
  // Ask `Intl` which CLDR category applies, then use the authored form for it.
  // Languages differ in which categories they need at all: English and French
  // only ever produce `one` and `other`, while Russian also produces `few` and
  // `many`. Falling back to `other` for an unauthored category keeps a
  // half-written catalogue readable instead of empty.
  return message[category] ?? message.other;
}

/**
 * Placeholders are substituted verbatim, numbers included.
 *
 * Deliberately *not* locale-formatted: most numbers reaching a message are
 * identifiers (match ids, replay uids, ports), and grouping those turns
 * `27456965` into `27,456,965`, which is both wrong and unsearchable. A caller
 * that genuinely wants a grouped quantity formats it with `formatNumber` and
 * passes the resulting string.
 */
function interpolate(template: string, values?: MessageValues): string {
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = values[name];
    return value === undefined ? match : String(value);
  });
}

/** Translate `key` into the currently selected language. */
export function t(key: MessageKey, values?: MessageValues): string {
  return translateIn(getLocale(), key, values);
}

/** Translate into an explicit language. Used by tests and by the language picker. */
export function translateIn(locale: Locale, key: MessageKey, values?: MessageValues): string {
  const message = resolve(key, locale);
  const template = isPlural(message) ? selectPlural(message, locale, values) : message;
  return interpolate(template, values);
}

export function formatNumber(value: number, locale: Locale = getLocale()): string {
  return new Intl.NumberFormat(intlTag(locale)).format(value);
}

/**
 * A number with exactly one decimal place, in the reader's own notation.
 *
 * Review scores are the reason this exists: "4.7" is a typo in German, where
 * the decimal separator is a comma and a full stop groups thousands.
 */
export function formatDecimal(value: number, locale: Locale = getLocale()): string {
  return new Intl.NumberFormat(intlTag(locale), {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(value);
}

export { DEFAULT_LOCALE, intlTag, isLocale, LOCALE_KEYS, LOCALES } from "./locales";
export type { Locale, LocaleDefinition } from "./locales";
export type { MessageKey } from "./catalog/en";
export { getLocale, setLocale, subscribeToLocale } from "./store";
