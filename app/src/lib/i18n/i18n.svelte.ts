// Minimal, dependency-free i18n. English is the source of truth: every key
// must exist in en.json; other locales fall back to English per key.

import en from "./locales/en.json";
import fr from "./locales/fr.json";
import de from "./locales/de.json";
import es from "./locales/es.json";

export type Locale = "en" | "fr" | "de" | "es";
export type MessageKey = keyof typeof en;
/** Catalog text: English is mandatory and the fallback of every locale. */
export type LocalizedText = { en: string } & Partial<Record<Exclude<Locale, "en">, string>>;

/** Each language under its own name, in its own language. */
export const LOCALES: { id: Locale; label: string }[] = [
  { id: "en", label: "English" },
  { id: "fr", label: "Français" },
  { id: "de", label: "Deutsch" },
  { id: "es", label: "Español" },
];

const dictionaries: Record<Locale, Partial<Record<MessageKey, string>>> = { en, fr, de, es };

function isLocale(value: string | null): value is Locale {
  return LOCALES.some((l) => l.id === value);
}
const STORAGE_KEY = "rk.locale";

function initialLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (isLocale(saved)) return saved;
  } catch {
    // Storage unavailable: fall through to the system language.
  }
  const system = navigator.language.toLowerCase().split("-")[0] ?? "en";
  return isLocale(system) ? system : "en";
}

let current = $state<Locale>(initialLocale());

export const i18n = {
  get locale(): Locale {
    return current;
  },
  set locale(value: Locale) {
    current = value;
    document.documentElement.lang = value;
    try {
      localStorage.setItem(STORAGE_KEY, value);
    } catch {
      // Not persisted; the choice still applies to this session.
    }
  },
};

/** Translates a UI message. `{name}` placeholders are replaced from `params`. */
export function t(key: MessageKey, params: Record<string, string | number> = {}): string {
  const template = dictionaries[current][key] ?? en[key];
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match,
  );
}

/** Picks the current locale from catalog text, falling back to English. */
export function localized(text: LocalizedText): string {
  return (current === "en" ? undefined : text[current]) ?? text.en;
}

/** Whether a message exists (error codes come from the backend). */
export function hasMessage(key: string): key is MessageKey {
  return key in en;
}

/** Bytes as gigabytes with one decimal, in the current locale's format. */
export function gigabytes(bytes: number): string {
  return new Intl.NumberFormat(current, { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(bytes / 1e9);
}
