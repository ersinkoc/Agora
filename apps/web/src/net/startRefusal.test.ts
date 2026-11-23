import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { noticeKey } from './chat';
import { ROLE_REFUSAL, refusalCode, startRefusalKey } from './startRefusal';

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
      const gaveUp = t(noticeKey('adapter.gave_up'), { restarts: '5', exit_code: '3' });
      expect(restarting, lang).toContain('2');
      expect(gaveUp, lang).toContain('3');
      expect(restarting).not.toBe(noticeKey('adapter.restarting'));
      expect(gaveUp).not.toBe(noticeKey('adapter.gave_up'));
    }
  });
});

describe('a start the hub refused for the person\'s role, not for a device (#1044)', () => {
  beforeEach(() => setLanguage('en'));
  it('tells the hub\'s refusal for a role (the refused command\'s schema name) from a machine\'s refusal for a limited device', () => {
    const refused = (action: string) => ({ code: 'auth.forbidden', params: { action } });
    // The hub (ADR-0049): a viewer, or a member outside their projects, names the command that was refused.
    expect(refusalCode(refused('AgentLaunch'), 'AgentLaunch')).toBe(ROLE_REFUSAL);
    // A machine that limits this device to other projects names its own action: the device wording stays.
    expect(refusalCode(refused('directory'), 'AgentLaunch')).toBe('auth.forbidden');
    expect(refusalCode(refused('grant.create'), 'AgentLaunch')).toBe('auth.forbidden');
    // Another command's name, no action, or another code: kept as it came.
    expect(refusalCode(refused('AgentStop'), 'AgentLaunch')).toBe('auth.forbidden');
    expect(refusalCode({ code: 'auth.forbidden' }, 'AgentLaunch')).toBe('auth.forbidden');
    expect(refusalCode({ code: 'node.offline', params: { action: 'AgentLaunch' } }, 'AgentLaunch')).toBe('node.offline');
    expect(refusalCode(undefined, 'AgentLaunch')).toBeUndefined();
    expect(refusalCode('boom', 'AgentLaunch')).toBeUndefined();
  });
  it('says the person\'s role in English and Turkish, and the device sentence only for a device', () => {
    expect(startRefusalKey(ROLE_REFUSAL)).toBe('start-role');
    expect(startRefusalKey('auth.forbidden')).toBe('start-directory-forbidden');
    for (const lang of ['en', 'tr']) {
      setLanguage(lang);
      const role = t('start-role');
      expect(role, lang).not.toBe('start-role');
      expect(role, lang).not.toBe(t('start-directory-forbidden'));
    }
    setLanguage('en');
    expect(t('start-role')).toContain('Your role');
    expect(t('start-role')).not.toContain('device');
  });
});
