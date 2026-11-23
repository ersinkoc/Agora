// @vitest-environment jsdom
// #1048: the answer holds of T44 are the window's, not one surface's. A request answered, or on its way, from a card above a terminal is
// held in the side panel, on Home and in the attention dialog, and the other way round; the side panel closed and opened again finds its
// holds where they were; a pane moved or resized starts that surface's dwell again (a pane taking the focus does not); a switch's card is
// off while busy, and in the window a failed switch answer (also the phone's) frees it again. Each case is drawn in the style of cardDwell.test.tsx, with the surfaces under one window (`AnswerHoldsProvider`).
import { act, Fragment, type ReactElement, type ReactNode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { ATTENTION_DWELL_MS, attentionPrefs } from '../net/attention';
import { attentionList } from '../net/fleet';
import { forgetSkipCapable, markSkipCapable } from '../net/skipApproval';
import type { ApprovalModel } from '../net/view';
import { reduce, workspaceOf } from '../layout/engine';
import { arrangementKey } from '../layout/geometry';
import { forgetLoaded, saveWorkspace } from '../layout/store';
import { LAYOUT_VERSION, type Layout } from '../layout/tree';
import { PhoneHome } from '../phone/Phone';
import { Focus } from '../screens';
import { demoSource } from '../terminal/source';
import { ApprovalCard } from './ApprovalCard';
import { AttentionLayer } from './AttentionLayer';
import { Console } from './Console';
import { AttentionQueue } from './FleetConsole';
import { Details } from './Inspector';
import { SkipApproval } from './SkipApproval';
import { AnswerHoldsProvider } from './useAnswerGuard';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
// These tests are about request cards, not terminals: a real terminal (xterm, loaded lazily) needs browser APIs jsdom lacks
// (matchMedia, canvas), and whether its lazy import resolves inside a test depends on how warm the module cache is (#1052's CI).
vi.mock('../terminal/TerminalView', () => ({ TerminalView: () => null }));

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id: 'n1/a', name: 'claude · teal', accent: 'var(--agent-teal)', kind: 'claude', machine: 'laptop', machineId: 'n1', workspace: 'login-fix', project: 'login-fix', state: 'needs-you', preview: '', ...over });
const ask = (id: string, file: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id, node: 'n1', localId: id, agentId: 'n1/a', agentName: 'claude · teal', machineName: 'laptop', action: 'Edit', detail: file, expiresAtMs: 0, ...over });
type Resolve = (a: ApprovalModel, ok: boolean) => unknown;

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
  document.body.replaceChildren();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

// One surface's card of a request (by the file it names), inside `where`, and its buttons.
const within = (where: string) => host.querySelector(`[data-surface="${where}"]`) ?? document.body;
const cardOf = (where: string, file: string) => [...within(where).querySelectorAll('section.approval-req')].find((s) => s.textContent?.includes(file));
const btn = (where: string, file: string, name: string) => [...(cardOf(where, file)?.querySelectorAll('button') ?? [])].find((b) => b.textContent === name) as HTMLButtonElement;
const off = (b: HTMLButtonElement) => b.disabled || b.getAttribute('aria-disabled') === 'true';
const dwell = (ms = ATTENTION_DWELL_MS) => act(() => { vi.advanceTimersByTime(ms); });
const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

const Surface = ({ name, children }: { name: string; children: ReactNode }) => <div data-surface={name}>{children}</div>;
const banner = (approvals: ApprovalModel[], onResolve: Resolve) => <Surface name="banner"><Focus agent={agent()} approvals={approvals} onResolve={onResolve as never} onBack={() => {}} /></Surface>;
// The card above a chat's composer: a banner with no terminal behind it, for the tests that let async timers run (a terminal would load).
const composer = (approvals: ApprovalModel[], onResolve: Resolve) => <Surface name="banner"><Focus agent={agent({ terminal: false })} approvals={approvals} onResolve={onResolve as never} onBack={() => {}} /></Surface>;
const panel = (approvals: ApprovalModel[], onResolve: Resolve) => <Surface name="panel"><Details tab="needs" fleet={[agent()]} attention={[agent()]} approvals={approvals} changed={[]} boards={[]} messages={[]}
  onOpen={() => {}} onResolve={onResolve as never} /></Surface>;
const home = (approvals: ApprovalModel[], onResolve: Resolve) => <Surface name="home"><AttentionQueue items={attentionList(approvals, [agent()])} machines={[]} now={0} onOpen={() => {}} onResolve={onResolve as never} /></Surface>;
const windowOf = (approvals: ApprovalModel[], ...surfaces: ReactElement[]) => <AnswerHoldsProvider listed={approvals.map((x) => x.id)}>{surfaces}</AnswerHoldsProvider>;
const k = (key: string, el: ReactElement) => <Fragment key={key}>{el}</Fragment>;
const draw = (el: ReactElement) => act(() => root.render(el));

describe('a request is held for the whole window, whichever surface answered it (#1048)', () => {
  const pairs: [string, string, typeof banner, typeof banner][] = [
    ['banner', 'panel', banner, panel],
    ['panel', 'banner', panel, banner],
    ['home', 'panel', home, panel],
    ['panel', 'home', panel, home],
    ['banner', 'home', banner, home],
  ];
  it.each(pairs)('answered on the %s, it is held on the %s while the answer goes and kept answered once it went', async (from, to, a, b) => {
    const sent: [string, boolean][] = [];
    let settle: (v: boolean) => void = () => {};
    const onResolve: Resolve = (x, ok) => { sent.push([x.id, ok]); return new Promise<boolean>((r) => { settle = r; }); };
    const list = [ask('r1', 'a.rs')];
    draw(windowOf(list, k('a', a(list, onResolve)), k('b', b(list, onResolve))));
    dwell();
    expect(off(btn(to, 'a.rs', 'Allow'))).toBe(false);
    act(() => btn(from, 'a.rs', 'Allow').click());
    expect(sent).toEqual([['r1', true]]);
    // On its way: the other surface's answers are off, and a press there sends nothing, also after any wait or a return to the window.
    expect(off(btn(to, 'a.rs', 'Allow'))).toBe(true);
    expect(off(btn(to, 'a.rs', 'Deny'))).toBe(true);
    act(() => { window.dispatchEvent(new Event('focus')); });
    dwell(60_000);
    act(() => btn(to, 'a.rs', 'Deny').click());
    act(() => btn(to, 'a.rs', 'Allow').click());
    expect(sent).toEqual([['r1', true]]);
    // It went: still never a second answer, from either surface.
    settle(true);
    await flush();
    dwell(60_000);
    act(() => btn(to, 'a.rs', 'Allow').click());
    act(() => btn(from, 'a.rs', 'Deny').click());
    expect(sent).toEqual([['r1', true]]);
    expect(off(btn(to, 'a.rs', 'Allow'))).toBe(true);
  });

  it('two presses on two surfaces in one batch send one answer', () => {
    const sent: string[] = [];
    const onResolve: Resolve = (x) => { sent.push(x.id); return new Promise(() => {}); };
    const list = [ask('r1', 'a.rs')];
    draw(windowOf(list, k('a', banner(list, onResolve)), k('b', panel(list, onResolve)), k('c', home(list, onResolve))));
    dwell();
    act(() => { btn('banner', 'a.rs', 'Allow').click(); btn('panel', 'a.rs', 'Deny').click(); btn('home', 'a.rs', 'Allow').click(); });
    expect(sent).toEqual(['r1']);
  });

  it('a failed answer frees the request on every surface, each after a fresh dwell of its own', async () => {
    const sent: string[] = [];
    const onResolve: Resolve = (x) => { sent.push(x.id); return Promise.resolve(false); };
    const list = [ask('r1', 'a.rs')];
    draw(windowOf(list, k('a', banner(list, onResolve)), k('b', panel(list, onResolve))));
    dwell();
    act(() => btn('banner', 'a.rs', 'Allow').click());
    await flush();
    act(() => btn('panel', 'a.rs', 'Allow').click()); // the panel's dwell started again when the answer came back
    expect(sent).toEqual(['r1']);
    dwell();
    act(() => btn('panel', 'a.rs', 'Allow').click());
    expect(sent).toEqual(['r1', 'r1']);
    await flush(); // that answer settles inside this test, not in the next one
  });

  it('the attention dialog shares the holds: answered on a banner it cannot answer, and answered in it the panel holds it', async () => {
    vi.spyOn(document, 'hasFocus').mockImplementation(() => true);
    attentionPrefs.reset();
    const sent: string[] = [];
    const onResolve: Resolve = (x) => { sent.push(x.id); return new Promise(() => {}); };
    const dialog = (list: ApprovalModel[]) => <AttentionLayer key="d" items={attentionList(list, [agent()])} onResolve={onResolve as never} onReview={() => {}} />;
    const dialogBtn = (name: string) => [...(document.querySelector('.attention-card')?.querySelectorAll('button') ?? [])].find((b) => b.textContent === name) as HTMLButtonElement;
    // Answered on the banner first: the dialog's Allow stays off.
    const one = [ask('r1', 'a.rs')];
    draw(windowOf(one, k('a', composer(one, onResolve)), dialog(one)));
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_DWELL_MS + 600); });
    expect(dialogBtn('Allow')).toBeTruthy();
    act(() => btn('banner', 'a.rs', 'Allow').click());
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_DWELL_MS * 2); });
    expect(off(dialogBtn('Allow'))).toBe(true);
    act(() => dialogBtn('Allow').click());
    expect(sent).toEqual(['r1']);
    // Answered in the dialog: the panel holds it.
    act(() => root.render(<></>));
    sent.length = 0;
    const two = [ask('r2', 'b.rs')];
    draw(windowOf(two, k('b', panel(two, onResolve)), dialog(two)));
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_DWELL_MS + 600); });
    act(() => dialogBtn('Allow').click());
    expect(sent).toEqual(['r2']);
    dwell(60_000);
    expect(off(btn('panel', 'b.rs', 'Allow'))).toBe(true);
    act(() => btn('panel', 'b.rs', 'Allow').click());
    expect(sent).toEqual(['r2']);
  });
});

describe('the attention dialog and the window\'s holds (#1048)', () => {
  const dialogBtn = (name: string) => [...(document.querySelector('.attention-card')?.querySelectorAll('button') ?? [])].find((b) => b.textContent === name) as HTMLButtonElement;
  const later = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
  beforeEach(() => { vi.spyOn(document, 'hasFocus').mockImplementation(() => true); attentionPrefs.reset(); });

  it('an answer from a banner that failed frees the dialog only after a fresh dwell of its own', async () => {
    const sent: string[] = [];
    const fromBanner: Resolve = (x) => { sent.push(`banner:${x.id}`); return Promise.resolve(false); };
    const fromDialog: Resolve = (x) => { sent.push(`dialog:${x.id}`); return new Promise(() => {}); };
    const one = [ask('r1', 'a.rs')];
    draw(windowOf(one, k('a', composer(one, fromBanner)), <AttentionLayer key="d" items={attentionList(one, [agent()])} onResolve={fromDialog as never} onReview={() => {}} />));
    await later(ATTENTION_DWELL_MS + 600);
    act(() => btn('banner', 'a.rs', 'Allow').click());
    await flush();
    expect(sent).toEqual(['banner:r1']);
    act(() => dialogBtn('Allow').click()); // open again, but the dialog's dwell started again with the settle
    expect(sent).toEqual(['banner:r1']);
    await later(ATTENTION_DWELL_MS);
    act(() => dialogBtn('Allow').click());
    expect(sent).toEqual(['banner:r1', 'dialog:r1']);
  });

  it('a dialog on a request answered elsewhere says it was sent and offers Review, not dead buttons alone', async () => {
    const onResolve: Resolve = () => true;
    const one = [ask('r1', 'a.rs')];
    draw(windowOf(one, k('a', composer(one, onResolve))));
    dwell();
    act(() => btn('banner', 'a.rs', 'Allow').click());
    await flush();
    draw(windowOf(one, k('a', composer(one, onResolve)), <AttentionLayer key="d" items={attentionList(one, [agent()])} onResolve={onResolve as never} onReview={() => {}} />));
    await later(ATTENTION_DWELL_MS * 2);
    expect(document.querySelector('.attention-card')?.textContent).toContain(t('attention-sent'));
    expect(dialogBtn('Review')).toBeTruthy();
    expect(off(dialogBtn('Allow'))).toBe(true);
  });
});

describe('the side panel closed and opened again keeps its holds (#1048)', () => {
  it('an answer on its way stays held, and one that went stays answered, while the window lists the request', async () => {
    const sent: string[] = [];
    let settle: (v: boolean) => void = () => {};
    const onResolve: Resolve = (x) => { sent.push(x.id); return new Promise<boolean>((r) => { settle = r; }); };
    const list = [ask('r1', 'a.rs')];
    draw(windowOf(list, k('p', panel(list, onResolve))));
    dwell();
    act(() => btn('panel', 'a.rs', 'Allow').click());
    draw(windowOf(list)); // closed
    draw(windowOf(list, k('p', panel(list, onResolve)))); // opened again
    dwell(60_000);
    expect(off(btn('panel', 'a.rs', 'Allow'))).toBe(true);
    act(() => btn('panel', 'a.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
    settle(true);
    await flush();
    draw(windowOf(list));
    draw(windowOf(list, k('p', panel(list, onResolve))));
    dwell(60_000);
    expect(cardOf('panel', 'a.rs')?.textContent).toContain(t('approval-sent'));
    act(() => btn('panel', 'a.rs', 'Deny').click());
    expect(sent).toEqual(['r1']);
  });
  it('an answer that went is forgotten once the window no longer lists its request (as before: a new request under that id is answerable)', async () => {
    const sent: string[] = [];
    const onResolve: Resolve = (x) => { sent.push(x.id); return true; };
    const list = [ask('r1', 'a.rs')];
    draw(windowOf(list, k('p', panel(list, onResolve))));
    dwell();
    act(() => btn('panel', 'a.rs', 'Allow').click());
    await flush();
    draw(windowOf([], k('p', panel([], onResolve))));
    draw(windowOf(list, k('p', panel(list, onResolve))));
    dwell();
    act(() => btn('panel', 'a.rs', 'Allow').click());
    expect(sent).toEqual(['r1', 'r1']);
    await flush(); // that answer settles inside this test, not in the next one
  });
});

describe('a pane rearranged starts the dwell again (#1048)', () => {
  const layout = (): Layout => ({ version: LAYOUT_VERSION, root: { type: 'split', direction: 'row', ratios: [0.5, 0.5], children: [
    { type: 'pane', id: 'p1', panel: 'agent', bind: { agent: 'n1/a' } }, { type: 'pane', id: 'p2', panel: 'empty' }] } });

  it('the key changes for a move, a swap, a resize, a maximize, a column dragged and a pane showing another agent, never for a focus', () => {
    const ws = workspaceOf(layout());
    const key = arrangementKey(ws, false, [272, 336]);
    expect(arrangementKey(reduce(ws, { type: 'focus', pane: 'p2' }), false, [272, 336])).toBe(key);
    expect(arrangementKey(reduce(ws, { type: 'focus', pane: 'p1' }), false, [272, 336])).toBe(key);
    expect(arrangementKey(reduce(ws, { type: 'swap', a: 'p1', b: 'p2' }), false, [272, 336])).not.toBe(key);
    expect(arrangementKey(reduce(ws, { type: 'move', pane: 'p1', target: 'p2', place: 'bottom' }), false, [272, 336])).not.toBe(key);
    expect(arrangementKey(reduce(ws, { type: 'resize', path: [], index: 0, delta: 0.1 }), false, [272, 336])).not.toBe(key);
    expect(arrangementKey(reduce(ws, { type: 'split', pane: 'p2', direction: 'column' }), false, [272, 336])).not.toBe(key);
    expect(arrangementKey(reduce(ws, { type: 'maximize', pane: 'p1' }), false, [272, 336])).not.toBe(key);
    expect(arrangementKey(ws, false, [288, 336])).not.toBe(key);
    // A pane pointed at another agent (or a following pane that follows another one now) puts another card where this one was.
    expect(arrangementKey(reduce(ws, { type: 'bind', pane: 'p1', bind: { agent: 'n1/b' }, panel: 'agent' }), false, [272, 336])).not.toBe(key);
    const follows = (agent: string) => arrangementKey(ws, false, [272, 336], (p) => (p.id === 'p2' ? agent : undefined));
    expect(follows('n1/a')).not.toBe(follows('n1/b'));
  });

  it('in the window: a card above a terminal waits again after its pane moves, and a click that focuses a pane holds nothing back', () => {
    forgetLoaded();
    saveWorkspace({ ws: workspaceOf(layout()), touched: true });
    const sent: string[] = [];
    const onResolve: Resolve = (x) => { sent.push(x.id); return new Promise(() => {}); };
    const a = agent({ terminal: true });
    const list = [ask('r1', 'a.rs'), ask('r2', 'b.rs')];
    draw(windowOf(list, <Console key="c" agents={[a]} machines={[]} approvals={list} messages={[]} terminalFor={() => demoSource('x')} onOpen={() => {}} onResolve={onResolve as never} link="live" />));
    const pane = () => host.querySelector('[data-pane="p1"]') as HTMLElement;
    const inPane = (file: string, name: string) => [...([...pane().querySelectorAll('section.approval-req')].find((s) => s.textContent?.includes(file))?.querySelectorAll('button') ?? [])].find((b) => b.textContent === name) as HTMLButtonElement;
    expect(inPane('a.rs', 'Allow')).toBeTruthy();
    dwell();
    // A click in the other pane takes the focus there: nothing moved, so the card stays ready.
    act(() => { (host.querySelector('[data-pane="p2"]') as HTMLElement).dispatchEvent(new Event('pointerdown', { bubbles: true })); });
    act(() => inPane('a.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
    // That answer changed what the cards show (one guard for every card in the panes): after its dwell the other card is ready.
    dwell();
    expect(inPane('b.rs', 'Allow').hasAttribute('aria-disabled')).toBe(false);
    // The pane is swapped with the other one (its menu): the card is somewhere else now, so it waits again.
    const menu = [...host.querySelectorAll('button[aria-haspopup="menu"]')].find((b) => b.getAttribute('aria-label') === t('layout-pane-menu', { name: a.name })) as HTMLButtonElement;
    act(() => menu.click());
    const item = (label: string) => [...document.querySelectorAll('[role=menu] button')].find((b) => b.textContent?.startsWith(label)) as HTMLButtonElement;
    act(() => item(t('layout-swap-with')).click());
    act(() => item(t('layout-panel-empty')).click());
    expect(pane().getAttribute('style')).toContain('inset-inline: 50% 0%');
    expect(inPane('b.rs', 'Allow').getAttribute('aria-disabled')).toBe('true');
    act(() => inPane('b.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
    dwell(ATTENTION_DWELL_MS - 1);
    act(() => inPane('b.rs', 'Allow').click());
    expect(sent).toEqual(['r1']);
    dwell(1);
    act(() => inPane('b.rs', 'Allow').click());
    expect(sent).toEqual(['r1', 'r2']);
  });
});

describe('a switch\'s card honours busy (#1048)', () => {
  const skip = (): ApprovalModel => ask('s1', '', { action: 'agent.skip_permissions', detail: '{}', skip: { node: 'n1', agent: 'n1/a', agentKind: 'claude-acp', folder: '/w', optionType: 'mode', option: 'mode', optionName: '', value: 'yolo', valueName: 'Bypass' } });
  // The whole card (itself, or in Review's dialog, which is drawn over the window) and the short card's own Deny.
  const named = (name: string) => [...document.querySelectorAll('[data-skip-approval] button')].find((b) => b.textContent === name) as HTMLButtonElement;
  const short = (name: string) => [...document.querySelectorAll('[data-review-card] button')].find((b) => b.textContent === name) as HTMLButtonElement;
  it('Allow and Deny are off while busy, a press sends nothing, and both come back when it is not', () => {
    markSkipCapable('n1');
    const on = vi.fn();
    for (const [el, review] of [[(busy: boolean) => <SkipApproval approval={skip()} busy={busy} onResolve={on} />, false], [(busy: boolean) => <ApprovalCard approval={skip()} busy={busy} onResolve={on} />, true]] as const) {
      draw(el(true));
      if (review) {
        expect(short('Deny').disabled).toBe(true);
        act(() => short('Deny').click());
        act(() => short('Review').click()); // Review is never an answer: it opens while busy too
      }
      dwell(ATTENTION_DWELL_MS * 2);
      expect(named('Deny').disabled).toBe(true);
      expect(named('Allow').disabled).toBe(true);
      act(() => { named('Deny').click(); named('Allow').click(); });
      expect(on).not.toHaveBeenCalled();
      draw(el(false));
      if (review) expect(short('Deny').disabled).toBe(false);
      expect(named('Deny').disabled).toBe(false);
      expect(named('Allow').disabled).toBe(false);
      act(() => root.render(<></>));
    }
    forgetSkipCapable();
  });
  it('in the window, a switch answer that failed is answerable again after its lock, and one that went stays off', async () => {
    markSkipCapable('n1');
    for (const [went, back] of [[false, true], [true, false]] as const) {
      const on = vi.fn(() => Promise.resolve(went));
      const list = [skip()];
      draw(windowOf(list, <SkipApproval key="a" approval={skip()} onResolve={on} />));
      dwell();
      act(() => named('Allow').click());
      await flush();
      dwell(4100);
      expect(named('Allow').disabled).toBe(!back);
      expect(named('Deny').disabled).toBe(!back);
      act(() => named('Deny').click());
      expect(on).toHaveBeenCalledTimes(back ? 2 : 1);
      await flush();
      act(() => root.render(<></>));
    }
    forgetSkipCapable();
  });

  it('on the phone, a switch answer that failed frees the card again (the phone\'s answer says how it went)', async () => {
    markSkipCapable('n1');
    const on = vi.fn(() => Promise.resolve(false));
    const list = [skip()];
    const machines = [{ id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: 0 }];
    draw(windowOf(list, <PhoneHome key="p" agents={[agent()]} machines={machines} approvals={list} onResolve={on as never} link="live" onOpen={() => undefined} />));
    await act(async () => { await vi.advanceTimersByTimeAsync(ATTENTION_DWELL_MS + 700); });
    act(() => named('Allow').click());
    expect(on).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(4100); });
    expect(named('Allow').disabled).toBe(false);
    act(() => named('Allow').click());
    expect(on).toHaveBeenCalledTimes(2);
    await act(async () => { await vi.advanceTimersByTimeAsync(0); }); // that answer settles inside this test, not in the next one
    forgetSkipCapable();
  });

  it('is held with the window: answered from one card, the same switch on another surface is off', () => {
    markSkipCapable('n1');
    const on = vi.fn(() => new Promise(() => {}));
    const list = [skip()];
    draw(windowOf(list, <Surface key="a" name="a"><SkipApproval approval={skip()} onResolve={on} /></Surface>, <Surface key="b" name="b"><SkipApproval approval={skip()} onResolve={on} /></Surface>));
    dwell();
    const inside = (where: string, name: string) => [...host.querySelectorAll(`[data-surface="${where}"] [data-skip-approval] button`)].find((b) => b.textContent === name) as HTMLButtonElement;
    act(() => inside('a', 'Deny').click());
    expect(on).toHaveBeenCalledTimes(1);
    dwell(10_000);
    expect(inside('b', 'Deny').disabled).toBe(true);
    act(() => inside('b', 'Allow').click());
    expect(on).toHaveBeenCalledTimes(1);
    forgetSkipCapable();
  });
});
