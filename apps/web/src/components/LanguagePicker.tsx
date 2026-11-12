import { useState, useSyncExternalStore } from 'react';
import { chooseLanguage, getLanguagePreference, subscribeLanguage, t, type LanguagePreference } from '../i18n';

export function LanguagePicker({ compact = false }: { compact?: boolean }) {
  const preference = useSyncExternalStore(subscribeLanguage, getLanguagePreference);
  const [unsaved, setUnsaved] = useState(false);
  return (
    <div className="grid gap-1">
      <label className="flex items-center gap-2 text-xs text-muted">
        <span className={compact ? 'sr-only' : undefined}>{t('language-label')}</span>
        <select value={preference} className={compact ? 'max-w-40 rounded-xs py-1 ps-1.5 text-xs' : 'max-w-48 py-1 text-xs'} onChange={event => setUnsaved(!chooseLanguage(event.target.value as LanguagePreference))}>
          <option value="system">{t('language-system')}</option>
          <option value="en" lang="en">{t('language-en')}</option>
          <option value="tr" lang="tr">{t('language-tr')}</option>
        </select>
      </label>
      {unsaved && <p role="status" className="max-w-64 text-xs text-muted">{t('language-not-saved')}</p>}
    </div>
  );
}
