// @vitest-environment jsdom
// #1048 item 4 (redesign PR 13): the phone's request cards and its bar for what needs you keep T44 exactly as Home's queue and the side
// panel do (cardDwell.test.tsx): Allow and Deny do nothing for a short dwell after what the surface shows changed, after the person came
// back and after the link came back; a request is answered once, held while its answer goes, kept answered once it went, and answerable
// again after a fresh dwell when the answer failed. One mechanism (`useAnswerGuard`) with the window's holds, so each case runs on every
// phone surface, and a request answered on one phone screen stays answered on the next. The cards are the one request component in its
// row density.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { act, type ReactElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { ATTENTION_DWELL_MS } from '../net/attention';
import type { ApprovalModel } from '../net/view';
import { AnswerHoldsProvider } from '../components/useAnswerGuard';
import { PhoneAgent, PhoneHome } from './Phone';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
// These tests are about request cards, not terminals (see cardDwell.test.tsx).
vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => null }));

const agent: AgentTileModel = { id: 'n1/a', name: 'claude · teal', accent: 'var(--agent-teal)', kind: 'claude', machine: 'laptop', machineId: 'n1', workspace: 'w', project: 'login-fix', state: 'needs-you', preview: '' };
const machines = [{ id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: 0 }];
// A request whose detail has two lines: never answered from the bar (it is not plain), so the bar only reviews and every card has buttons.
const ask = (id: string, file: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id, node: 'n1', localId: id, agentId: 'n1/a', agentName: 'claude · teal', machineName: 'laptop', action: 'Edit', detail: `${file}\n+1 -1`, expiresAtMs: 0, ...over });
// A plain one the bar answers.
const plain = (id: string, file: string, over: Partial<ApprovalModel> = {}): ApprovalModel => ask(id, file, { detail: file, ...over });
type Resolve = (a: ApprovalModel, ok: boolean) => unknown;
interface Opts { stale?: boolean }

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); vi.useFakeTimers(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });

const dwell = (ms = ATTENTION_DWELL_MS) => act(() => { vi.advanceTimersByTime(ms); });
const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

const surfaces: [string, (approvals: ApprovalModel[], onResolve: Resolve, o?: Opts) => ReactElement][] = [
  ['the phone list', (approvals, onResolve, o = {}) => <PhoneHome agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link={o.stale ? 'offline' : 'live'} onOpen={() => {}} />],
  ['an agent on the phone', (approvals, onResolve, o = {}) => <PhoneAgent agent={agent} agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link={o.stale ? 'offline' : 'live'} onOpen={() => {}} onBack={() => {}} />],
];

// A request's card in the screen (by the file it names), and its buttons.
const cardOf = (file: string) => [...host.querySelectorAll('.phone__main section.approval-req')].find((s) => s.textContent?.includes(file));
const btn = (file: string, name: string) => [...(cardOf(file)?.querySelectorAll('button') ?? [])].find((b) => b.textContent === name) as HTMLButtonElement;

describe.each(surfaces)('request cards on %s keep T44 (#1048 item 4)', (_name, draw) => {
  const render = (approvals: ApprovalModel[], onResolve: Resolve, o?: Opts) => act(() => root.render(draw(approvals, onResolve, o)));

  it('waits out the dwell before Allow or Deny does anything, and a press during it sends nothing', () => {
    const sent: [string, boolean][] = [];
    render([ask('r1', 'a.rs')], (x, ok) => { sent.push([x.id, ok]); });
    expect(btn('a.rs', 'Allow').getAttribute('aria-disabled')).toBe('true');
    act(() => btn('a.rs', 'Allow').click());
    act(() => btn('a.rs', 'Deny').click());
    expect(sent).toEqual([]);
    dwell();
    expect(btn('a.rs', 'Allow').hasAttribute('aria-disabled')).toBe(false);
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual([['r1', true]]);
  });

  it('starts the dwell again when a card comes in above, and when what a card shows changes', () => {
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); };
    render([ask('r1', 'a.rs', { expiresAtMs: 9e12 })], onResolve);
    dwell();
    render([ask('r2', 'b.rs', { expiresAtMs: 1e12 }), ask('r1', 'a.rs', { expiresAtMs: 9e12 })], onResolve);
    act(() => btn('a.rs', 'Allow').click());
    act(() => btn('b.rs', 'Allow').click());
    expect(sent).toEqual([]);
    dwell();
    render([ask('r2', 'b.rs', { expiresAtMs: 1e12 }), ask('r1', 'a.rs', { expiresAtMs: 9e12, machineName: 'server' })], onResolve);
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual([]);
    dwell(ATTENTION_DWELL_MS - 1);
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual([]);
    dwell(1);
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
  });

  it('is not ready the moment the person comes back: focus or the tab shown again and a press in the same batch send nothing', () => {
    const sent: string[] = [];
    render([ask('r1', 'a.rs')], (x) => { sent.push(x.id); });
    dwell();
    act(() => { window.dispatchEvent(new Event('focus')); btn('a.rs', 'Allow').click(); });
    expect(sent).toEqual([]);
    dwell();
    act(() => { document.dispatchEvent(new Event('visibilitychange')); btn('a.rs', 'Allow').click(); });
    expect(sent).toEqual([]);
    dwell();
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
  });

  it('answers a request once: two presses in one batch, or a second press on either button, send one answer', () => {
    const sent: [string, boolean][] = [];
    render([ask('r1', 'a.rs')], (x, ok) => { sent.push([x.id, ok]); });
    dwell();
    act(() => { btn('a.rs', 'Deny').click(); btn('a.rs', 'Allow').click(); });
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual([['r1', false]]);
    expect(btn('a.rs', 'Allow').getAttribute('aria-disabled')).toBe('true');
    expect(btn('a.rs', 'Deny').getAttribute('aria-disabled')).toBe('true');
  });

  it('keeps an answer on its way held when the list drops its request and brings it back', () => {
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); return new Promise(() => {}); };
    render([ask('r1', 'a.rs')], onResolve);
    dwell();
    act(() => btn('a.rs', 'Allow').click());
    render([], onResolve);
    render([ask('r1', 'a.rs')], onResolve);
    act(() => { window.dispatchEvent(new Event('focus')); });
    dwell(60_000);
    expect(btn('a.rs', 'Allow').getAttribute('aria-disabled')).toBe('true');
    act(() => btn('a.rs', 'Allow').click());
    act(() => btn('a.rs', 'Deny').click());
    expect(sent).toEqual(['r1']);
  });

  it('keeps an answer that went the only one while its card is there: "Answer sent", Allow and Deny off', async () => {
    for (const went of [() => true, () => Promise.resolve(true), () => undefined]) {
      const sent: string[] = [];
      render([ask('r1', 'a.rs')], (x) => { sent.push(x.id); return went(); });
      dwell();
      act(() => btn('a.rs', 'Allow').click());
      await flush();
      dwell(60_000);
      act(() => { window.dispatchEvent(new Event('focus')); });
      dwell();
      expect(btn('a.rs', 'Allow').getAttribute('aria-disabled')).toBe('true');
      act(() => btn('a.rs', 'Allow').click());
      act(() => btn('a.rs', 'Deny').click());
      expect(sent).toEqual(['r1']);
      expect(cardOf('a.rs')?.textContent).toContain(t('approval-sent'));
      act(() => root.render(<></>));
    }
  });

  it('re-arms with a fresh dwell when the answer fails (false, a rejected promise, a throw), never before the dwell', async () => {
    const fails: (() => unknown)[] = [() => Promise.resolve(false), () => Promise.reject(new Error('refused')), () => false, () => { throw new Error('no key'); }];
    for (const fail of fails) {
      const sent: string[] = [];
      render([ask('r1', 'a.rs')], (x) => { sent.push(x.id); return fail(); });
      dwell();
      act(() => btn('a.rs', 'Allow').click());
      await flush();
      act(() => btn('a.rs', 'Allow').click());
      expect(sent).toEqual(['r1']);
      dwell();
      expect(btn('a.rs', 'Allow').hasAttribute('aria-disabled')).toBe(false);
      act(() => btn('a.rs', 'Allow').click());
      expect(sent).toEqual(['r1', 'r1']);
      act(() => root.render(<></>));
    }
  });

  it('waits while the link is down, says why, and starts the dwell again when it is back', () => {
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); };
    render([ask('r1', 'a.rs')], onResolve, { stale: true });
    dwell(ATTENTION_DWELL_MS * 2);
    expect(btn('a.rs', 'Allow').disabled).toBe(true);
    expect(cardOf('a.rs')?.textContent).toContain(t('fleet-stale-no-answer'));
    render([ask('r1', 'a.rs')], onResolve, { stale: false });
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual([]);
    dwell();
    act(() => btn('a.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
  });

  it('a start request in its card waits too, and is answered once', async () => {
    const { demoStartApproval } = await import('../fixtures');
    const on = vi.fn();
    render([{ ...(demoStartApproval as ApprovalModel), agentId: agent.id }], on);
    const allow = () => [...host.querySelectorAll('.phone__main button')].find((b) => b.textContent === 'Allow') as HTMLButtonElement;
    expect(allow().disabled).toBe(true);
    act(() => allow().click());
    expect(on).not.toHaveBeenCalled();
    dwell();
    act(() => { allow().click(); allow().click(); });
    expect(on).toHaveBeenCalledTimes(1);
  });
});

describe('node-supplied phone labels', () => {
  it('makes machine group headings visible', () => {
    const raw = 'lap\u3164top\u202e';
    act(() => root.render(<PhoneHome agents={[{ ...agent, machine: raw }]} machines={[{ ...machines[0]!, name: raw }]} approvals={[]} onResolve={() => {}} link="live" onOpen={() => {}} />));
    const group = [...host.querySelectorAll<HTMLElement>('.phone-section[aria-label]')].at(-1)!;
    expect(group.getAttribute('aria-label')).toBe('lap\\u{3164}top\\u{202e}');
    expect(group.querySelector('h2')?.textContent).toContain('lap\\u{3164}top\\u{202e}');
  });

  it('verifies machine headings across format, control, and private-use characters', () => {
    for (const [char, escaped] of [['\u200b', '\\u{200b}'], ['\u0001', '\\u{1}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const raw = `lap${char}top`;
      act(() => root.render(<PhoneHome agents={[{ ...agent, machine: raw }]} machines={[{ ...machines[0]!, name: raw }]} approvals={[]} onResolve={() => {}} link="live" onOpen={() => {}} />));
      const group = [...host.querySelectorAll<HTMLElement>('.phone-section[aria-label]')].at(-1)!;
      expect(group.getAttribute('aria-label')).toBe(`lap${escaped}top`);
      expect(group.textContent).toContain(`lap${escaped}top`);
    }
  });

  it('makes agent identity and work labels visible in fleet rows', () => {
    const shown = { ...agent, name: 'clau\u034fde\ue000', project: 'lo\u2066gin', branch: 'fix\u0007now' };
    act(() => root.render(<PhoneHome agents={[shown]} machines={machines} approvals={[]} onResolve={() => {}} link="live" onOpen={() => {}} />));
    const row = host.querySelector('.phone-row') as HTMLElement;
    expect(row.textContent).toContain('clau\\u{34f}de\\u{e000}');
    expect(row.textContent).toContain('lo\\u{2066}gin · fix\\u{7}now');
  });

  it('verifies fleet row labels without changing the raw agent identity', () => {
    const opened = vi.fn();
    for (const [char, escaped] of [['\u061c', '\\u{61c}'], ['\u0085', '\\u{85}'], ['\ue000', '\\u{e000}']] as const) {
      const shown = { ...agent, name: `co${char}dex`, workspace: `wo${char}rk`, project: '', branch: `ma${char}in` };
      act(() => root.render(<PhoneHome agents={[shown]} machines={machines} approvals={[]} onResolve={() => {}} link="live" onOpen={opened} />));
      const row = host.querySelector<HTMLButtonElement>('.phone-row')!;
      expect(row.textContent).toContain(`co${escaped}dex`);
      expect(row.textContent).toContain(`wo${escaped}rk · ma${escaped}in`);
      act(() => row.click());
      expect(opened).toHaveBeenLastCalledWith(agent.id);
    }
  });

  it('makes the focused agent and machine labels visible', () => {
    const shown = { ...agent, name: 'clau\u200bde\u202e', machine: 'lap\u3164top\ue000' };
    act(() => root.render(<PhoneAgent agent={shown} agents={[shown]} machines={machines} approvals={[]} onResolve={() => {}} onOpen={() => {}} onBack={() => {}} />));
    expect(host.querySelector('.phone__main')?.getAttribute('aria-label')).toBe('clau\\u{200b}de\\u{202e}');
    expect(host.querySelector('h1')?.textContent).toBe('clau\\u{200b}de\\u{202e}');
    expect(host.querySelector('h1')?.parentElement?.textContent).toContain('lap\\u{3164}top\\u{e000}');
  });

  it('verifies focused labels across isolate, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u2066', '\\u{2066}'], ['\u0007', '\\u{7}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const shown = { ...agent, name: `cl${char}aude`, machine: `ma${char}chine` };
      act(() => root.render(<PhoneAgent agent={shown} agents={[shown]} machines={machines} approvals={[]} onResolve={() => {}} onOpen={() => {}} onBack={() => {}} />));
      expect(host.querySelector('.phone__main')?.getAttribute('aria-label')).toBe(`cl${escaped}aude`);
      expect(host.querySelector('h1')?.textContent).toBe(`cl${escaped}aude`);
      expect(host.querySelector('h1')?.parentElement?.textContent).toContain(`ma${escaped}chine`);
    }
  });
});

describe('the phone bar keeps T44 with the shared guard (#1048 item 4)', () => {
  const bar = () => host.querySelector('.phone-dock') as HTMLElement;
  const barBtn = (name: string) => [...bar().querySelectorAll('button')].find((b) => b.textContent === name) as HTMLButtonElement;
  const home = (approvals: ApprovalModel[], onResolve: Resolve, link: 'live' | 'offline' = 'live') =>
    act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={approvals} onResolve={onResolve} link={link} onOpen={() => {}} />));

  it('keeps an answer that went: "Answer sent", Allow and Deny off, even after the person comes back', async () => {
    const on = vi.fn(() => Promise.resolve(true));
    home([plain('r1', 'a.rs')], on);
    dwell();
    act(() => barBtn('Allow').click());
    await flush();
    act(() => { window.dispatchEvent(new Event('focus')); });
    dwell(60_000);
    expect(barBtn('Allow').getAttribute('aria-disabled')).toBe('true');
    act(() => barBtn('Deny').click());
    expect(on).toHaveBeenCalledTimes(1);
    expect(bar().textContent).toContain(t('approval-sent'));
  });

  it('re-arms after a fresh dwell when the answer fails, never before', async () => {
    for (const fail of [() => Promise.resolve(false), () => Promise.reject(new Error('refused')), () => { throw new Error('no key'); }] as (() => unknown)[]) {
      const on = vi.fn(fail);
      home([plain('r1', 'a.rs')], on);
      dwell();
      act(() => barBtn('Allow').click());
      await flush();
      act(() => barBtn('Allow').click());
      expect(on).toHaveBeenCalledTimes(1);
      dwell();
      act(() => barBtn('Allow').click());
      expect(on).toHaveBeenCalledTimes(2);
      await flush();
      act(() => root.render(<></>));
    }
  });

  it('is not ready the moment the person comes back', () => {
    const on = vi.fn();
    home([plain('r1', 'a.rs')], on);
    dwell();
    act(() => { window.dispatchEvent(new Event('focus')); barBtn('Allow').click(); });
    expect(on).not.toHaveBeenCalled();
    dwell();
    act(() => barBtn('Allow').click());
    expect(on).toHaveBeenCalledTimes(1);
  });

  it('starts the dwell again when the link comes back', () => {
    const on = vi.fn();
    home([plain('r1', 'a.rs')], on, 'offline');
    dwell();
    expect(barBtn('Allow').disabled).toBe(true);
    home([plain('r1', 'a.rs')], on, 'live');
    act(() => barBtn('Allow').click());
    expect(on).not.toHaveBeenCalled();
    dwell();
    act(() => barBtn('Allow').click());
    expect(on).toHaveBeenCalledTimes(1);
  });
});

describe('what a phone card draws restarts its dwell', () => {
  it('the machine the card names comes from the agent: when it changes, Allow waits again', () => {
    const on = vi.fn();
    const draw = (machine: string) => act(() => root.render(<PhoneHome agents={[{ ...agent, machine }]} machines={machines} approvals={[ask('r1', 'a.rs')]} onResolve={on} link="live" onOpen={() => {}} />));
    draw('laptop');
    dwell();
    draw('server');
    expect(cardOf('a.rs')?.textContent).toContain('server');
    act(() => btn('a.rs', 'Allow').click());
    expect(on).not.toHaveBeenCalled();
    dwell();
    act(() => btn('a.rs', 'Allow').click());
    expect(on).toHaveBeenCalledTimes(1);
  });
});

describe('one answer per request across the phone\'s screens (the window\'s holds)', () => {
  it('an answer on its way from the list\'s bar holds the request on the agent\'s screen', () => {
    const on = vi.fn(() => new Promise(() => {}));
    const list = [plain('r1', 'a.rs')];
    const inWindow = (screen: ReactElement) => act(() => root.render(<AnswerHoldsProvider listed={list.map((x) => x.id)}>{screen}</AnswerHoldsProvider>));
    inWindow(<PhoneHome key="home" agents={[agent]} machines={machines} approvals={list} onResolve={on} link="live" onOpen={() => {}} />);
    dwell();
    act(() => ([...host.querySelectorAll('.phone-dock button')].find((b) => b.textContent === 'Allow') as HTMLButtonElement).click());
    inWindow(<PhoneAgent key="agent" agent={agent} agents={[agent]} machines={machines} approvals={list} onResolve={on} onOpen={() => {}} onBack={() => {}} />);
    dwell(60_000);
    const allow = [...host.querySelectorAll('.phone-dock button')].find((b) => b.textContent === 'Allow') as HTMLButtonElement;
    expect(allow.getAttribute('aria-disabled')).toBe('true');
    act(() => allow.click());
    expect(on).toHaveBeenCalledTimes(1);
  });

  it('an answer that went from a card stays sent on the other screen while the request is listed', async () => {
    const on = vi.fn(() => true);
    const list = [ask('r1', 'a.rs')];
    const inWindow = (screen: ReactElement) => act(() => root.render(<AnswerHoldsProvider listed={list.map((x) => x.id)}>{screen}</AnswerHoldsProvider>));
    inWindow(<PhoneAgent key="agent" agent={agent} agents={[agent]} machines={machines} approvals={list} onResolve={on} onOpen={() => {}} onBack={() => {}} />);
    dwell();
    act(() => btn('a.rs', 'Deny').click());
    await flush();
    inWindow(<PhoneHome key="home" agents={[agent]} machines={machines} approvals={list} onResolve={on} link="live" onOpen={() => {}} />);
    dwell(60_000);
    expect(btn('a.rs', 'Allow').getAttribute('aria-disabled')).toBe('true');
    act(() => btn('a.rs', 'Allow').click());
    expect(on).toHaveBeenCalledTimes(1);
    expect(cardOf('a.rs')?.textContent).toContain(t('approval-sent'));
  });
});

describe('the phone\'s requests are the one request component, in the row density (redesign PR 13)', () => {
  it('draws each plain request as ApprovalRequest in the row density with its buttons under the text, an avatar and the ask whole', () => {
    act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={[ask('r1', 'a.rs'), plain('r2', 'b.rs', { expiresAtMs: 1 })]} onResolve={() => {}} link="live" onOpen={() => {}} />));
    const cards = [...host.querySelectorAll('.phone__main .phone-req > section.approval-req')];
    expect(cards.length).toBe(2);
    for (const c of cards) {
      expect(c.getAttribute('data-density')).toBe('row');
      expect(c.classList.contains('approval-req--stacked')).toBe(true);
      expect(c.querySelector('.kit-avatar')).not.toBeNull();
      expect(c.querySelector('.kit-chip')).not.toBeNull();
    }
    // The whole detail, both lines, is on the card that answers it.
    expect(cardOf('a.rs')?.querySelector('.phone-card__detail')?.textContent).toBe('a.rs\n+1 -1');
    // The plain one the bar answers says so, and keeps no second pair of buttons.
    expect(cardOf('b.rs')?.textContent).toContain(t('phone-answer-below'));
    expect([...(cardOf('b.rs')?.querySelectorAll('button') ?? [])].map((b) => b.textContent)).not.toContain('Allow');
    // Deny first, then Allow (the primary, last).
    expect([...(cardOf('a.rs')?.querySelectorAll('.kit-callout__actions button') ?? [])].map((b) => b.textContent)).toEqual(['Deny', 'Allow']);
    // No amber outline per card any more, no repeated trust warning.
    expect(host.querySelector('.phone-card--needs')).toBeNull();
  });

  it('draws the bar\'s request in the row density too, stacked, with Deny then Allow', () => {
    act(() => root.render(<PhoneHome agents={[agent]} machines={machines} approvals={[plain('r1', 'a.rs')]} onResolve={() => {}} link="live" onOpen={() => {}} />));
    const req = host.querySelector('.phone-dock section.approval-req')!;
    expect(req.getAttribute('data-density')).toBe('row');
    expect(req.classList.contains('approval-req--stacked')).toBe(true);
    expect(req.querySelector('.kit-avatar')).not.toBeNull();
    expect(req.querySelector('.approval-req__cmd')?.textContent).toBe('Edit · a.rs');
    expect([...req.querySelectorAll('.kit-callout__actions button')].map((b) => b.textContent)).toEqual(['Deny', 'Allow']);
  });

  it('names every machine that does not trust this device once, at the top, with Settings', () => {
    const other = { ...agent, id: 'n2/b', name: 'codex · rose', machine: 'server', machineId: 'n2' };
    const approvals = [ask('r1', 'a.rs'), ask('r2', 'b.rs', { node: 'n2', agentId: 'n2/b', machineName: 'server' }), ask('r3', 'c.rs')];
    act(() => root.render(<PhoneHome agents={[agent, other]} machines={machines} approvals={approvals} onResolve={() => {}} untrusted={() => true} link="live" onOpen={() => {}} />));
    const notes = [...host.querySelectorAll('.phone__main section.kit-callout:not(.approval-req)')];
    expect(notes.length).toBe(1);
    expect(notes[0]?.textContent).toContain(t('phone-untrusted-title'));
    expect(notes[0]?.textContent).toContain('laptop');
    expect(notes[0]?.textContent).toContain('server');
    expect(notes[0]?.querySelector('a[href="#/device"]')).not.toBeNull();
    for (const c of host.querySelectorAll('.phone-req')) expect(c.textContent).not.toContain('does not trust');
  });

  it('lists agents as kit rows: an avatar, the name, and the state as a pill', () => {
    act(() => root.render(<PhoneHome agents={[{ ...agent, state: 'working' }]} machines={machines} approvals={[]} onResolve={() => {}} link="live" onOpen={() => {}} />));
    const row = host.querySelector('.phone-row')!;
    expect(row.querySelector('.kit-avatar')).not.toBeNull();
    expect(row.querySelector('.kit-pill')?.textContent).toContain(t('state-working'));
  });
});

describe('the phone uses the type scale, never a pixel font size', () => {
  // visual-language.md §3: components never use pixel font sizes. A Tailwind arbitrary size in px (`text-[15px]`) or a CSS `font-size` in
  // px in any phone file fails here; use the type roles (`text-title`, `text-strong`, `text-body`, `text-caption`) or the tokens.
  const dir = join(process.cwd(), 'src/phone');
  const files = readdirSync(dir).filter((f) => /\.(tsx?|css)$/.test(f) && !/\.test\.tsx?$/.test(f));
  it('scans every phone file', () => { expect(files).toEqual(expect.arrayContaining(['Phone.tsx', 'phone.css', 'nav.ts'])); });
  it.each(files)('%s has no pixel font size', (f) => {
    const src = readFileSync(join(dir, f), 'utf8');
    expect(src.match(/\btext-\[\s*[\d.]+px\s*\]/g) ?? []).toEqual([]);
    expect(src.match(/font-size:\s*[\d.]+px/g) ?? []).toEqual([]);
    expect(src.match(/fontSize:\s*['"]?[\d.]+(px)?['"]?/g) ?? []).toEqual([]);
  });
  it('the scan finds the kinds it looks for', () => {
    expect('className="text-[15px]"'.match(/\btext-\[\s*[\d.]+px\s*\]/g)).toHaveLength(1);
    expect('font-size: 15px;'.match(/font-size:\s*[\d.]+px/g)).toHaveLength(1);
  });
});
