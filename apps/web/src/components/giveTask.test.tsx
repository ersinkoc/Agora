// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel, SessionModel } from '../model';
import { declineGive, forgetGiveSupport, forgetOffline, givableTasks, giveAsk, giveCandidates, giveRefusal, giveRequest, giveTask, noteRefusal, offersGive, resendGive, type GiveResult, type GiveTask } from '../net/give';
import { grantFor } from '../net/grant';
import { REFUSAL_KEYS, refusalKey } from '../net/refusal';
import type { BoardModel, BoardTaskModel } from '../net/view';
import { BoardColumns } from './BoardView';
import { StartAgent } from './StartAgent';
import { BoardView } from './BoardView';
import { APPEAR_WAIT_MS, GiveContext, GiveDialog, type GiveApi, type StartForTask } from './GiveTask';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); forgetGiveSupport(); giveAsk.cancel(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); forgetGiveSupport(); giveAsk.cancel(); });

const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id, machineId: id.split('/')[0], name: `claude · ${id.split('/')[1]}`, kind: 'claude', accent: '#fff', machine: id.split('/')[0] === 'n1' ? 'laptop' : 'server',
  workspace: '/w', state: 'idle', preview: '', projectId: 'p', ...over,
});
const task = (id: string, status: BoardTaskModel['status'], extra: Partial<BoardTaskModel> = {}): BoardTaskModel => ({ id: `p/${id}`, taskId: id, title: `Task ${id}`, status, owner: status === 'open' ? '' : 'Ada', ...extra });
const board = (tasks: BoardTaskModel[]): BoardModel => ({ projectId: 'p', project: 'shop', folders: [], tasks, leases: [] });
const theTask: GiveTask = { key: 'p/t-1', projectId: 'p', taskId: 't-1', project: 'shop', title: 'Write the release notes' };
const machines = [{ id: 'n1', name: 'laptop' }, { id: 'n2', name: 'server' }];
const api = (agents: AgentTileModel[], send: GiveApi['send'] = async () => ({ ok: true, queued: false })): GiveApi => ({ agents, online: new Set(['n1', 'n2']), machines, send });

const render = (node: React.ReactNode) => act(() => root.render(node));
const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });
const dialog = () => document.querySelector('[data-give-dialog]') as HTMLElement;
const button = (text: string, scope: ParentNode = document) => [...scope.querySelectorAll('button')].find((b) => b.textContent?.trim() === text) as HTMLButtonElement | undefined;
const click = (el: Element | null | undefined) => { if (!el) throw new Error('missing element'); act(() => { (el as HTMLElement).click(); }); };

describe('Give to an agent on the board', () => {
  it('is offered on Backlog cards only', () => {
    render(<GiveContext.Provider value={api([agent('n1/a-1')])}><BoardColumns board={board([task('1', 'open'), task('2', 'claimed'), task('3', 'done'), task('4', 'open')])} /></GiveContext.Provider>);
    const actions = [...host.querySelectorAll('button')].filter((b) => b.textContent === 'Give to an agent');
    expect(actions).toHaveLength(2);
    for (const b of actions) expect(b.closest('[data-column]')?.getAttribute('data-column')).toBe('backlog');
    // Each card's action names its task for a screen reader, and opens the dialog for exactly that task.
    expect(actions[0]!.getAttribute('aria-label')).toBe('Give “Task 1” to an agent');
    click(actions[1]);
    expect(giveAsk.get()).toEqual({ key: 'p/4', projectId: 'p', taskId: '4', project: 'shop', title: 'Task 4' });
  });

  it('is hidden without a hub, without an agent of the project, and on a machine that does not take tasks', () => {
    const cards = (give?: GiveApi) => {
      render(give ? <GiveContext.Provider value={give}><BoardColumns board={board([task('1', 'open')])} /></GiveContext.Provider> : <BoardColumns board={board([task('1', 'open')])} />);
      return [...host.querySelectorAll('button')].filter((b) => b.textContent === 'Give to an agent').length;
    };
    expect(cards()).toBe(0);
    expect(cards(api([agent('n1/a-1', { projectId: 'other' })]))).toBe(0);
    expect(cards(api([agent('n1/a-1')]))).toBe(1);
    // The hub (or an older node) refused it for that machine: `task.dispatch` is not offered there.
    act(() => noteRefusal('n1', 'protocol.capability_missing', { capability: 'task.dispatch' }));
    expect(offersGive('n1')).toBe(false);
    expect(cards(api([agent('n1/a-1')]))).toBe(0);
    expect(cards(api([agent('n1/a-1'), agent('n2/a-2')]))).toBe(1);
    // A node too old to decode the command (or a hub that does not know it: the window cannot tell) hides that machine only.
    act(() => noteRefusal('n2', 'protocol.invalid_message', { field: 'command' }));
    expect(cards(api([agent('n2/a-2')]))).toBe(0);
    expect(cards({ ...api([agent('n3/a-3')]), online: undefined })).toBe(1);
  });

  it('lists running agents of the task\'s project only, helpers under their parent', () => {
    const list = giveCandidates([
      agent('n1/helper', { parentId: 'n2/parent' }),
      agent('n2/parent'),
      agent('n1/other', { projectId: 'q' }),
      agent('n1/closed', { closed: true, state: 'finished' }),
      agent('n1/sh', { kind: 'shell' }),
      // ADR-0094: a Shell shown as the agent a person runs in it is still a Shell: it takes no task.
      agent('n1/sh-claude', { kind: 'claude-code', hostKind: 'shell' }),
      agent('n3/away', { machineId: 'n3' }),
    ], 'p', new Set(['n1', 'n2']));
    expect(list.map(({ agent: a, depth }) => [a.id, depth])).toEqual([['n2/parent', 0], ['n1/helper', 1]]);
    expect(giveCandidates([agent('n1/a')], '')).toEqual([]);
  });

  it('shows each agent\'s name, kind, machine, state and the "acts freely" badge in the picker', () => {
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1', { skipsPermissions: true, state: 'working' }), agent('n2/b-2', { parentId: 'n1/a-1' }), agent('n1/x', { projectId: 'q', name: 'claude · elsewhere' })])} onClose={() => {}} />);
    const rows = [...dialog().querySelectorAll('[data-agent-id]')];
    expect(rows.map((r) => r.getAttribute('data-agent-id'))).toEqual(['n1/a-1', 'n2/b-2']);
    expect(rows[0]!.textContent).toContain('claude · a-1');
    expect(rows[0]!.textContent).toContain('claude on laptop');
    expect(rows[0]!.textContent).toContain('Working');
    expect(rows[0]!.querySelector('[data-no-ask="started"]')?.getAttribute('aria-label')).toMatch(/^Runs without asking: /);
    expect(rows[1]!.textContent).toContain('Helper · claude on server');
    expect(rows[1]!.querySelector('[data-no-ask]')).toBeNull();
    expect(dialog().textContent).not.toContain('elsewhere');
    for (const b of rows.map((r) => r.querySelector('button')!)) expect(b.className).toContain('min-h-11');
  });

  it('confirms with agent, kind, machine and project in full and the warning for an agent that acts without asking', () => {
    const long = 'claude · a very long agent name that is not cut short anywhere in the confirmation';
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1', { skipsPermissions: true, name: long })])} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    const text = dialog().textContent ?? '';
    expect(text).toContain(`Give this task to ${long}?`);
    expect(text).toContain('Write the release notes');
    expect(text).toContain('laptop');
    expect(text).toContain('shop');
    expect(text).toContain('claude');
    expect(text).toContain(`${long} acts without asking`);
    // Nothing is sent before the person says so.
    expect(dialog().querySelector('[data-give-step="confirm"]')).not.toBeNull();
  });

  it('says nothing about acting freely for an agent that asks', () => {
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')])} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    expect(dialog().textContent).not.toContain('acts without asking');
    expect(dialog().querySelector('[role=note]')).toBeNull();
  });

  it('names two machines with the same name by their id', () => {
    const twin = { ...api([agent('n1/a-1', { machine: 'box' })]), machines: [{ id: 'n1', name: 'box' }, { id: 'n2', name: 'box' }] };
    render(<GiveDialog task={theTask} give={twin} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    expect(dialog().textContent).toContain(t('machine-name-id', { name: 'box', id: 'n1' }));
  });

  const sendWith = async (result: GiveResult) => {
    const send = vi.fn(async () => result);
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], send)} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    click(button('Give the task', dialog()));
    await flush();
    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0]).toEqual([expect.objectContaining({ id: 'n1/a-1' }), theTask]);
    return dialog().textContent ?? '';
  };
  it('says "sent" when the agent got it now', async () => {
    expect(await sendWith({ ok: true, queued: false })).toContain('Sent to claude · a-1.');
    expect(dialog().querySelector('[role=status]')).not.toBeNull();
  });
  it('says it waits for the turn to end when it was queued', async () => {
    expect(await sendWith({ ok: true, queued: true })).toContain('It waits until the agent\'s current turn ends.');
  });
  it('shows the refusal sentence, and lets the person choose another agent', async () => {
    const text = await sendWith({ ok: false, ...giveRefusal('task.not_open') });
    expect(text).toContain('This task was already taken by an agent, or it is done.');
    expect(dialog().querySelector('[role=alert]')).not.toBeNull();
    click(button('Choose another agent', dialog()));
    expect(dialog().querySelector('[data-agent-id]')).not.toBeNull();
  });

  for (const lang of ['en', 'tr'] as const) {
    it(`uses the UI vocabulary and shows task text as text (${lang})`, () => {
      setLanguage(lang);
      const evil = { ...theTask, title: '<img src=x onerror=alert(1)> ‮evil' };
      render(<GiveDialog task={evil} give={api([agent('n1/a-1', { skipsPermissions: true })])} onClose={() => {}} />);
      const picker = dialog().innerHTML;
      expect(dialog().querySelector('img')).toBeNull();
      expect(picker).not.toContain('‮');
      click(dialog().querySelector('[data-agent-id] button'));
      const all = `${picker} ${dialog().textContent}`;
      expect(all).not.toMatch(/claim|lease|mesh|dispatch/i);
      expect(all).not.toMatch(/>give-[a-z-]+</);
    });
  }
});

describe('the dispatch command', () => {
  it('goes to the agent\'s machine with the agent\'s own id and an empty byUser (the hub writes who asked)', () => {
    expect(giveRequest(agent('n2/agent-7'), theTask)).toEqual({ node: 'n2', body: { taskDispatch: { projectId: 'p', taskId: 't-1', agentId: 'agent-7', byUser: '' } } });
  });

  it('reads queued from the answer, and a refusal as its sentence', async () => {
    const ok = vi.fn(async () => ({ taskDispatch: { queued: true, queueLen: 2, promptSha256: 'ab' } }));
    expect(await giveTask(ok, agent('n1/a-1'), theTask)).toEqual({ ok: true, queued: true });
    expect(ok).toHaveBeenCalledWith('n1', { taskDispatch: { projectId: 'p', taskId: 't-1', agentId: 'a-1', byUser: '' } });
    expect(await giveTask(async () => ({ taskDispatch: { queued: false } }), agent('n1/a-1'), theTask)).toEqual({ ok: true, queued: false });
    const refuse = (code: string, params: Record<string, string> = {}) => giveTask(async () => { throw { code, params }; }, agent('n1/a-1'), theTask);
    expect(await refuse('task.blocked')).toEqual({ ok: false, key: 'refusal-task-blocked' });
    expect(await refuse('task.dispatch_limited', { limit: '8' })).toEqual({ ok: false, key: 'give-refused-limited', args: { limit: '8' } });
    // A terminal program without bracketed paste is about that agent, not the machine: the machine is still offered.
    expect(await refuse('protocol.capability_missing', { capability: 'terminal.bracketed_paste' })).toEqual({ ok: false, key: 'give-refused-paste' });
    expect(offersGive('n1')).toBe(true);
    expect(await refuse('protocol.capability_missing', { capability: 'task.dispatch' })).toEqual({ ok: false, key: 'refusal-unsupported' });
    expect(offersGive('n1')).toBe(false);
  });

  it('has a sentence for every refusal the node and the hub give, in both languages', () => {
    const cases: [string, Record<string, string>][] = [
      ['task.not_open', {}], ['task.blocked', {}], ['task.not_found', {}], ['mesh.recipient_unknown', {}], ['agent.queue_full', { limit: '8' }],
      ['agent.queue_full', { limit: '1' }], ['agent.not_found', {}], ['board.unavailable', {}], ['protocol.capability_missing', { capability: 'terminal.bracketed_paste' }],
      ['protocol.capability_missing', { capability: 'task.dispatch' }], ['task.dispatch_limited', { limit: '8' }], ['approval.grant_required', {}], ['node.offline', {}],
      ['auth.forbidden', {}], ['something.new', {}],
    ];
    const keys = new Set(cases.map(([code, params]) => giveRefusal(code, params).key));
    expect(keys.size).toBe(cases.length); // each its own words; only an unknown code gets the general sentence
    for (const lang of ['en', 'tr']) {
      const ids = new Set([...readFileSync(`../../locales/${lang}/web.ftl`, 'utf8').matchAll(/^([a-z][a-z0-9-]*) =/gm)].map((m) => m[1]));
      for (const k of [...keys, ...REFUSAL_KEYS]) expect(ids.has(k), `${lang} ${k}`).toBe(true);
    }
    setLanguage('en');
    expect(t('give-refused-limited', { limit: '8' })).toContain('8 tasks');
  });

  it('asks a limited device for a grant on that agent, not the machine', () => {
    expect(grantFor({ taskDispatch: { projectId: 'p', taskId: 't-1', agentId: 'a-1', byUser: '' } })).toEqual({ scope: 'agent:a-1', commands: ['task.dispatch'], kind: 'task' });
    for (const lang of ['en', 'tr']) expect(readFileSync(`../../locales/${lang}/web.ftl`, 'utf8')).toMatch(/^grant-title-task = .*\{ \$agent \}/m);
  });
});

describe('the command palette', () => {
  it('lists Backlog tasks of projects that have an agent to take them', () => {
    const boards = [board([task('1', 'open'), task('2', 'claimed'), task('3', 'done')]), { ...board([task('9', 'open')]), projectId: 'q', project: 'other' }];
    expect(givableTasks(boards, [agent('n1/a-1')]).map((x) => x.key)).toEqual(['p/1']);
    expect(givableTasks(boards, [agent('n1/a-1'), agent('n1/b', { projectId: 'q' })]).map((x) => x.key)).toEqual(['p/1', 'p/9']);
    expect(givableTasks(boards, [])).toEqual([]);
    expect(givableTasks([board(Array.from({ length: 50 }, (_, i) => task(String(i), 'open')))], [agent('n1/a-1')], undefined, 10)).toHaveLength(10);
  });
});

describe('review fixes', () => {
  const key = (el: Element, init: KeyboardEventInit) => { const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }); act(() => { el.dispatchEvent(e); }); return e; };

  it('ignores a held Enter and never puts the focus on "Give the task"', () => {
    const send = vi.fn(async () => ({ ok: true, queued: false }) as GiveResult);
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], send)} onClose={() => {}} />);
    const row = dialog().querySelector('[data-agent-id] button')!;
    // A repeat of the Enter that chose the palette entry: its default (clicking the focused row) is cancelled.
    expect(key(row, { key: 'Enter', repeat: true }).defaultPrevented).toBe(true);
    expect(key(row, { key: ' ', repeat: true }).defaultPrevented).toBe(true);
    // The keypress of a press that began before the dialog opened is ignored too (a browser clicks a button on Enter's keypress).
    const press = (el: Element) => { const e = new KeyboardEvent('keypress', { bubbles: true, cancelable: true, key: 'Enter', charCode: 13, keyCode: 13 }); act(() => { el.dispatchEvent(e); }); return e; };
    expect(press(row).defaultPrevented).toBe(true);
    expect(key(row, { key: 'Enter' }).defaultPrevented).toBe(false);
    expect(press(row).defaultPrevented).toBe(false); // a fresh press in the dialog works
    click(row);
    expect(document.activeElement?.textContent).toBe('Back');
    expect(key(button('Give the task', dialog())!, { key: 'Enter', repeat: true }).defaultPrevented).toBe(true);
    expect(send).not.toHaveBeenCalled();
  });

  it('keeps the dialog open while the task is being sent, and shows the answer', async () => {
    let answer!: (r: GiveResult) => void;
    const onClose = vi.fn();
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], () => new Promise((r) => { answer = r; }))} onClose={onClose} />);
    click(dialog().querySelector('[data-agent-id] button'));
    click(button('Give the task', dialog()));
    expect(dialog().getAttribute('aria-busy')).toBe('true');
    key(document.activeElement ?? document.body, { key: 'Escape' });
    key(document.body, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
    expect(dialog()).not.toBeNull();
    await act(async () => { answer({ ok: true, queued: true }); await Promise.resolve(); });
    expect(dialog().textContent).toContain('It waits until the agent\'s current turn ends.');
    // Once the answer is there, Escape closes as usual (the same key that was held back while sending).
    key(document.body, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('shows the answer of the command sent again after the owner allowed it, without offering another try meanwhile', async () => {
    let calls = 0;
    const command = vi.fn(async () => { calls++; if (calls === 1) throw { code: 'approval.grant_required', params: {} }; return { taskDispatch: { queued: false } }; });
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], (a, task) => giveTask(command, a, task))} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    click(button('Give the task', dialog()));
    await flush();
    expect(dialog().textContent).toContain('This machine needs your permission first.');
    expect(button('Choose another agent', dialog())).toBeUndefined();
    // The grant dialog's Allow sends the same command again (App.allowGrant -> resendGive).
    await act(async () => { await resendGive(command, 'n1', giveRequest(agent('n1/a-1'), theTask).body); await Promise.resolve(); });
    expect(command).toHaveBeenCalledTimes(2);
    expect(dialog().textContent).toContain('Sent to claude · a-1.');
  });

  it('forgets a machine that went offline, so an updated one is offered again', () => {
    noteRefusal('n1', 'protocol.capability_missing', { capability: 'task.dispatch' });
    forgetOffline(new Set(['n1', 'n2']));
    expect(offersGive('n1')).toBe(false);
    forgetOffline(new Set(['n2']));
    expect(offersGive('n1')).toBe(true);
    noteRefusal('n1', 'protocol.invalid_message', { field: 'command' });
    forgetGiveSupport(); // a new connection
    expect(offersGive('n1')).toBe(true);
  });

  it('keeps "not of this project" to giving a task; other commands keep their general sentence', () => {
    expect(refusalKey('mesh.recipient_unknown')).toBeUndefined();
    expect(giveRefusal('mesh.recipient_unknown')).toEqual({ key: 'give-refused-other-project' });
  });

  it('shows names as plain text in the title, the no-ask note, the rows and the answer', async () => {
    const name = 'claude · a\u202Eb\u0007c';
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1', { name, skipsPermissions: true, machine: 'lap\u202Etop' })])} onClose={() => {}} />);
    expect(dialog().textContent).not.toMatch(/[\u202E\u0007]/);
    click(dialog().querySelector('[data-agent-id] button'));
    expect(dialog().textContent).not.toMatch(/[\u202E\u0007]/);
    click(button('Give the task', dialog()));
    await flush();
    expect(dialog().textContent).toContain('Sent to claude · a\\u{202e}b\\u{7}c.');
  });
});

describe('cross-review fixes', () => {
  const freeSession = (mode: string): SessionModel => ({ modes: [{ id: 'ask', name: 'Ask', description: '', skipsPermissions: false }, { id: 'yolo', name: 'Yolo', description: '', skipsPermissions: true }], models: [], currentMode: mode, currentModel: '', contextTokens: 0, contextWindow: 0 });

  it('offers a running agent whose last turn is Finished (between turns), and not a closed or interrupted one', () => {
    const between = agent('n1/a-1', { state: 'finished', closed: false });
    expect(giveCandidates([between], 'p').map((x) => x.agent.id)).toEqual(['n1/a-1']);
    expect(giveCandidates([agent('n1/a-1', { state: 'finished', closed: true })], 'p')).toEqual([]);
    expect(giveCandidates([agent('n1/a-1', { interrupted: { resumable: true } })], 'p')).toEqual([]);
    render(<GiveContext.Provider value={api([between])}><BoardColumns board={board([task('1', 'open')])} /></GiveContext.Provider>);
    expect([...host.querySelectorAll('button')].some((b) => b.textContent === 'Give to an agent')).toBe(true);
    expect(givableTasks([board([task('1', 'open')])], [between]).map((x) => x.key)).toEqual(['p/1']);
  });

  it('shows "acts freely" for a mode that skips approvals, and follows a change while the confirmation is open', () => {
    const at = (mode: string) => api([agent('n1/a-1', { session: freeSession(mode) })]);
    render(<GiveDialog task={theTask} give={at('yolo')} onClose={() => {}} />);
    expect(dialog().querySelector('[data-agent-id] [data-no-ask="session"]')).not.toBeNull();
    click(dialog().querySelector('[data-agent-id] button'));
    expect(dialog().querySelector('[role=note]')?.textContent).toContain('acts without asking');
    // The agent is switched back to a mode that asks: the warning goes. Switched to skipping again: it comes back.
    render(<GiveDialog task={theTask} give={at('ask')} onClose={() => {}} />);
    expect(dialog().querySelector('[data-give-step="confirm"]')).not.toBeNull();
    expect(dialog().querySelector('[role=note]')).toBeNull();
    render(<GiveDialog task={theTask} give={at('yolo')} onClose={() => {}} />);
    expect(dialog().querySelector('[role=note]')?.textContent).toContain('acts without asking');
  });

  for (const [what, change] of [
    ['stopped', { closed: true, state: 'finished' }],
    ['moved to another project', { projectId: 'q' }],
    ['interrupted', { interrupted: { resumable: false } }],
  ] as [string, Partial<AgentTileModel>][]) {
    it(`does not send to an agent that ${what} while the confirmation was open, and says so`, () => {
      const send = vi.fn(async () => ({ ok: true, queued: false }) as GiveResult);
      render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], send)} onClose={() => {}} />);
      click(dialog().querySelector('[data-agent-id] button'));
      render(<GiveDialog task={theTask} give={api([agent('n1/a-1', change)], send)} onClose={() => {}} />);
      expect(button('Give the task', dialog())).toBeUndefined();
      expect(dialog().textContent).toContain('Nothing was sent; choose another agent.');
      expect(send).not.toHaveBeenCalled();
    });
  }
  it('does not send to an agent that is gone from the list', () => {
    const send = vi.fn(async () => ({ ok: true, queued: false }) as GiveResult);
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], send)} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    render(<GiveDialog task={theTask} give={api([], send)} onClose={() => {}} />);
    expect(dialog().textContent).toContain('Nothing was sent');
    expect(send).not.toHaveBeenCalled();
  });

  it('settles a dispatch waiting for permission when the grant dialog is dismissed, with another try offered', async () => {
    const command = vi.fn(async () => { throw { code: 'approval.grant_required', params: {} }; });
    render(<GiveDialog task={theTask} give={api([agent('n1/a-1')], (a, task) => giveTask(command, a, task))} onClose={() => {}} />);
    click(dialog().querySelector('[data-agent-id] button'));
    click(button('Give the task', dialog()));
    await flush();
    expect(button('Choose another agent', dialog())).toBeUndefined();
    // "Not now" / Escape in the grant dialog (App -> declineGive).
    await act(async () => { declineGive('n1', giveRequest(agent('n1/a-1'), theTask).body); await Promise.resolve(); });
    expect(dialog().textContent).toContain('nothing was sent. You can try again.');
    expect(button('Choose another agent', dialog())).toBeDefined();
    // The waiter was released: a later decline or resend for the same command reaches no dialog and changes nothing.
    declineGive('n1', giveRequest(agent('n1/a-1'), theTask).body);
    expect(command).toHaveBeenCalledTimes(1);
    for (const lang of ['en', 'tr']) {
      const src = readFileSync(`../../locales/${lang}/web.ftl`, 'utf8');
      for (const k of ['give-grant-declined', 'give-target-gone']) expect(src, `${lang} ${k}`).toMatch(new RegExp(`^${k} = `, 'm'));
    }
  });
});

// ADR-0095 step 3: "Start a new agent for this task" opens the ordinary start window for the task's project; once it started, the same
// confirmation follows for the new agent, and only then is the task sent.
describe('start a new agent for the task', () => {
  const projects = [
    { node: 'n1', projectId: 'p', label: 'laptop · shop', path: '/w/shop' },
    { node: 'n1', projectId: 'q', label: 'laptop · other', path: '/w/other' },
  ];
  const onlineMachines = [
    { id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 0 },
    { id: 'n2', name: 'server', os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 0 },
  ];
  const starter = (launch: StartForTask['launch']): StartForTask => ({
    projects, machines: onlineMachines,
    loadKinds: async () => [{ kind: 'claude', name: 'Claude Code', installed: true }, { kind: 'shell', name: 'Shell', installed: true }],
    inspect: async (_node, path) => ({ path, projectId: path === '/w/shop' ? 'p' : 'q', gitRoot: path, branch: 'main', portable: false }),
    launch,
  });
  const withStart = (agents: AgentTileModel[], launch: StartForTask['launch'], send: GiveApi['send'] = async () => ({ ok: true, queued: false })): GiveApi => ({ ...api(agents, send), start: starter(launch) });
  const startWindow = () => document.querySelector('[role=dialog][aria-label="Start an agent"]') as HTMLElement | null;
  const typeInto = (input: HTMLInputElement, value: string) => act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  /** Opens the start window from the picker and chooses the prefilled folder; returns the window. */
  const openAndChoose = async () => {
    click(dialog().querySelector('[data-give-start]'));
    await flush();
    const win = startWindow();
    expect(win).not.toBeNull();
    click(button('Use this folder', win!));
    await flush();
    return startWindow()!;
  };

  it('is the main action when no agent of the project can take the task, and a quieter entry under the list otherwise', () => {
    render(<GiveDialog task={theTask} give={withStart([], vi.fn())} onClose={() => {}} />);
    expect(dialog().textContent).toContain('No agent of shop is running on a connected machine that can take tasks. Start one for this task.');
    const main = dialog().querySelector('[data-give-start]') as HTMLButtonElement;
    expect(main.textContent).toBe('Start a new agent for this task');
    expect(main.className).toContain('kit-btn--primary');
    expect(main.className).toContain('min-h-11');
    act(() => root.unmount());
    root = createRoot(host);
    render(<GiveDialog task={theTask} give={withStart([agent('n1/a-1')], vi.fn())} onClose={() => {}} />);
    const entry = dialog().querySelector('[data-give-start]') as HTMLButtonElement;
    expect(entry.textContent).toBe('Start a new agent for this task');
    expect(entry.className).not.toContain('kit-btn--primary');
    expect(entry.className).toContain('min-h-11');
    // Under the list of agents, not instead of it.
    expect(dialog().querySelector('[data-agent-id]')).not.toBeNull();
    expect(entry.compareDocumentPosition(dialog().querySelector('ul')!) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
  });

  it('is not offered by a window that cannot start agents', () => {
    render(<GiveDialog task={theTask} give={api([])} onClose={() => {}} />);
    expect(dialog().querySelector('[data-give-start]')).toBeNull();
    expect(dialog().textContent).toContain('Start one first.');
  });

  it('puts "Give to an agent" on Backlog cards and in the palette when no agent runs but one can be started', () => {
    render(<GiveContext.Provider value={withStart([], vi.fn())}><BoardColumns board={board([task('1', 'open'), task('2', 'claimed')])} /></GiveContext.Provider>);
    expect([...host.querySelectorAll('button')].filter((b) => b.textContent === 'Give to an agent')).toHaveLength(1);
    expect(givableTasks([board([task('1', 'open'), task('2', 'done')])], [], undefined, 300, (id) => id === 'p').map((x) => x.key)).toEqual(['p/1']);
    expect(givableTasks([board([task('1', 'open')])], [], undefined, 300, () => false)).toEqual([]);
    expect(givableTasks([board([task('1', 'open')])], [])).toEqual([]);
  });

  it('opens the ordinary start window for the task\'s project: its folders only, agents only, the task named as text', async () => {
    const evil = { ...theTask, title: '<b>bold</b> ‮evil' };
    render(<GiveDialog task={evil} give={withStart([], vi.fn())} onClose={() => {}} />);
    click(dialog().querySelector('[data-give-start]'));
    await flush();
    const win = startWindow()!;
    expect(dialog()).toBeNull(); // the picker gave way to the start window
    const named = win.querySelector('[data-start-for-task]')!;
    expect(named.textContent).toBe('For the task “<b>bold</b> \\u{202e}evil” of shop');
    expect(named.querySelector('b')).toBeNull();
    // Prefilled as an ordinary start prefills: the machine with the project's folder, and that folder; the person still chooses it.
    expect((win.querySelector('input:not([type])') as HTMLInputElement).value).toBe('/w/shop');
    expect(win.textContent).not.toContain('/w/other');
    expect(button('Start', win)).toBeUndefined();
    click(button('Use this folder', win));
    await flush();
    // An agent card is named by the kind's name alone (its letter and its Terminal or Chat tag are not part of the name).
    const radios = [...startWindow()!.querySelectorAll('[role=radiogroup] [role=radio]')].map((r) => r.getAttribute('aria-label') ?? r.textContent);
    expect(radios).toContain('Claude Code');
    expect(radios).not.toContain('Shell'); // a plain terminal cannot take a task
  });

  it('confirms for the new agent once it started and shows up, then sends the task to it', async () => {
    const launch = vi.fn<StartForTask['launch']>(async () => ({ agentId: 'agent-9' }));
    const send = vi.fn<GiveApi['send']>(async () => ({ ok: true, queued: true }));
    render(<GiveDialog task={theTask} give={withStart([], launch, send)} onClose={() => {}} />);
    const win = await openAndChoose();
    click(button('Start', win));
    await flush();
    expect(launch).toHaveBeenCalledTimes(1);
    expect(launch.mock.calls[0]!.slice(0, 5)).toEqual(['n1', expect.objectContaining({ path: '/w/shop', projectId: 'p' }), 'claude', false, false]);
    // Started, not in the live view yet: it waits, and nothing is sent.
    expect(startWindow()).toBeNull();
    expect(dialog().querySelector('[data-give-step="waiting"]')).not.toBeNull();
    expect(send).not.toHaveBeenCalled();
    // It shows up (still starting): the same confirmation as for any agent, nothing sent before "Give the task".
    const fresh = agent('n1/agent-9', { state: 'starting' as AgentTileModel['state'], skipsPermissions: true });
    render(<GiveDialog task={theTask} give={withStart([fresh], launch, send)} onClose={() => {}} />);
    expect(dialog().querySelector('[data-give-step="confirm"]')).not.toBeNull();
    const text = dialog().textContent ?? '';
    for (const part of ['Give this task to claude · agent-9?', 'Write the release notes', 'laptop', 'shop', 'claude', 'acts without asking']) expect(text).toContain(part);
    expect(document.activeElement?.textContent).toBe('Back');
    expect(send).not.toHaveBeenCalled();
    click(button('Give the task', dialog()));
    await flush();
    expect(send).toHaveBeenCalledTimes(1);
    expect(send.mock.calls[0]![0].id).toBe('n1/agent-9');
    expect(giveRequest(send.mock.calls[0]![0], send.mock.calls[0]![1])).toEqual({ node: 'n1', body: { taskDispatch: { projectId: 'p', taskId: 't-1', agentId: 'agent-9', byUser: '' } } });
    expect(dialog().textContent).toContain('Sent to claude · agent-9. It waits until the agent is ready and its current turn ends.');
  });

  it('returns to the picker with the start\'s refusal when it failed, and sends nothing', async () => {
    const launch = vi.fn<StartForTask['launch']>(async () => ({ failure: 'start-limit' }));
    const send = vi.fn<GiveApi['send']>();
    render(<GiveDialog task={theTask} give={withStart([agent('n1/a-1')], launch, send)} onClose={() => {}} />);
    const win = await openAndChoose();
    click(button('Start', win));
    await flush();
    // The ordinary start window keeps the refusal, so the person can change something and try again.
    expect(startWindow()!.querySelector('[role=alert]')?.textContent).toBe(t('start-limit'));
    click(button('Cancel', startWindow()!));
    expect(startWindow()).toBeNull();
    expect(dialog().querySelector('[data-give-refusal]')?.textContent).toBe(t('start-limit'));
    expect(dialog().querySelector('[data-agent-id]')).not.toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it('returns to the picker without a refusal when the start window is cancelled, and sends nothing', async () => {
    const launch = vi.fn<StartForTask['launch']>();
    const send = vi.fn<GiveApi['send']>();
    render(<GiveDialog task={theTask} give={withStart([], launch, send)} onClose={() => {}} />);
    click(dialog().querySelector('[data-give-start]'));
    await flush();
    click(button('Cancel', startWindow()!));
    expect(dialog().querySelector('[data-give-start]')).not.toBeNull();
    expect(dialog().querySelector('[role=alert]')).toBeNull();
    expect(launch).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
  });

  it('keeps the task\'s project: no "shared project identity" in the start for a task, and the folder is inspected without making one', async () => {
    const inspect = vi.fn<StartForTask['inspect']>(async (_node, path) => ({ path, projectId: 'p', gitRoot: path, branch: 'main', portable: false }));
    const give = withStart([], vi.fn());
    give.start = { ...give.start!, inspect };
    render(<GiveDialog task={theTask} give={give} onClose={() => {}} />);
    click(dialog().querySelector('[data-give-start]'));
    await flush();
    expect(startWindow()!.textContent).not.toContain(t('start-portable'));
    expect(startWindow()!.querySelector('input[type=checkbox]')).toBeNull();
    click(button('Use this folder', startWindow()!));
    await flush();
    expect(inspect).toHaveBeenCalledWith('n1', '/w/shop', false);
    // The ordinary start keeps it.
    act(() => root.unmount());
    root = createRoot(host);
    const s = give.start!;
    render(<StartAgent projects={s.projects} machines={s.machines} loadKinds={s.loadKinds} inspect={inspect} onStart={async () => undefined} onClose={() => {}} />);
    await flush();
    expect(startWindow()!.textContent).toContain(t('start-portable'));
  });

  it('does not start in a folder of another project', async () => {
    const launch = vi.fn<StartForTask['launch']>();
    render(<GiveDialog task={theTask} give={withStart([], launch)} onClose={() => {}} />);
    click(dialog().querySelector('[data-give-start]'));
    await flush();
    typeInto(startWindow()!.querySelector('input:not([type])') as HTMLInputElement, '/w/other');
    click(button('Use this folder', startWindow()!));
    await flush();
    const win = startWindow()!;
    expect(win.querySelector('[role=alert]')?.textContent).toBe('This folder is not part of shop. The task can only be given to an agent working in its project: choose one of its folders.');
    expect(button('Start', win)?.disabled).toBe(true);
    expect(launch).not.toHaveBeenCalled();
  });

  it('says so when a machine started the agent without naming it, and sends nothing', async () => {
    const send = vi.fn<GiveApi['send']>();
    render(<GiveDialog task={theTask} give={withStart([], async () => ({ agentId: '' }), send)} onClose={() => {}} />);
    click(button('Start', await openAndChoose()));
    await flush();
    expect(dialog().querySelector('[data-give-refusal]')?.textContent).toBe(t('give-start-unnamed'));
    expect(send).not.toHaveBeenCalled();
  });

  for (const [what, change] of [['already stopped', { closed: true }], ['showed up in another project', { projectId: 'q' }]] as [string, Partial<AgentTileModel>][]) {
    it(`does not send to a new agent that ${what}`, async () => {
      const send = vi.fn<GiveApi['send']>();
      const launch: StartForTask['launch'] = async () => ({ agentId: 'agent-9' });
      render(<GiveDialog task={theTask} give={withStart([], launch, send)} onClose={() => {}} />);
      click(button('Start', await openAndChoose()));
      await flush();
      render(<GiveDialog task={theTask} give={withStart([agent('n1/agent-9', change)], launch, send)} onClose={() => {}} />);
      expect(dialog().textContent).toContain(t('give-target-gone'));
      expect(button('Give the task', dialog())).toBeUndefined();
      expect(send).not.toHaveBeenCalled();
    });
  }

  it('offers the start only where the started agent could take the task: not on a machine heard to lack tasks', async () => {
    const cards = () => [...host.querySelectorAll('button')].filter((b) => b.textContent === 'Give to an agent').length;
    render(<GiveContext.Provider value={withStart([], vi.fn())}><BoardColumns board={board([task('1', 'open')])} /></GiveContext.Provider>);
    expect(cards()).toBe(1);
    // The project's only folder is on n1, and n1 was heard not to take tasks: no start, no card action, no palette entry.
    act(() => noteRefusal('n1', 'protocol.capability_missing', { capability: 'task.dispatch' }));
    render(<GiveContext.Provider value={withStart([], vi.fn())}><BoardColumns board={board([task('1', 'open')])} /></GiveContext.Provider>);
    expect(cards()).toBe(0);
    act(() => root.unmount());
    root = createRoot(host);
    render(<GiveDialog task={theTask} give={withStart([], vi.fn())} onClose={() => {}} />);
    expect(dialog().querySelector('[data-give-start]')).toBeNull();
    expect(dialog().textContent).toContain(t('give-none-plain', { project: 'shop' }));
    // A folder of the project on n2 as well: the start is offered, and the start window does not offer n1.
    act(() => root.unmount());
    root = createRoot(host);
    const both = withStart([], vi.fn());
    both.start = { ...both.start!, projects: [...projects, { node: 'n2', projectId: 'p', label: 'server · shop', path: '/s/shop' }] };
    render(<GiveDialog task={theTask} give={both} onClose={() => {}} />);
    click(dialog().querySelector('[data-give-start]'));
    await flush();
    const machinesShown = [...startWindow()!.querySelectorAll('[role=radiogroup][aria-label="Machine"] [role=radio]')].map((r) => r.textContent);
    expect(machinesShown).toEqual(['server']);
    expect((startWindow()!.querySelector('input:not([type])') as HTMLInputElement).value).toBe('/s/shop');
  });

  it('says truthfully that a new agent\'s machine cannot take tasks', async () => {
    const send = vi.fn<GiveApi['send']>();
    const launch: StartForTask['launch'] = async () => ({ agentId: 'agent-9' });
    render(<GiveDialog task={theTask} give={withStart([], launch, send)} onClose={() => {}} />);
    click(button('Start', await openAndChoose()));
    await flush();
    act(() => noteRefusal('n1', 'protocol.invalid_message', { field: 'command' }));
    render(<GiveDialog task={theTask} give={withStart([agent('n1/agent-9')], launch, send)} onClose={() => {}} />);
    expect(dialog().textContent).toContain(t('give-machine-unsupported'));
    expect(dialog().textContent).not.toContain(t('give-target-gone'));
    expect(send).not.toHaveBeenCalled();
  });

  it('says when the new agent has not appeared after a while, and still follows it when it does', async () => {
    const send = vi.fn<GiveApi['send']>();
    const launch: StartForTask['launch'] = async () => ({ agentId: 'agent-9' });
    render(<GiveDialog task={theTask} give={withStart([], launch, send)} onClose={() => {}} />);
    const win = await openAndChoose();
    // Only the wait's own timer is faked (from before the waiting step starts it); promises run as usual.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      click(button('Start', win));
      await flush();
      expect(dialog().textContent).toContain(t('give-start-waiting'));
      act(() => { vi.advanceTimersByTime(APPEAR_WAIT_MS); });
      expect(dialog().textContent).toContain(t('give-start-late'));
      expect(button('Back', dialog())).toBeDefined();
    } finally { vi.useRealTimers(); }
    // Another agent with the same local id on another machine is not it: the binding is by machine and id.
    render(<GiveDialog task={theTask} give={withStart([agent('n2/agent-9')], launch, send)} onClose={() => {}} />);
    expect(dialog().querySelector('[data-give-step="waiting"]')).not.toBeNull();
    render(<GiveDialog task={theTask} give={withStart([agent('n2/agent-9'), agent('n1/agent-9')], launch, send)} onClose={() => {}} />);
    expect(dialog().textContent).toContain('Give this task to claude · agent-9?');
    expect(send).not.toHaveBeenCalled();
  });

  for (const lang of ['en', 'tr'] as const) {
    it(`has every new sentence in ${lang}, in the UI vocabulary`, () => {
      setLanguage(lang);
      const src = readFileSync(`../../locales/${lang}/web.ftl`, 'utf8');
      for (const key of ['give-start-new', 'give-none-start', 'give-none-plain', 'give-start-waiting', 'give-start-late', 'give-start-unnamed', 'give-machine-unsupported', 'give-queued-new', 'start-for-task', 'start-for-task-other-project']) {
        expect(src).toMatch(new RegExp(`^${key} = `, 'm'));
        expect(t(key, { project: 'shop', title: 'x', agent: 'a' })).not.toMatch(/claim|lease|mesh|dispatch|sahiplen/i);
      }
    });
  }
});

// Escape belongs to the topmost dialog: the board never closes under a dialog opened over it (the "Give to an agent" dialog listens for
// Escape a render after it appears, and an Escape in that moment closed the board instead).
describe('the board under another dialog', () => {
  it('ignores Escape while another dialog is open above it, and closes on Escape otherwise', () => {
    const onClose = vi.fn();
    render(<BoardView board={board([task('1', 'open')])} onClose={onClose} />);
    const above = document.createElement('div');
    above.setAttribute('role', 'dialog');
    above.setAttribute('data-state', 'open');
    document.body.append(above);
    act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })); });
    expect(onClose).not.toHaveBeenCalled();
    above.remove();
    act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })); });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('ADR-0123: the picker says each agent\'s model, and narrows to one provider or one model', () => {
    render(<GiveDialog task={theTask} give={api([
      agent('n1/a-1', { model: 'opus', effort: 'high', provider: 'anthropic' }),
      agent('n1/a-2', { model: 'gpt-6', provider: 'openai', kind: 'codex' }),
      agent('n2/b-1', { model: 'sonnet', provider: 'anthropic', parentId: 'n1/a-2' }),
    ])} onClose={() => {}} />);
    const rows = () => [...dialog().querySelectorAll('[data-agent-id]')].map((r) => r.getAttribute('data-agent-id'));
    expect(rows()).toHaveLength(3);
    expect(dialog().textContent).toContain('opus · high');
    const select = dialog().querySelector('select') as HTMLSelectElement;
    act(() => { select.value = 'p:anthropic'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(rows().sort()).toEqual(['n1/a-1', 'n2/b-1']);
    expect(dialog().querySelector('[data-agent-id="n2/b-1"]')?.className, 'a helper without its parent shown is not indented').toBe('');
    act(() => { select.value = 'm:gpt-6'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    expect(rows()).toEqual(['n1/a-2']);
  });
});
