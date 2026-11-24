// @vitest-environment jsdom
// #1045: the side panel's request cards and the cards above a terminal and above a chat's composer keep T44 exactly as Home's queue does
// (homeRedesign.test.tsx): Allow and Deny do nothing for a short dwell after the list changed, after the person came back and after the
// link came back; a request is answered once, held while its answer goes, kept answered once it went, and answerable again after a fresh
// dwell when the answer failed. One mechanism (`useAnswerGuard`), so each case runs on every surface.
import { act, type ReactElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { ATTENTION_DWELL_MS } from '../net/attention';
import type { ApprovalModel } from '../net/view';
import { Focus } from '../screens';
import { ApprovalBanner } from './ApprovalBanner';
import { ApprovalCard } from './ApprovalCard';
import { Details } from './Inspector';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
// These tests are about request cards, not terminals: a real terminal (xterm, loaded lazily) needs browser APIs jsdom lacks
// (matchMedia, canvas), and whether its lazy import resolves inside a test depends on how warm the module cache is (#1052's CI).
vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => null }));

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id: 'n1/a', name: 'claude · teal', accent: 'var(--agent-teal)', kind: 'claude', machine: 'laptop', machineId: 'n1', workspace: 'login-fix', project: 'login-fix', state: 'needs-you', preview: '', ...over });
const ask = (id: string, file: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id, node: 'n1', localId: id, agentId: 'n1/a', agentName: 'claude · teal', machineName: 'laptop', action: 'Edit', detail: file, expiresAtMs: 0, ...over });
type Resolve = (a: ApprovalModel, ok: boolean) => unknown;
interface Opts { stale?: boolean }

// Each surface, drawn with a list of requests from one agent.
const surfaces: [string, (approvals: ApprovalModel[], onResolve: Resolve, o?: Opts) => ReactElement][] = [
  ['the side panel', (approvals, onResolve, o = {}) => <Details tab="needs" fleet={[agent()]} attention={[agent()]} approvals={approvals} changed={[]} boards={[]} messages={[]}
    onOpen={() => {}} onResolve={onResolve as never} {...{ stale: o.stale }} />],
  ['above a terminal', (approvals, onResolve, o = {}) => <Focus agent={agent()} approvals={approvals} onResolve={onResolve as never} {...{ stale: o.stale }} onBack={() => {}} />],
  ['above a composer', (approvals, onResolve, o = {}) => <Focus agent={agent({ terminal: false })} approvals={approvals} onResolve={onResolve as never} {...{ stale: o.stale }} onBack={() => {}} />],
];

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  vi.useFakeTimers();
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

// The card of one request (by the file it names), and its buttons.
const cardOf = (file: string) => [...host.querySelectorAll('section.approval-req')].find((s) => s.querySelector('.approval-req__cmd')?.textContent?.includes(file));
const btn = (file: string, name: string) => [...(cardOf(file)?.querySelectorAll('button') ?? [])].find((b) => b.textContent === name) as HTMLButtonElement;
const dwell = (ms = ATTENTION_DWELL_MS) => act(() => { vi.advanceTimersByTime(ms); });
const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

describe.each(surfaces)('request cards %s keep T44 (#1045)', (_name, draw) => {
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
    render([ask('r1', 'a.rs')], onResolve);
    dwell();
    render([ask('r2', 'b.rs'), ask('r1', 'a.rs')], onResolve);
    act(() => btn('a.rs', 'Allow').click());
    act(() => btn('b.rs', 'Allow').click());
    expect(sent).toEqual([]);
    dwell();
    // The machine it runs on changes, the request does not.
    render([ask('r2', 'b.rs'), ask('r1', 'a.rs', { machineName: 'server' })], onResolve);
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
    // A timer started before the return does not make the card ready early.
    dwell(ATTENTION_DWELL_MS - 100);
    act(() => { window.dispatchEvent(new Event('focus')); });
    dwell(100);
    act(() => btn('a.rs', 'Allow').click());
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
      expect(btn('a.rs', 'Deny').getAttribute('aria-disabled')).toBe('true');
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
      act(() => btn('a.rs', 'Allow').click()); // the fresh dwell has not passed
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
});

describe('a card drawn alone has a dwell of its own (#1045)', () => {
  it('the side panel\'s card and the banner wait it out, and answer once', () => {
    for (const draw of [(on: () => void) => <ApprovalCard approval={ask('r1', 'a.rs')} onResolve={on} />, (on: () => void) => <ApprovalBanner approval={ask('r1', 'a.rs')} onResolve={on} />]) {
      const on = vi.fn();
      act(() => root.render(draw(on)));
      act(() => btn('a.rs', 'Allow').click());
      expect(on).not.toHaveBeenCalled();
      dwell();
      act(() => { btn('a.rs', 'Allow').click(); btn('a.rs', 'Allow').click(); });
      expect(on).toHaveBeenCalledTimes(1);
      act(() => root.render(<></>));
    }
  });
  it('a start request through the card waits too, and is answered once', () => {
    const start = { ...ask('r9', 'x'), action: 'agent.start', start: { agent: '', agentKind: '', node: '', kind: 'k', folder: '/w', prompt: 'p', inside: true, worktree: false, depth: 1, grantAllowed: false, settings: [], options: [] } } as ApprovalModel;
    const on = vi.fn();
    act(() => root.render(<ApprovalCard approval={start} onResolve={on} />));
    dwell(); // the card's own dwell ran out: Review's dialog starts one of its own (a second click on Review never lands on Allow)
    act(() => ([...host.querySelectorAll('button')].find((b) => b.textContent === 'Review') as HTMLButtonElement).click());
    const allow = () => [...document.querySelectorAll('[data-review-dialog] button')].find((b) => b.textContent === 'Allow') as HTMLButtonElement;
    expect(allow().disabled).toBe(true);
    act(() => allow().click());
    expect(on).not.toHaveBeenCalled();
    dwell();
    act(() => { allow().click(); allow().click(); });
    expect(on).toHaveBeenCalledTimes(1);
  });
});
