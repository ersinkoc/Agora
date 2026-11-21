// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { forgetGiveSupport, givableTasks, giveAsk, giveCandidates, giveRefusal, giveRequest, giveTask, noteRefusal, offersGive, type GiveResult, type GiveTask } from '../net/give';
import { grantFor } from '../net/grant';
import { REFUSAL_KEYS } from '../net/refusal';
import type { BoardModel, BoardTaskModel } from '../net/view';
import { BoardColumns } from './BoardView';
import { GiveContext, GiveDialog, type GiveApi } from './GiveTask';

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
    // A hub that does not know the command at all: no machine is offered.
    act(() => noteRefusal('n2', 'protocol.invalid_message', { field: 'command' }));
    expect(cards(api([agent('n2/a-2')]))).toBe(0);
  });

  it('lists running agents of the task\'s project only, helpers under their parent', () => {
    const list = giveCandidates([
      agent('n1/helper', { parentId: 'n2/parent' }),
      agent('n2/parent'),
      agent('n1/other', { projectId: 'q' }),
      agent('n1/closed', { closed: true, state: 'finished' }),
      agent('n1/sh', { kind: 'shell' }),
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
    expect(rows[0]!.textContent).toContain('Acts freely');
    expect(rows[1]!.textContent).toContain('Helper · claude on server');
    expect(rows[1]!.textContent).not.toContain('Acts freely');
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
