import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { noticeKey } from './chat';
import { startRefusalKey } from './startRefusal';

describe('a refused start and an adapter that ended (#902, #903)', () => {
  beforeEach(() => setLanguage('en'));
  it('says a missing program in words, and a code it does not know as the general sentence', () => {
    expect(startRefusalKey('agent.program_missing')).toBe('start-program-missing');
    expect(t('start-program-missing')).toContain('cannot find this agent\'s program in its PATH');
    expect(startRefusalKey('agent.kind_unknown')).toBe('start-kind-unknown');
    expect(startRefusalKey('internal')).toBe('start-failed');
    expect(startRefusalKey('toString')).toBe('start-failed');
    expect(startRefusalKey(undefined)).toBe('start-failed');
  });
  it('says the node\'s restart and give-up notices with their numbers, in English and Turkish', () => {
    for (const lang of ['en', 'tr']) {
      setLanguage(lang);
      const restarting = t(noticeKey('adapter.restarting'), { attempt: '2' });
      const gaveUp = t(noticeKey('adapter.gave_up'), { exit_code: '3' });
      expect(restarting, lang).toContain('2');
      expect(gaveUp, lang).toContain('3');
      expect(restarting).not.toBe(noticeKey('adapter.restarting'));
      expect(gaveUp).not.toBe(noticeKey('adapter.gave_up'));
    }
  });
});
