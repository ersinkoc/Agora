import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { decodeFrame, encodeFrame, EVENT_KIND, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS } from './frame';
import { connectLive, type LinkState, type Socket } from './live';
import { HISTORY_PENDING_MAX, snapshotAfterSize, type TerminalEvent } from '../terminal/frames';
import { boards, costByProject, applyEvent, emptyView, fromSnapshot, machines, recentMessages, machinesOnline, projectChoices, tiles, type View } from './view';

const v1 = agora.v1;

function ev(producer: string, seq: number, body: Record<string, unknown>) {
  return v1.Event.create({ producerId: producer, seq, atMs: 1000 + seq, ...body });
}

describe('view', () => {
  it('folds events into tiles, keyed by producer', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('laptop', 1, { nodeOnline: { nodeId: 'laptop', hostname: 'Laptop' } }));
    applyEvent(v, 2, ev('laptop', 2, { worktreeCreated: { worktreeId: 'w1', checkoutId: 'c1', path: '/x/login-fix', branch: 'login-fix' } }));
    applyEvent(v, 3, ev('laptop', 3, { agentStarted: { agentId: 'a1', nodeId: 'laptop', worktreeId: 'w1', kind: 'claude' } }));
    applyEvent(v, 4, ev('other', 1, { agentStarted: { agentId: 'a1', nodeId: 'other', worktreeId: 'w9', kind: 'codex' } }));
    applyEvent(v, 5, ev('laptop', 4, { agentStateChanged: { agentId: 'a1', state: 1, reported: true } }));
    const t = tiles(v);
    expect(t.map((x) => x.id).sort()).toEqual(['laptop/a1', 'other/a1']); // same local id, two machines
    const a = t.find((x) => x.id === 'laptop/a1');
    expect(a).toMatchObject({ machine: 'Laptop', workspace: 'login-fix', state: 'working', reportedBy: 'agent' });
    expect(machinesOnline(v)).toBe(1);
  });

  it('names the workspace by project folder and hides machine-made branches', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { checkoutDiscovered: { checkoutId: 'c', nodeId: 'n', projectId: 'p', path: '/home/me/shop' } }));
    applyEvent(v, 2, ev('n', 2, { worktreeCreated: { worktreeId: 'w', checkoutId: 'c', path: '/x/agent-1', branch: 'agent/shell-1' } }));
    applyEvent(v, 3, ev('n', 3, { worktreeCreated: { worktreeId: 'w2', checkoutId: 'c', path: '/x/fix', branch: 'login-fix' } }));
    applyEvent(v, 4, ev('n', 4, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    applyEvent(v, 5, ev('n', 5, { agentStarted: { agentId: 'b', nodeId: 'n', worktreeId: 'w2', kind: 'shell' } }));
    const t = tiles(v);
    expect(t.find((x) => x.id === 'n/a')?.workspace).toBe('shop');
    expect(t.find((x) => x.id === 'n/a')?.project).toBe('shop');
    expect(t.find((x) => x.id === 'n/b')?.workspace).toBe('shop (login-fix)');
  });

  it('lists recent messages, newest first, and marks the ones that cross machines', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('a', 1, { agentStarted: { agentId: 'x', nodeId: 'a', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('a', 2, { messagePosted: { messageId: 'm1', fromAgentId: 'x', to: 'y', body: 'local' } }));
    applyEvent(v, 3, ev('a', 3, { messagePosted: { messageId: 'm2', fromAgentId: 'x', to: 'b/z', body: 'remote' } }));
    const m = recentMessages(v);
    expect(m.map((x) => x.body)).toEqual(['remote', 'local']);
    expect(m.map((x) => x.crossMachine)).toEqual([true, false]);
    expect(m[1]?.from).toBe('claude · ' + m[1]?.from.split(' · ')[1]);
    // The team chat draws the sender's avatar from its key, and names the other machine of a message between machines.
    expect(m.map((x) => x.fromId)).toEqual(['a/x', 'a/x']);
    expect(m.map((x) => x.toMachine)).toEqual(['b', undefined]);
  });

  it('lists a message between machines once, from its sender, marked, whichever copy comes first (#905)', () => {
    for (const senderFirst of [true, false]) {
      const v = emptyView();
      applyEvent(v, 1, ev('blackview', 1, { agentStarted: { agentId: 'agent-16', nodeId: 'blackview', worktreeId: 'w', kind: 'shell' } }));
      applyEvent(v, 2, ev('rm-two', 1, { agentStarted: { agentId: 'agent-1', nodeId: 'rm-two', worktreeId: 'w', kind: 'shell' } }));
      // The sender's node numbers it m-1 and names the recipient `<node>/<agent>`; the receiver's numbers it m-7 and names the sender so.
      const sent = ev('blackview', 2, { messagePosted: { messageId: 'm-1', fromAgentId: 'agent-16', to: 'rm-two/agent-1', body: 'ping' } });
      const got = ev('rm-two', 2, { messagePosted: { messageId: 'm-7', fromAgentId: 'blackview/agent-16', to: 'agent-1', body: 'ping' } });
      const [first, second] = senderFirst ? [sent, got] : [got, sent];
      applyEvent(v, 3, first);
      applyEvent(v, 4, second);
      expect(v.messages.map((m) => [m.from, m.to])).toEqual([['blackview/agent-16', 'rm-two/agent-1']]);
      const m = recentMessages(v);
      expect(m).toHaveLength(1);
      expect(m[0]?.crossMachine).toBe(true);
      expect(m[0]?.from).toMatch(/^shell · /);
      expect(m[0]?.to).toMatch(/^shell · /);
      expect(m[0]?.via).toBeUndefined(); // the sender's machine confirmed it, whichever came first
    }
  });

  it('says which machine alone names the sender of a copy nobody confirmed, so a node cannot speak for another (#905)', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('a', 1, { nodeOnline: { nodeId: 'a', hostname: 'Alpha' } }));
    applyEvent(v, 2, ev('b', 1, { nodeOnline: { nodeId: 'b', hostname: 'Bravo' } }));
    // b writes a message "from" a's agent that a never recorded.
    applyEvent(v, 3, ev('b', 2, { messagePosted: { messageId: 'm-9', fromAgentId: 'a/x', to: 'z', body: 'trust me' } }));
    // Named by its id, which the hub checked, not by the host name it reports.
    expect(recentMessages(v)[0]).toMatchObject({ from: 'a/x', to: 'b/z', via: 'b' });
    // Its own machine's copy of another message does not vouch for this one.
    applyEvent(v, 4, ev('a', 2, { messagePosted: { messageId: 'm-2', fromAgentId: 'x', to: 'b/z', body: 'hello' } }));
    expect(recentMessages(v).map((r) => r.via)).toEqual([undefined, 'b']);
  });

  it('never names the sender\'s machine as the reporter, even when another node calls itself by its host name (#905)', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('a', 1, { nodeOnline: { nodeId: 'a', hostname: 'laptop' } }));
    applyEvent(v, 2, ev('b', 1, { nodeOnline: { nodeId: 'b', hostname: 'laptop' } }));
    applyEvent(v, 3, ev('b', 2, { messagePosted: { messageId: 'm-9', fromAgentId: 'a/x', to: 'z', body: 'trust me' } }));
    expect(recentMessages(v)[0]?.via).toBe('b');
  });

  it('pairs a copy only with one the sender\'s machine or the hub recorded: two unconfirmed copies stay apart (#905)', () => {
    const v = emptyView();
    // b received a/x's message honestly; c writes the same "from a/x" itself.
    applyEvent(v, 1, ev('b', 1, { messagePosted: { messageId: 'm-4', fromAgentId: 'a/x', to: 'z', body: 'ping' } }));
    applyEvent(v, 2, ev('c', 1, { messagePosted: { messageId: 'm-1', fromAgentId: 'a/x', to: 'b/z', body: 'ping' } }));
    expect(recentMessages(v).map((r) => r.via)).toEqual(['c', 'b']);
    // a's own copy confirms the first unconfirmed one; the forged copy still says who reported it.
    applyEvent(v, 3, ev('a', 1, { messagePosted: { messageId: 'm-2', fromAgentId: 'x', to: 'b/z', body: 'ping' } }));
    const m = recentMessages(v);
    expect(m).toHaveLength(2);
    expect(m.map((r) => r.via)).toEqual(['c', undefined]);
  });

  it('folds a handoff the shared board recorded with the receiver\'s copy, the sender named once (#905)', () => {
    for (const boardFirst of [true, false]) {
      const v = emptyView();
      // The hub's board writes hub-wide ids; the receiving node gets the handoff's message from `a/x`.
      const board = ev('hub', 1, { messagePosted: { messageId: 'm-3', fromAgentId: 'a/x', to: 'b/y', body: 'take t-1' } });
      const got = ev('b', 1, { messagePosted: { messageId: 'm-5', fromAgentId: 'a/x', to: 'y', body: 'take t-1' } });
      const [first, second] = boardFirst ? [board, got] : [got, board];
      applyEvent(v, 1, first);
      applyEvent(v, 2, second);
      const m = recentMessages(v);
      expect(m.map((r) => [r.from, r.to, r.crossMachine, r.via])).toEqual([['a/x', 'b/y', true, undefined]]);
    }
  });

  it('keeps a message sent twice as two rows, and a channel copy that has no twin (#905)', () => {
    const v = emptyView();
    const sent = (pos: number, seq: number) => applyEvent(v, pos, ev('a', seq, { messagePosted: { messageId: `m-${seq}`, fromAgentId: 'x', to: 'b/z', body: 'ping' } }));
    const got = (pos: number, seq: number) => applyEvent(v, pos, ev('b', seq, { messagePosted: { messageId: `m-${seq}`, fromAgentId: 'a/x', to: 'z', body: 'ping' } }));
    sent(1, 1); sent(2, 2); got(3, 1); got(4, 2);
    expect(v.messages).toHaveLength(2);
    // A shared channel's copies are recorded by their receivers only, one per receiver: the board sends every other running agent
    // of the project a `MeshPost` with `origin` = the sender's machine, also to the agents on that same machine, whose node then
    // records `from` = `a/x` itself (deliver_remote). The sender's own node records nothing for a shared channel.
    applyEvent(v, 5, ev('b', 3, { messagePosted: { messageId: 'm-3', fromAgentId: 'a/x', to: 'z', body: '[#ops] hi' } }));
    applyEvent(v, 6, ev('a', 3, { messagePosted: { messageId: 'm-3', fromAgentId: 'a/x', to: 'y', body: '[#ops] hi' } }));
    // A channel of a project on one machine only stays on its broker: from the local id, to the channel, the text unprefixed.
    applyEvent(v, 7, ev('a', 4, { messagePosted: { messageId: 'm-4', fromAgentId: 'x', to: '#ops', body: 'hi' } }));
    expect(v.messages.slice(-3).map((m) => [m.from, m.to])).toEqual([['a/x', 'b/z'], ['a/x', 'a/y'], ['a/x', '#ops']]);
    expect(recentMessages(v).slice(0, 3).map((m) => m.crossMachine)).toEqual([false, false, true]);
    // Only b names the sender of its copy: it says so. a's copies name a's own agent, which a may.
    expect(recentMessages(v).slice(0, 3).map((m) => m.via)).toEqual([undefined, undefined, 'b']);
  });

  it('shows the shared board per project: tasks with their owner, leases with their holder, from the hub\'s events', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('alpha', 1, { checkoutDiscovered: { checkoutId: 'c', nodeId: 'alpha', projectId: 'p', path: '/home/me/shop' } }));
    applyEvent(v, 2, ev('alpha', 2, { worktreeCreated: { worktreeId: 'w', checkoutId: 'c', path: '/x', branch: 'b' } }));
    applyEvent(v, 3, ev('alpha', 3, { agentStarted: { agentId: 'a1', nodeId: 'alpha', worktreeId: 'w', kind: 'claude' } }));
    // The hub's events already carry `<node>/<agent>`.
    applyEvent(v, 4, ev('hub', 1, { taskChanged: { taskId: 't-1', projectId: 'p', title: 'port parser', claimedByAgentId: 'alpha/a1', done: false } }));
    applyEvent(v, 5, ev('hub', 2, { taskChanged: { taskId: 't-2', projectId: 'p', title: 'docs', claimedByAgentId: '', done: false } }));
    applyEvent(v, 6, ev('hub', 3, { leaseGranted: { agentId: 'alpha/a1', path: 'src/x.rs', expiresAtMs: 5000 } }));
    let b = boards(v);
    expect(b).toHaveLength(1);
    expect(b[0]?.project).toBe('shop');
    expect(b[0]?.tasks.map((t) => [t.title, t.status])).toEqual([['port parser', 'claimed'], ['docs', 'open']]);
    expect(b[0]?.tasks[0]?.owner).toMatch(/^claude/);
    expect(b[0]?.leases.map((l) => l.path)).toEqual(['src/x.rs']);
    applyEvent(v, 7, ev('hub', 4, { leaseReleased: { agentId: 'alpha/a1', path: 'src/x.rs' } }));
    applyEvent(v, 8, ev('hub', 5, { taskChanged: { taskId: 't-1', projectId: 'p', title: 'port parser', claimedByAgentId: 'alpha/a1', done: true } }));
    b = boards(v);
    expect(b[0]?.leases).toEqual([]);
    expect(b[0]?.tasks[0]?.status).toBe('done');
  });

  it('has the board right after a snapshot, before any new event', () => {
    const v = fromSnapshot(v1.Snapshot.create({
      checkouts: [{ checkoutId: 'c', nodeId: 'alpha', projectId: 'p', path: '/home/me/shop' }],
      worktrees: [{ worktreeId: 'w', checkoutId: 'c', path: '/x', branch: 'b' }],
      agents: [{ agentId: 'alpha/a1', nodeId: 'alpha', worktreeId: 'w', kind: 'claude', running: true }],
      tasks: [{ taskId: 't-1', projectId: 'p', title: 'port parser', claimedByAgentId: 'alpha/a1', done: false }],
      leases: [{ agentId: 'alpha/a1', path: 'src/x.rs', expiresAtMs: 9000 }],
    }));
    const b = boards(v);
    expect(b[0]?.project).toBe('shop');
    expect(b[0]?.tasks.map((t) => t.title)).toEqual(['port parser']);
    expect(b[0]?.leases.map((l) => l.path)).toEqual(['src/x.rs']);
  });

  it('forgets that an agent id had a terminal when the id is started again or its node restarts', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    applyEvent(v, 2, ev('n', 2, { terminalOpened: { terminalId: 't', agentId: 'a', nodeId: 'n' } }));
    expect(tiles(v)[0]?.terminal).toBe(true);
    // The id is reused by a chat agent: no terminal event follows.
    applyEvent(v, 3, ev('n', 3, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    expect(tiles(v)[0]?.terminal).toBe(false);
    applyEvent(v, 4, ev('n', 4, { terminalOpened: { terminalId: 't2', agentId: 'a', nodeId: 'n' } }));
    applyEvent(v, 5, ev('n', 5, { nodeOnline: { nodeId: 'n', hostname: 'N' } }));
    expect(v.withTerminal.size).toBe(0);
  });

  // The hub keeps a forgotten agent, hidden, and a node says its interrupted agents again on every start (ADR-0053, ADR-0060).
  it('a forgotten agent stays forgotten when its machine says again that it was interrupted', () => {
    const v = fromSnapshot(v1.Snapshot.create({
      pos: 10,
      agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'claude', state: 4, running: false, interrupted: true, resumable: true, interruptedAtMs: 500 }],
    }));
    const interrupted = { agentInterrupted: { agentId: 'a', kind: 'claude', resumable: true, interruptedAtMs: 500 } };
    applyEvent(v, 11, ev('hub', 1, { agentForgotten: { agentId: 'n/a' } }));
    applyEvent(v, 12, ev('n', 1, { nodeOnline: { nodeId: 'n', hostname: 'N' } }));
    applyEvent(v, 13, ev('n', 2, interrupted));
    expect(tiles(v)).toEqual([]);
    // One never forgotten is listed from the event; a new start of the forgotten id lists it again.
    applyEvent(v, 14, ev('n', 3, { agentInterrupted: { agentId: 'b', kind: 'codex' } }));
    applyEvent(v, 15, ev('n', 4, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    expect(tiles(v).map((t) => t.id).sort()).toEqual(['n/a', 'n/b']);
  });

  // As the hub's `remember_node`: a machine that connects again shows what it still runs, not an agent that stopped while hidden.
  it('a forgotten machine that connects again does not show an agent that stopped while it was hidden', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { nodeOnline: { nodeId: 'n', hostname: 'N' } }));
    applyEvent(v, 2, ev('n', 2, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 3, ev('n', 3, { agentStarted: { agentId: 'b', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 4, ev('hub', 1, { nodeForgotten: { nodeId: 'n' } }));
    applyEvent(v, 5, ev('n', 4, { agentExited: { agentId: 'a', exitCode: 0 } }));
    applyEvent(v, 6, ev('hub', 2, { nodeLink: { nodeId: 'n', up: true, generation: 1 } }));
    applyEvent(v, 7, ev('n', 5, { agentInterrupted: { agentId: 'a', kind: 'claude' } }));
    expect(tiles(v).map((t) => t.id)).toEqual(['n/b']);
    // One first heard of while the machine was hidden was never forgotten: the hub lists it when the machine comes back.
    applyEvent(v, 8, ev('hub', 3, { nodeForgotten: { nodeId: 'n' } }));
    applyEvent(v, 9, ev('n', 6, { agentInterrupted: { agentId: 'c', kind: 'codex' } }));
    applyEvent(v, 10, ev('hub', 4, { nodeLink: { nodeId: 'n', up: true, generation: 2 } }));
    expect(tiles(v).map((t) => t.id).sort()).toEqual(['n/b', 'n/c']);
  });

  it('adds up estimated cost per project and skips agents without a price', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { checkoutDiscovered: { checkoutId: 'c', nodeId: 'n', projectId: 'p', path: '/home/me/shop' } }));
    applyEvent(v, 2, ev('n', 2, { worktreeCreated: { worktreeId: 'w', checkoutId: 'c', path: '/x', branch: 'b' } }));
    for (const [i, id] of ['a', 'b', 'c'].entries()) applyEvent(v, 3 + i, ev('n', 3 + i, { agentStarted: { agentId: id, nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    expect(costByProject(v, new Map([['n/a', 1_000_000], ['n/b', 500_000]]))).toEqual([{ label: 'shop', microUsd: 1_500_000 }]);
    expect(costByProject(v, new Map())).toEqual([]);
  });

  it('adds the cost of an agent started in a folder to that folder\'s project (F1420)', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { workspaceRegistered: { workspaceId: 'directory:/home/me/api', nodeId: 'n', projectId: 'q', path: '/home/me/api', gitRoot: '/home/me/api' } }));
    applyEvent(v, 2, ev('n', 2, { agentStarted: { agentId: 'd', nodeId: 'n', worktreeId: '', kind: 'claude' } }));
    applyEvent(v, 3, ev('n', 3, { agentWorkspaceBound: { agentId: 'd', workspaceId: 'directory:/home/me/api' } }));
    expect(costByProject(v, new Map([['n/d', 2_000_000]]))).toEqual([{ label: 'api', microUsd: 2_000_000 }]);
  });

  it('drops the settings and commands of an agent that ended by interruption or by its machine\'s restart (F1421)', () => {
    for (const end of [{ agentInterrupted: { agentId: 'a', kind: 'claude', resumable: true } }, { nodeOnline: { nodeId: 'n', hostname: 'N' } }]) {
      const v = emptyView();
      applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', kind: 'claude' } }));
      applyEvent(v, 2, ev('n', 2, { agentSessionConfig: { agentId: 'a', options: [{ id: 'mode', name: 'Mode', category: 1, type: 1, current: 'ask', choices: [{ id: 'ask', name: 'Ask' }] }] } }));
      applyEvent(v, 3, ev('n', 3, { agentCommands: { agentId: 'a', commands: [{ name: 'compact' }] } }));
      applyEvent(v, 4, ev('n', 4, end));
      const s = tiles(v)[0]?.session;
      expect([s?.config, s?.commands]).toEqual([undefined, undefined]);
    }
  });

  it('carries each workspace\'s uncommitted file count onto its agent', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { worktreeCreated: { worktreeId: 'w', checkoutId: 'c', path: '/x', branch: 'b' } }));
    applyEvent(v, 2, ev('n', 2, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    expect(tiles(v)[0]?.filesChanged).toBe(0);
    applyEvent(v, 3, ev('n', 3, { gitStatusChanged: { worktreeId: 'w', filesChanged: 3 } }));
    expect(tiles(v)[0]).toMatchObject({ filesChanged: 3, branch: 'b' });
  });

  it('a node that starts again has lost its agents and waiting approvals', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { nodeOnline: { nodeId: 'n', hostname: 'N' } }));
    applyEvent(v, 2, ev('n', 2, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 3, ev('n', 3, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    expect(v.approvals.size).toBe(1);
    applyEvent(v, 4, ev('n', 4, { nodeOnline: { nodeId: 'n', hostname: 'N' } }));
    expect(v.approvals.size).toBe(0);
    expect(tiles(v)[0]?.state).toBe('finished');
  });

  it('knows which agents have a terminal and which are chat agents', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'sh', nodeId: 'n', worktreeId: 'w', kind: 'shell' } }));
    applyEvent(v, 2, ev('n', 2, { agentStarted: { agentId: 'chat', nodeId: 'n', worktreeId: 'w', kind: 'fake' } }));
    applyEvent(v, 3, ev('n', 3, { terminalOpened: { terminalId: 't-sh', nodeId: 'n', agentId: 'sh' } }));
    const t = tiles(v);
    expect(t.find((x) => x.id === 'n/sh')?.terminal).toBe(true);
    expect(t.find((x) => x.id === 'n/chat')?.terminal).toBe(false);
  });

  it('a waiting approval makes the agent need you, and answering it clears that', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('n', 2, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    expect(tiles(v)[0]?.state).toBe('needs-you');
    applyEvent(v, 3, ev('n', 3, { approvalResolved: { approvalId: 'p' } }));
    expect(tiles(v)[0]?.state).toBe('idle');
  });

  // #1150: a machine can record a request after its decision (a re-announcement that copied it first); its card never comes back.
  // Another machine's request with that id is its own, and the ids remembered are bounded per machine, oldest forgotten first.
  it('a request stated after its decision brings no card back', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('n', 2, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    applyEvent(v, 3, ev('n', 3, { approvalResolved: { approvalId: 'p' } }));
    applyEvent(v, 4, ev('n', 4, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    expect([...v.approvals.keys()]).toEqual([]);
    expect(tiles(v)[0]?.state).toBe('idle');
    applyEvent(v, 5, ev('m', 1, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    expect([...v.approvals.keys()]).toEqual(['m/p']);
    let pos = 6;
    for (let i = 0; i < 256; i++) applyEvent(v, pos++, ev('n', pos, { approvalResolved: { approvalId: `r-${i}` } }));
    applyEvent(v, pos++, ev('n', pos, { approvalResolved: { approvalId: 'r-0' } })); // said again: remembered once
    applyEvent(v, pos++, ev('n', pos, { approvalRequested: { approvalId: 'p', agentId: 'a', action: 'write' } }));
    expect(v.approvals.has('n/p')).toBe(true); // `p` was the oldest of 257
    applyEvent(v, pos++, ev('n', pos, { approvalRequested: { approvalId: 'r-0', agentId: 'a', action: 'write' } }));
    expect(v.approvals.has('n/r-0')).toBe(false);
    // Review of #1234: a forgotten machine's decisions are not kept for it; only the hub can forget one.
    applyEvent(v, pos++, ev('m', 2, { nodeForgotten: { nodeId: 'n' } }));
    expect(v.resolved?.has('n')).toBe(true);
    applyEvent(v, pos++, ev('hub', 1, { nodeForgotten: { nodeId: 'n' } }));
    expect(v.resolved?.has('n')).toBe(false);
  });

  it('ignores events at or before its position, so a replayed tail is harmless', () => {
    const v = emptyView();
    const e = ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } });
    applyEvent(v, 5, e);
    applyEvent(v, 5, ev('n', 2, { agentExited: { agentId: 'a', exitCode: 0 } }));
    expect(tiles(v)[0]?.state).toBe('idle');
  });

  it('lists machines, online first, and follows their resources', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('b', 1, { nodeOnline: { nodeId: 'b', hostname: 'Build box', os: 'linux' } }));
    applyEvent(v, 2, ev('a', 1, { nodeOnline: { nodeId: 'a', hostname: 'Alpha', os: 'macos' } }));
    applyEvent(v, 3, ev('hub', 1, { nodeOffline: { nodeId: 'a' } }));
    applyEvent(v, 4, ev('b', 2, { nodeResources: { nodeId: 'b', agentsRunning: 3 } }));
    expect(machines(v).map((m) => [m.name, m.online, m.agentsRunning])).toEqual([['Build box', true, 3], ['Alpha', false, 0]]);
  });

  // ADR-0070: the hub's NodeLink decides whether a machine is online, a reconnect resets nothing, and old facts still count.
  it('follows the hub\'s presence facts: down and up again by itself, with what the machine runs untouched', () => {
    const R = v1.NodeLinkReason;
    const v = emptyView();
    applyEvent(v, 1, ev('hub', 1, { nodeLink: { nodeId: 'n', up: true, reason: R.NODE_LINK_REASON_OPENED, generation: 1, lastHeardMs: 10 } }));
    applyEvent(v, 2, ev('n', 1, { nodeOnline: { nodeId: 'n', hostname: 'N' } }));
    applyEvent(v, 3, ev('n', 2, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 4, ev('hub', 2, { nodeOffline: { nodeId: 'n' } }));
    applyEvent(v, 5, ev('hub', 3, { nodeLink: { nodeId: 'n', up: false, reason: R.NODE_LINK_REASON_SILENT, generation: 1, lastHeardMs: 50_000 } }));
    expect(machines(v)[0]).toMatchObject({ online: false, lastSeenMs: 50_000 });
    applyEvent(v, 6, ev('hub', 4, { nodeLink: { nodeId: 'n', up: true, reason: R.NODE_LINK_REASON_OPENED, generation: 2, lastHeardMs: 90_000 } }));
    expect(machines(v)[0]?.online).toBe(true);
    expect(tiles(v)[0]?.closed).toBeFalsy();
  });

  it('ignores a late down from a replaced connection, and presence claimed by a node', () => {
    const R = v1.NodeLinkReason;
    const v = fromSnapshot(v1.Snapshot.create({ pos: 1, nodes: [{ nodeId: 'n', hostname: 'N', online: true }] }));
    applyEvent(v, 2, ev('hub', 1, { nodeLink: { nodeId: 'n', up: true, reason: R.NODE_LINK_REASON_OPENED, generation: 5 } }));
    applyEvent(v, 3, ev('hub', 2, { nodeLink: { nodeId: 'n', up: false, reason: R.NODE_LINK_REASON_CLOSED, generation: 4 } }));
    expect(machinesOnline(v)).toBe(1);
    applyEvent(v, 4, ev('hub', 3, { nodeLink: { nodeId: 'n', up: false, reason: R.NODE_LINK_REASON_REPLACED, generation: 5 } }));
    expect(machinesOnline(v)).toBe(1);
    // Right after a snapshot the generation is unknown: a replaced connection's end still changes nothing.
    const w = fromSnapshot(v1.Snapshot.create({ pos: 1, nodes: [{ nodeId: 'n', hostname: 'N', online: true }] }));
    applyEvent(w, 2, ev('hub', 1, { nodeLink: { nodeId: 'n', up: false, reason: R.NODE_LINK_REASON_REPLACED, generation: 1 } }));
    expect(machinesOnline(w)).toBe(1);
    applyEvent(w, 3, ev('evil', 1, { nodeLink: { nodeId: 'n', up: false, reason: R.NODE_LINK_REASON_CLOSED, generation: 9 } }));
    expect(machinesOnline(w)).toBe(1);
    applyEvent(w, 4, ev('hub', 2, { nodeLink: { nodeId: 'n', up: false, reason: R.NODE_LINK_REASON_CLOSED, generation: 1 } }));
    expect(machinesOnline(w)).toBe(0);
  });

  it('adds up reported token usage', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('n', 1, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }));
    applyEvent(v, 2, ev('n', 2, { agentUsageReported: { agentId: 'a', model: 'm', inputTokens: 100, outputTokens: 20 } }));
    applyEvent(v, 3, ev('n', 3, { agentUsageReported: { agentId: 'a', model: 'm', inputTokens: 5, outputTokens: 5 } }));
    expect(tiles(v)[0]?.tokens).toBe(130);
  });

  it('offers projects only on machines that are online', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('a', 1, { nodeOnline: { nodeId: 'a', hostname: 'Alpha' } }));
    applyEvent(v, 2, ev('a', 2, { checkoutDiscovered: { checkoutId: 'c1', nodeId: 'a', projectId: '4be2b2e8', path: '/home/me/webapp' } }));
    applyEvent(v, 3, ev('b', 1, { nodeOnline: { nodeId: 'b', hostname: 'Beta' } }));
    applyEvent(v, 4, ev('b', 2, { checkoutDiscovered: { checkoutId: 'c1', nodeId: 'b', projectId: 'api', path: '/a' } }));
    applyEvent(v, 5, ev('hub', 1, { nodeOffline: { nodeId: 'b' } }));
    expect(projectChoices(v)).toEqual([{ node: 'a', projectId: '4be2b2e8', label: 'Alpha · webapp', path: '/home/me/webapp' }]);
  });

  it('starts from a snapshot', () => {
    const v = fromSnapshot(v1.Snapshot.create({
      pos: 7,
      nodes: [{ nodeId: 'n', hostname: 'Box', online: true }],
      agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'codex', state: 2, running: true }],
    }));
    expect(v.pos).toBe(7);
    expect(tiles(v)[0]).toMatchObject({ machine: 'Box', state: 'needs-you' });
  });
});

class FakeSocket implements Socket {
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  closed = false;
  constructor(readonly url: string, readonly protocols: string[]) {}
  send(d: Uint8Array) { this.sent.push(d); }
  close() { this.closed = true; this.onclose?.(); }
  deliver(kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId: STREAM_EVENTS, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

describe('live connection', () => {
  it('subscribes with the token as a subprotocol, follows events, and resumes from its position', async () => {
    const sockets: FakeSocket[] = [];
    const states: LinkState[] = [];
    let latest: View | undefined;
    const live = connectLive({
      url: 'ws://hub/', token: 'tok',
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      backoffMs: () => 0,
      onState: (s) => states.push(s),
      onView: (v) => { latest = v; },
    });
    const s1 = sockets[0] as FakeSocket;
    expect(s1.protocols).toEqual(['agora.v1', 'agora.bearer.tok']);
    s1.onopen?.();
    const sub = decodeFrame(s1.sent[0] as Uint8Array);
    expect(sub?.kind).toBe(EVENT_KIND.SUBSCRIBE);
    expect(v1.Subscribe.decode(sub?.payload as Uint8Array).afterPos).toBeFalsy(); // no position yet: snapshot first

    s1.deliver(EVENT_KIND.SNAPSHOT, v1.Snapshot.encode({ pos: 3, nodes: [{ nodeId: 'n', hostname: 'Box', online: true }] }).finish());
    s1.deliver(EVENT_KIND.POSITIONED_EVENT, v1.PositionedEvent.encode({
      pos: 4, event: ev('n', 9, { agentStarted: { agentId: 'a', nodeId: 'n', worktreeId: 'w', kind: 'claude' } }),
    }).finish());
    expect(latest?.pos).toBe(4);
    expect(tiles(latest as View)).toHaveLength(1);
    expect(states.at(-1)).toBe('live');

    s1.onclose?.(); // the hub went away
    expect(states.at(-1)).toBe('offline');
    await new Promise((r) => setTimeout(r, 5));
    const s2 = sockets[1] as FakeSocket;
    s2.onopen?.();
    expect(v1.Subscribe.decode(decodeFrame(s2.sent[0] as Uint8Array)?.payload as Uint8Array).afterPos).toBeTruthy();
    live.close();
    expect(s2.closed).toBe(true);
  });

  it('closes on a malformed frame and starts over from a snapshot', async () => {
    const sockets: FakeSocket[] = [];
    connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
    });
    const s = sockets[0] as FakeSocket;
    s.onmessage?.({ data: new Uint8Array([0xff, 0xff]).buffer });
    expect(s.closed).toBe(true);
  });
});

describe('commands and terminals', () => {
  function setup(onGrantRequired?: (node: string, body: unknown) => void) {
    const sockets: FakeSocket[] = [];
    const live = connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {}, onGrantRequired,
    });
    const s = sockets[0] as FakeSocket;
    s.onopen?.();
    const sentFrames = () => s.sent.map((b) => decodeFrame(b)).filter((f) => f !== undefined);
    return { live, s, sentFrames };
  }
  const answer = (s: FakeSocket, requestId: string, body: Record<string, unknown>) => {
    const b = encodeFrame({ streamId: STREAM_COMMANDS, kind: 1, payload: v1.CommandResponse.encode(v1.CommandResponse.create({ requestId, ...body })).finish() });
    s.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  };

  it('sends a command with the target node and resolves with the answer or rejects with the code', async () => {
    const { live, s, sentFrames } = setup();
    const ok = live.command('laptop', { agentList: {} });
    const req = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    expect(req.targetNode).toBe('laptop');
    answer(s, req.requestId, { ok: { agentList: { agents: [] } } });
    expect((await ok).agentList).toBeTruthy();

    const bad = live.command('laptop', { agentList: {} });
    const req2 = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    answer(s, req2.requestId, { err: { code: 'node.offline' } });
    await expect(bad).rejects.toMatchObject({ code: 'node.offline' });
  });

  it('tells the window which command a machine refused until the owner grants it', async () => {
    const asked: Array<[string, unknown]> = [];
    const { live, s, sentFrames } = setup((node, body) => asked.push([node, body]));
    const p = live.command('laptop', { agentPrompt: { agentId: 'a-1', text: 'hi' } });
    const req = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    answer(s, req.requestId, { err: { code: 'approval.grant_required' } });
    await expect(p).rejects.toMatchObject({ code: 'approval.grant_required' });
    expect(asked).toHaveLength(1);
    expect(asked[0]?.[0]).toBe('laptop');
    expect(asked[0]?.[1]).toMatchObject({ agentPrompt: { agentId: 'a-1' } });
    // Other refusals are not asks.
    const q = live.command('laptop', { agentList: {} });
    const req2 = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    answer(s, req2.requestId, { err: { code: 'node.offline' } });
    await expect(q).rejects.toBeTruthy();
    expect(asked).toHaveLength(1);
  });

  it('relays a terminal on its own stream: subscribe, output, input as commands, close', () => {
    const { live, s, sentFrames } = setup();
    const events: string[] = [];
    const session = live.terminal('laptop', 't-agent-1').connect((e) => events.push(e.type));
    const sub = sentFrames().at(-1);
    expect(sub?.streamId).toBe(3);
    const req = v1.CommandRequest.decode(sub?.payload as Uint8Array);
    expect(req.targetNode).toBe('laptop');
    expect(req.terminalSubscribe?.terminalId).toBe('t-agent-1');

    const deliver = (kind: number, payload: Uint8Array) => {
      const b = encodeFrame({ streamId: 3, kind, payload });
      s.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
    };
    deliver(1, new TextEncoder().encode('screen'));
    deliver(0, new TextEncoder().encode('out'));
    expect(events).toEqual(['snapshot', 'data']);

    session.sendInput(new TextEncoder().encode('ls\n'));
    const input = v1.CommandRequest.decode(sentFrames().at(-1)?.payload as Uint8Array);
    expect(sentFrames().at(-1)?.streamId).toBe(STREAM_COMMANDS);
    expect(input.terminalInput?.terminalId).toBe('t-agent-1');

    session.close();
    const last = sentFrames().at(-1);
    expect(last).toMatchObject({ streamId: 3, kind: STREAM_CLOSE });
    live.close();
  });
});

describe('reconnects', () => {
  it('does not subscribe a terminal twice when a retry scheduled on the old connection comes due after the new one subscribed', async () => {
    const sockets: FakeSocket[] = [];
    const live = connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
    });
    const first = sockets[0] as FakeSocket;
    first.onopen?.();
    live.terminal('laptop', 't-1').connect(() => {});
    const subscribes = (s: FakeSocket) => s.sent.map((b) => decodeFrame(b)).filter((f) => f && f.streamId >= 3 && f.kind === 0).length;
    // The hub answers "not there"; a retry is scheduled on this connection.
    const req = v1.CommandRequest.decode(first.sent.map((b) => decodeFrame(b)).filter((f) => f && f.streamId >= 3).at(-1)?.payload as Uint8Array);
    const bad = encodeFrame({ streamId: STREAM_COMMANDS, kind: 1, payload: v1.CommandResponse.encode(v1.CommandResponse.create({ requestId: req.requestId, err: { code: 'node.offline' } })).finish() });
    first.onmessage?.({ data: bad.buffer.slice(bad.byteOffset, bad.byteOffset + bad.byteLength) });
    // The connection drops and comes back at once: the new one subscribes by itself.
    first.onclose?.();
    await new Promise((r) => setTimeout(r, 10));
    const second = sockets[1] as FakeSocket;
    second.onopen?.();
    expect(subscribes(second)).toBe(1);
    // The old retry fires (2 s) and must find itself out of date.
    await new Promise((r) => setTimeout(r, 2100));
    expect(subscribes(second)).toBe(1);
    live.close();
  }, 10000);
});

describe('refused sessions', () => {
  function run() {
    const sockets: FakeSocket[] = [];
    let rejected = 0;
    connectLive({
      url: 'ws://hub/', token: 'stale', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {}, onRejected: () => { rejected++; },
    });
    return { sockets, rejected: () => rejected };
  }
  it('reports a session the hub keeps closing without a word, but not an unreachable hub', async () => {
    const down = run();
    (down.sockets[0] as FakeSocket).onclose?.(); // never opened
    await new Promise((r) => setTimeout(r, 5));
    (down.sockets[1] as FakeSocket).onclose?.();
    await new Promise((r) => setTimeout(r, 5));
    expect(down.rejected()).toBe(0);

    const refused = run();
    for (let i = 0; i < 2; i++) {
      const s = refused.sockets[i] as FakeSocket;
      s.onopen?.();
      s.onclose?.();
      await new Promise((r) => setTimeout(r, 5));
    }
    expect(refused.rejected()).toBe(1);
    expect(refused.sockets).toHaveLength(2); // and it stops trying
  });
});

describe('shared terminal size', () => {
  // ADR-0106: once the connection has said the role (here an older hub's first frame, the snapshot), never before it.
  it('says the window\'s size again right behind every subscription, after a reconnect too', async () => {
    const sockets: FakeSocket[] = [];
    const live = connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
    });
    const first = sockets[0] as FakeSocket;
    first.onopen?.();
    first.deliver(EVENT_KIND.SNAPSHOT, v1.Snapshot.encode({ pos: 1 }).finish());
    const session = live.terminal('laptop', 't-1').connect(() => {});
    session.resize(120, 40);
    const commands = (s: FakeSocket) => s.sent.map((b) => decodeFrame(b)).filter((f) => f && f.kind === 0 && (f.streamId === STREAM_COMMANDS || f.streamId >= 3))
      .map((f) => v1.CommandRequest.decode(f?.payload as Uint8Array));
    const shape = (s: FakeSocket) => commands(s).map((c) => c.terminalSubscribe ? 'subscribe' : c.terminalResize ? `${c.terminalResize.cols}x${c.terminalResize.rows}` : '?');
    expect(shape(first)).toEqual(['subscribe', '120x40']);
    first.onclose?.();
    await new Promise((r) => setTimeout(r, 10));
    const second = sockets[1] as FakeSocket;
    second.onopen?.();
    expect(shape(second)).toEqual(['subscribe']);
    second.deliver(EVENT_KIND.SNAPSHOT, v1.Snapshot.encode({ pos: 1 }).finish());
    expect(shape(second)).toEqual(['subscribe', '120x40']);
    live.close();
  });

  it('unregisters wake listeners when the session is rejected', async () => {
    let unwakeCalls = 0;
    let rejected = false;
    const sockets: FakeSocket[] = [];
    connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      wake: () => () => { unwakeCalls++; },
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
      onRejected: () => { rejected = true; },
    });
    sockets[0]!.onopen?.();
    sockets[0]!.onclose?.();
    await new Promise((r) => setTimeout(r, 10));
    sockets[1]!.onopen?.();
    sockets[1]!.onclose?.();
    expect(rejected).toBe(true);
    expect(unwakeCalls).toBe(1);
  });
});

describe('terminal history (ADR-0100 decision 2)', () => {
  function open() {
    const sockets: FakeSocket[] = [];
    const live = connectLive({
      url: 'ws://hub/', token: 't', backoffMs: () => 0,
      socket: (u, p) => { const s = new FakeSocket(u, p); sockets.push(s); return s; },
      onState: () => {}, onView: () => {},
    });
    (sockets[0] as FakeSocket).onopen?.();
    const asked = (s: FakeSocket) => s.sent.map((b) => decodeFrame(b)).filter((f) => f && f.kind === 0 && f.streamId >= 3)
      .map((f) => v1.CommandRequest.decode(f?.payload as Uint8Array).terminalSubscribe?.historyLines ?? 0);
    const deliver = (s: FakeSocket, stream: number, kind: number, payload: Uint8Array) => {
      const b = encodeFrame({ streamId: stream, kind, payload });
      s.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
    };
    const reconnect = async () => {
      (sockets.at(-1) as FakeSocket).onclose?.();
      await new Promise((r) => setTimeout(r, 10));
      const next = sockets.at(-1) as FakeSocket;
      next.onopen?.();
      return next;
    };
    return { live, sockets, asked, deliver, reconnect };
  }

  it('asks for the window\'s own history when its terminal has none, and 0 once it painted a screen', async () => {
    const { live, sockets, asked, deliver, reconnect } = open();
    const first = sockets[0] as FakeSocket;
    const events: string[] = [];
    live.terminal('laptop', 't-1').connect((e) => events.push(e.type), { historyLines: 5000 });
    expect(asked(first)).toEqual([5000]);
    // A reconnect before any screen arrived: the terminal still has no history of its own, so it asks again.
    const second = await reconnect();
    expect(asked(second)).toEqual([5000]);
    deliver(second, 3, 4, new TextEncoder().encode('old lines'));
    deliver(second, 3, 1, new TextEncoder().encode('screen'));
    expect(events).toEqual(['snapshot']); // the history travels with its snapshot
    // From now on it keeps its own (longer) history: a reconnect, or the hub ending the relay (falling behind), asks for none.
    const third = await reconnect();
    expect(asked(third)).toEqual([0]);
    deliver(third, 3, STREAM_CLOSE, new Uint8Array());
    await new Promise((r) => setTimeout(r, 600));
    expect(asked(third)).toEqual([0, 0]);
    live.close();
  });

  // Codex review of #964: history from a subscription whose screen never came must not be painted with a later subscription's screen,
  // and retries that each lose their screen never pile up more than one replay.
  it('drops history whose screen never came when the connection is lost or the relay ends, and never holds more than one replay', async () => {
    const { live, sockets, asked, deliver, reconnect } = open();
    const painted: TerminalEvent[] = [];
    const ordered = snapshotAfterSize((e) => painted.push(e));
    live.terminal('laptop', 't-1').connect(ordered.push, { historyLines: 5000 });
    let s = sockets[0] as FakeSocket;
    // History arrives, then the connection is lost before its screen; three times, each with almost a whole replay.
    const half = new Uint8Array(HISTORY_PENDING_MAX / 2 - 1).fill(0x61);
    for (let i = 0; i < 3; i++) {
      expect(asked(s)).toEqual([5000]);
      deliver(s, 3, 4, half);
      deliver(s, 3, 4, half);
      s = await reconnect();
    }
    // The relay ends after history too.
    deliver(s, 3, 4, new TextEncoder().encode('stale'));
    deliver(s, 3, STREAM_CLOSE, new Uint8Array());
    await new Promise((r) => setTimeout(r, 600));
    // The next subscription sends only a screen and its size (the program went full screen meanwhile: no history).
    deliver(s, 3, 1, new TextEncoder().encode('screen'));
    deliver(s, 3, 2, new Uint8Array([80, 0, 24, 0]));
    expect(painted.map((e) => e.type)).toEqual(['resize', 'snapshot']);
    const snap = painted[1];
    expect(snap?.type === 'snapshot' && snap.history).toBeFalsy();
    ordered.dispose();
    live.close();
  });

  it('pairs a whole replay with its screen and drops one larger than a replay can be', () => {
    const { live, sockets, deliver } = open();
    const s = sockets[0] as FakeSocket;
    const got: TerminalEvent[] = [];
    live.terminal('laptop', 't-1').connect((e) => got.push(e), { historyLines: 5000 });
    deliver(s, 3, 4, new TextEncoder().encode('ab'));
    deliver(s, 3, 4, new TextEncoder().encode('c'));
    deliver(s, 3, 1, new TextEncoder().encode('screen'));
    expect(got.map((e) => e.type)).toEqual(['snapshot']);
    expect(new TextDecoder().decode(got[0]?.type === 'snapshot' ? got[0].history : undefined)).toBe('abc');
    live.terminal('laptop', 't-2').connect((e) => got.push(e), { historyLines: 5000 });
    deliver(s, 4, 4, new Uint8Array(HISTORY_PENDING_MAX).fill(0x61));
    deliver(s, 4, 4, new Uint8Array(1).fill(0x62));
    deliver(s, 4, 1, new TextEncoder().encode('screen'));
    const last = got.at(-1);
    expect(last?.type === 'snapshot' && last.history).toBeFalsy();
    live.close();
  });

  it('asks a new window (a remount) again, and none when the window asks for none', () => {
    const { live, sockets, asked, deliver } = open();
    const first = sockets[0] as FakeSocket;
    const source = live.terminal('laptop', 't-1');
    const a = source.connect(() => {}, { historyLines: 2000 });
    deliver(first, 3, 1, new TextEncoder().encode('screen'));
    a.close();
    source.connect(() => {}, { historyLines: 2000 });
    live.terminal('laptop', 't-2').connect(() => {});
    expect(asked(first)).toEqual([2000, 2000, 0]);
    live.close();
  });
});
