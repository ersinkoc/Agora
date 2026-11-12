import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { forgetReasonKey } from './forget';

describe('forgetReasonKey', () => {
  beforeEach(() => setLanguage('en'));
  it('has words for every refusal the window shows, in both languages', () => {
    for (const lang of ['en', 'tr'] as const) {
      setLanguage(lang);
      for (const code of ['auth.forbidden', 'agent.running', 'agent.not_found', 'hub.unreachable', 'something.else']) {
        const key = forgetReasonKey(code);
        expect(t(key), `${lang} ${code}`).not.toBe(key);
      }
    }
  });
});
