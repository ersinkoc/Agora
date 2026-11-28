// @vitest-environment jsdom
// ADR-0130 Amendment A in the window: the hub's hint, the project's existing channels, the owner's reads of each Review-enabled project's
// notices (on connecting and after each hint, one project at a time, an older answer never replacing a newer one, a refusal stopping
// further asks), the channel setting on the board card and the notices as Team chat rows.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { BoardNotices, BoardNoticesContext, type BoardNoticesApi } from '../components/BoardNotices';
import { merged, TeamChat } from '../components/TeamChat';
import { CommandError } from './live';
import { noticeRows, readNotices, useProjectNotices, type NoticeSettings, type ProjectNoticeModel } from './notices';
import { applyEvent, emptyView, projectChannels, type BoardModel, type MessageModel, type View } from './view';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const hint = (project: string, producerId = 'hub') => ({ producerId, body: 'projectNoticesChanged', projectNoticesChanged: { projectId: project } }) as never;
const wire = (id: string, project: string, createdMs: number, taskId = 't1') =>
  ({ noticeId: id, projectId: project, taskId, reportRevision: 1, acceptedMs: createdMs, channel: '#ops', createdMs });

describe('the hint and the channels in the view', () => {
  it('records only the hub\'s hint, at its position', () => {
    const v = emptyView();
    applyEvent(v, 1, hint('p'));
    applyEvent(v, 2, hint('q', 'n1'));
    expect([...(v.noticeHints ?? new Map())]).toEqual([['p', 1]]);
    applyEvent(v, 5, hint('p'));
    expect(v.noticeHints?.get('p')).toBe(5);
  });

  it('offers the channels the project\'s own agents wrote to, newest first, and nothing a third machine alone vouches for', () => {
    const v = emptyView();
    let pos = 0;
    const agent = (node: string, project: string) => {
      applyEvent(v, ++pos, { producerId: node, body: 'nodeOnline', nodeOnline: { nodeId: node, hostname: node } } as never);
      applyEvent(v, ++pos, { producerId: node, body: 'workspaceRegistered', workspaceRegistered: { workspaceId: 'w', nodeId: node, projectId: project, path: `/${project}` } } as never);
      applyEvent(v, ++pos, { producerId: node, body: 'agentWorkspaceBound', agentWorkspaceBound: { agentId: 'a', workspaceId: 'w' } } as never);
      applyEvent(v, ++pos, { producerId: node, body: 'agentStarted', agentStarted: { agentId: 'a', nodeId: node, projectId: project, kind: 'codex' } } as never);
    };
    agent('n1', 'p');
    agent('n2', 'other');
    const say = (producer: string, from: string, to: string) => applyEvent(v, ++pos, { producerId: producer, body: 'messagePosted', messagePosted: { messageId: String(pos), fromAgentId: from, to, body: 'x' } } as never);
    say('n1', 'a', '#ops');
    say('n2', 'a', '#elsewhere');
    say('n1', 'a', '#Bad Name');
    say('n1', 'a', '#build');
    say('n3', 'n1/a', '#forged');
    say('n1', 'a', '#ops');
    expect(projectChannels(v, 'p')).toEqual(['#ops', '#build']);
    expect(projectChannels(v, 'other')).toEqual(['#elsewhere']);
  });
});

describe('reading a project\'s notices', () => {
  it('follows the pages from the start and keeps only the project\'s own rows', async () => {
    const asked: unknown[] = [];
    const command = async (_n: string, body: Record<string, unknown>) => {
      const page = body.projectNoticesPage as { afterNoticeId: string };
      asked.push(page);
      if (!page.afterNoticeId) return { projectNoticesPage: { notices: [wire('a', 'p', 1), wire('x', 'p/b', 2)], more: true } };
      return { projectNoticesPage: { notices: [wire('b', 'p', 3)], more: false } };
    };
    const rows = await readNotices(command as never, 'p');
    expect(rows.map((r) => r.id)).toEqual(['a', 'b']);
    expect(asked).toEqual([
      { projectId: 'p', afterCreatedMs: 0, afterNoticeId: '', limit: 50 },
      { projectId: 'p', afterCreatedMs: 1, afterNoticeId: 'a', limit: 50 },
    ]);
  });
});

/** A hub that answers pages from `rows`; `hold` keeps answers until released, in order. */
function hub(rows: Map<string, ReturnType<typeof wire>[]>) {
  const sent: string[] = [];
  const held: (() => void)[] = [];
  let hold = false;
  let refuse = false;
  const command = (_n: string, body: Record<string, unknown>) => {
    const name = Object.keys(body)[0] ?? '';
    const project = (body[name] as { projectId: string }).projectId;
    sent.push(`${name}:${project}`);
    if (refuse) return Promise.reject(new CommandError('auth.forbidden'));
    const answer = { projectNoticesPage: { notices: [...(rows.get(project) ?? [])], more: false } };
    if (!hold) return Promise.resolve(answer);
    return new Promise((resolve) => held.push(() => resolve(answer)));
  };
  return { command, sent, held, setHold: (h: boolean) => { hold = h; }, setRefuse: (r: boolean) => { refuse = r; } };
}

function probe(command: unknown, owner = true) {
  let out: ProjectNoticeModel[] = [];
  const Probe = ({ view, connected }: { view: View; connected: boolean }) => { out = useProjectNotices(command as never, view, owner, connected); return null; };
  return { Probe, rows: () => out.map((n) => n.id) };
}

const reviewed = (...projects: string[]) => { const v = emptyView(); for (const p of projects) v.reviewProjects.add(p); return v; };
const withHint = (v: View, project: string, pos: number) => { const next = { ...v, noticeHints: new Map(v.noticeHints ?? []) }; next.noticeHints.set(project, pos); return next; };

describe('the owner\'s window reads the notices', () => {
  it('reads every Review-enabled project on connecting and a project again after its hint', async () => {
    const rows = new Map([['p', [wire('a', 'p', 1)]], ['q', [wire('c', 'q', 2)]]]);
    const h = hub(rows);
    const { Probe, rows: got } = probe(h.command);
    let v = reviewed('p', 'q');
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    expect(h.sent).toEqual(['projectNoticesPage:p', 'projectNoticesPage:q']);
    expect(got().sort()).toEqual(['a', 'c']);
    // The hub pruned `a` and kept `b`: the re-read replaces the project's rows.
    rows.set('p', [wire('b', 'p', 3)]);
    v = withHint(v, 'p', 9);
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    expect(h.sent.slice(2)).toEqual(['projectNoticesPage:p']);
    expect(got().sort()).toEqual(['b', 'c']);
    // The link comes back: everything is read again.
    act(() => root.render(<Probe view={v} connected={false} />));
    expect(got()).toEqual([]);
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    expect(h.sent.slice(3).sort()).toEqual(['projectNoticesPage:p', 'projectNoticesPage:q']);
  });

  it('never lets an older answer replace a newer one', async () => {
    const rows = new Map([['p', [wire('old', 'p', 1)]]]);
    const h = hub(rows);
    h.setHold(true);
    const { Probe, rows: got } = probe(h.command);
    let v = reviewed('p');
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    v = withHint(v, 'p', 4);
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    // The first read's answer comes back after the hint: it is dropped, and the second read (with the hub's newer rows) is sent.
    rows.set('p', [wire('new', 'p', 2)]);
    h.held.shift()?.();
    await settle();
    expect(got()).toEqual([]);
    h.held.shift()?.();
    await settle();
    expect(got()).toEqual(['new']);
  });

  it('stops asking after a refusal, and a member\'s window never asks', async () => {
    const h = hub(new Map());
    h.setRefuse(true);
    const { Probe } = probe(h.command);
    let v = reviewed('p', 'q');
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    expect(h.sent).toEqual(['projectNoticesPage:p']);
    v = withHint(v, 'q', 3);
    act(() => root.render(<Probe view={v} connected />));
    await settle();
    expect(h.sent).toEqual(['projectNoticesPage:p']);

    const member = hub(new Map());
    const m = probe(member.command, false);
    act(() => root.render(<m.Probe view={reviewed('p')} connected />));
    await settle();
    expect(member.sent).toEqual([]);
  });
});

const board = (projectId: string, tasks: { taskId: string; title: string }[]): BoardModel =>
  ({ projectId, project: `name-${projectId}`, reviewEnabled: true, folders: [], leases: [], tasks: tasks.map((t) => ({ id: `${projectId}/${t.taskId}`, taskId: t.taskId, title: t.title, status: 'done', owner: '' })) });

describe('notices in Team chat', () => {
  it('are the newest few, with the board\'s title, among the messages by time, without anyone\'s name', () => {
    const notices: ProjectNoticeModel[] = [
      { id: 'n1', projectId: 'p', taskId: 't1', revision: 1, acceptedMs: 1000, channel: '#ops', createdMs: 1000 },
      { id: 'n2', projectId: 'p', taskId: 'gone', revision: 1, acceptedMs: 3000, channel: '#ops', createdMs: 3000 },
    ];
    const rows = noticeRows(notices, [board('p', [{ taskId: 't1', title: 'Fix‮ the build' }])]);
    expect(rows).toEqual([
      { id: 'n2', channel: '#ops', project: 'name-p', atMs: 3000 },
      { id: 'n1', channel: '#ops', project: 'name-p', atMs: 1000, title: 'Fix‮ the build' },
    ]);
    const message = (atMs: number): MessageModel => ({ from: 'a', to: 'b', body: 'hi', atMs, crossMachine: false });
    expect(merged([message(2000), message(500)], rows).map((r) => r.notice?.id ?? r.m?.atMs)).toEqual(['n2', 2000, 'n1', 500]);
    act(() => root.render(<TeamChat messages={[message(2000)]} notices={rows} now={4000} />));
    const shown = [...host.querySelectorAll('[data-notice-id]')].map((e) => e.textContent ?? '');
    expect(shown).toHaveLength(2);
    // As on the board: a hidden character in the task's text is shown as a mark, never applied.
    expect(shown[1]).toContain('the build');
    expect(shown[1]).not.toContain('\u202e');
    expect(shown[0]).toContain('#ops');
    expect(host.querySelectorAll('[data-notice-id] button, button[data-notice-id]')).toHaveLength(0);
  });
});

describe('the channel setting on the board card', () => {
  const read: NoticeSettings = { dispatcher: true, channel: '#old', active: false, inactiveReason: 'not_shared' };
  it('keeps the saved channel offered, says why it is paused, and saves keeping the other setting', async () => {
    const saved: unknown[] = [];
    const api: BoardNoticesApi = {
      channels: () => ['#ops'],
      read: async () => read,
      save: async (_p, r, channel) => { saved.push([r.dispatcher, channel]); return { ...r, channel, active: true, inactiveReason: '' }; },
    };
    act(() => root.render(<BoardNoticesContext.Provider value={api}><BoardNotices projectId="p" /></BoardNoticesContext.Provider>));
    await settle();
    const select = host.querySelector('select')!;
    expect([...select.options].map((o) => o.value)).toEqual(['', '#old', '#ops']);
    expect(select.value).toBe('#old');
    expect(host.textContent).toContain('one machine');
    await act(async () => { select.value = '#ops'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    await settle();
    expect(saved).toEqual([[true, '#ops']]);
    expect(host.textContent).not.toContain('Paused');
  });

  it('cannot be changed while the link is down, and shows nothing when the hub does not answer', async () => {
    act(() => root.render(<BoardNoticesContext.Provider value={{ channels: () => [], read: async () => ({ ...read, channel: '' }) }}><BoardNotices projectId="p" /></BoardNoticesContext.Provider>));
    await settle();
    expect(host.querySelector('select')?.disabled).toBe(true);
    expect(host.textContent).toContain('No channel yet');
    act(() => root.render(<BoardNoticesContext.Provider value={{ channels: () => [], read: () => Promise.reject(new CommandError('auth.forbidden')) }}><BoardNotices projectId="q" /></BoardNoticesContext.Provider>));
    await settle();
    expect(host.querySelector('[data-board-notices]')).toBeNull();
  });
});
