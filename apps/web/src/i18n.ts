// Fluent messages from the shared locales/ directory (ADR-0007). All user-facing text comes from here.
// The user's language first, English behind it: a message missing from a translation shows in English, not as a key.
import { FluentBundle, FluentResource, type FluentVariable } from '@fluent/bundle';
import webEn from '../../../locales/en/web.ftl?raw';
import webTr from '../../../locales/tr/web.ftl?raw';

const SOURCES: Record<string, string> = { en: webEn, tr: webTr };

function bundle(lang: string): FluentBundle {
  const b = new FluentBundle(lang, { useIsolating: false });
  const errors = b.addResource(new FluentResource(SOURCES[lang] ?? webEn));
  if (errors.length > 0) console.error('locale errors', lang, errors);
  return b;
}

// `tr-TR` -> `tr`; anything without a translation is English.
export function pickLanguage(tag: string | undefined): string {
  const primary = (tag ?? 'en').split(/[-_.@]/)[0]?.toLowerCase() ?? 'en';
  return primary in SOURCES ? primary : 'en';
}

// Languages written right to left. The page's direction follows the language so a right-to-left translation lays out correctly
// from its first release (docs/architecture.md: RTL is fully supported in the web client).
const RTL = new Set(['ar', 'he', 'fa', 'ur', 'ps', 'sd', 'yi', 'dv', 'ku']);

export function directionOf(lang: string): 'rtl' | 'ltr' {
  return RTL.has(lang.split(/[-_]/)[0]?.toLowerCase() ?? '') ? 'rtl' : 'ltr';
}

export type LanguagePreference = 'system' | 'en' | 'tr';
const PREFERENCE_KEY = 'ui.language';
const listeners = new Set<() => void>();
let chain: FluentBundle[] = [];
let language = 'en';

function readPreference(): LanguagePreference {
  try {
    const saved = localStorage.getItem(PREFERENCE_KEY);
    if (saved === 'en' || saved === 'tr') return saved;
  } catch { /* Storage can be unavailable; the browser language remains a useful default. */ }
  return 'system';
}

let preference = readPreference();
const browserLanguage = () => typeof navigator !== 'undefined' ? navigator.language : 'en';

export const getLanguage = () => language;
export const getLanguagePreference = () => preference;
export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function setLanguage(tag: string | undefined): void {
  const lang = pickLanguage(tag);
  chain = lang === 'en' ? [bundle('en')] : [bundle(lang), bundle('en')];
  language = lang;
  if (typeof document !== 'undefined') {
    document.documentElement.lang = lang;
    document.documentElement.dir = directionOf(lang);
  }
  for (const listener of listeners) listener();
}

// Only an explicit choice writes a browser preference. Changing it never replaces application/session state.
export function chooseLanguage(value: LanguagePreference): boolean {
  preference = value;
  let saved = true;
  try {
    if (value === 'system') localStorage.removeItem(PREFERENCE_KEY);
    else localStorage.setItem(PREFERENCE_KEY, value);
  } catch { saved = false; }
  setLanguage(value === 'system' ? browserLanguage() : value);
  return saved;
}

setLanguage(preference === 'system' ? browserLanguage() : preference);
if (typeof window !== 'undefined') {
  window.addEventListener('languagechange', () => {
    if (preference === 'system') setLanguage(browserLanguage());
  });
  window.addEventListener('storage', (event) => {
    if (event.key === PREFERENCE_KEY || event.key === null) {
      preference = readPreference();
      setLanguage(preference === 'system' ? browserLanguage() : preference);
    }
  });
}

export function t(id: string, args?: Record<string, FluentVariable>): string {
  for (const b of chain) {
    const msg = b.getMessage(id);
    if (msg?.value) return b.formatPattern(msg.value, args);
  }
  return id; // a missing key is visible in the UI and caught by the test
}
