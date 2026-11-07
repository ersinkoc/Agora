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

let chain: FluentBundle[] = [];

export function setLanguage(tag: string | undefined): void {
  const lang = pickLanguage(tag);
  chain = lang === 'en' ? [bundle('en')] : [bundle(lang), bundle('en')];
  if (typeof document !== 'undefined') document.documentElement.lang = lang;
}

setLanguage(typeof navigator !== 'undefined' ? navigator.language : 'en');

export function t(id: string, args?: Record<string, FluentVariable>): string {
  for (const b of chain) {
    const msg = b.getMessage(id);
    if (msg?.value) return b.formatPattern(msg.value, args);
  }
  return id; // a missing key is visible in the UI and caught by the test
}
