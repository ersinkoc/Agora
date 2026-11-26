import { afterEach, describe, expect, it, vi } from 'vitest';
import { EXIT_UNKNOWN, type AgentTileModel, type UiState } from '../model';
import { attentionList } from './fleet';
import { changes, finishedMarks, nextUnseen, type Seen } from './finished';
import type { ApprovalModel } from './view';

const agent = (id: string, state: UiState, more: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: 'var(--agent-amber)', machine: 'laptop', workspace: 'w', state, preview: '', ...more });
const approval = (id: string, agentId: string): ApprovalModel =>
  ({ id, agentId, agentName: agentId, action: 'run', detail: 'ls', node: 'm', localId: id, expiresAtMs: 0 } as ApprovalModel);
const look = (prev: Seen | undefined, agents: AgentTileModel[], approvals: ApprovalModel[] = [], pairs: { id: string; expiresMs: number }[] = []) =>
  changes(prev, agents, attentionList(approvals, agents, pairs as never));

afterEach(() => finishedMarks.set([]));

describe('what changed for the person', () => {
  it('takes the first look as it is: no sound and no mark for what was already there', () => {
    const c = look(undefined, [agent('a', 'idle'), agent('b', 'needs-you'), agent('c', 'finished')], [approval('r1', 'a')]);
    expect(c.events).toEqual([]);
    expect(c.finished).toEqual([]);
  });

  it('hears an agent finish when it goes from working to idle or finished, once', () => {
    let s = look(undefined, [agent('a', 'working'), agent('b', 'working')]).next;
    const c = look(s, [agent('a', 'idle'), agent('b', 'finished')]);
    expect(c.events).toEqual([{ kind: 'finished', agent: 'a' }, { kind: 'finished', agent: 'b' }]);
    expect(c.finished).toEqual(['a', 'b']);
    s = c.next;
    // The same state again is not a change.
    expect(look(s, [agent('a', 'idle'), agent('b', 'finished')]).events).toEqual([]);
  });

  it('does not call an agent that ended with an error finished, nor one that was not working', () => {
    const s = look(undefined, [agent('a', 'working'), agent('b', 'needs-you'), agent('c', 'unknown')]).next;
    const c = look(s, [agent('a', 'finished', { closed: true, exitCode: 1 }), agent('b', 'idle'), agent('c', 'idle')]);
    expect(c.events).toEqual([]);
    expect(c.finished).toEqual([]);
  });

  it('calls a closed agent finished only when its program exited by itself with 0, not when stopped or interrupted', () => {
    const ids = ['clean', 'stopped', 'signal', 'unknown', 'interrupted'];
    const s = look(undefined, ids.map((id) => agent(id, 'working'))).next;
    const c = look(s, [
      agent('clean', 'finished', { closed: true, exitCode: 0 }),
      agent('stopped', 'finished', { closed: true, exitCode: EXIT_UNKNOWN }), // Close agent, or a stop through the node
      agent('signal', 'finished', { closed: true, exitCode: -15 }), // the node's own SIGTERM
      agent('unknown', 'finished', { closed: true }),
      agent('interrupted', 'finished', { closed: true, exitCode: 0, interrupted: { resumable: true } }),
    ]);
    expect(c.finished).toEqual(['clean']);
    expect(c.events).toEqual([{ kind: 'finished', agent: 'clean' }]);
  });

  it('says which agents left the list', () => {
    const s = look(undefined, [agent('a', 'idle'), agent('b', 'idle')]).next;
    expect(look(s, [agent('b', 'idle')]).gone).toEqual(['a']);
  });

  it('does not count an agent seen for the first time later as a change', () => {
    const s = look(undefined, [agent('a', 'working')]).next;
    expect(look(s, [agent('a', 'working'), agent('new', 'idle')]).events).toEqual([]);
  });

  it('hears an agent come onto the Needs you list once, however many requests it opens', () => {
    let s = look(undefined, [agent('a', 'working')]).next;
    // Blocked in its terminal first...
    let c = look(s, [agent('a', 'needs-you')]);
    expect(c.events).toEqual([{ kind: 'needs-you', agent: 'a' }]);
    s = c.next;
    // ...then its request arrives, and a second one: still the same thing that needs you.
    c = look(s, [agent('a', 'needs-you')], [approval('r1', 'a')]);
    expect(c.events).toEqual([]);
    c = look(c.next, [agent('a', 'needs-you')], [approval('r1', 'a'), approval('r2', 'a')]);
    expect(c.events).toEqual([]);
    // Answered, at work, then asking again: a new change.
    c = look(c.next, [agent('a', 'working')]);
    expect(c.resumed).toEqual(['a']);
    c = look(c.next, [agent('a', 'working')], [approval('r3', 'a')]);
    expect(c.events).toEqual([{ kind: 'needs-you', agent: 'a' }]);
  });

  it('hears a request from an agent the list does not have yet, but not a machine asking to join', () => {
    const s = look(undefined, []).next;
    const c = look(s, [], [approval('r1', 'm/ghost')], [{ id: 'p1', expiresMs: 0 }]);
    expect(c.events).toEqual([{ kind: 'needs-you', agent: 'm/ghost' }]);
  });
});

describe('the finished mark', () => {
  it('is set for an agent that finished while the person was not looking at it', () => {
    expect([...nextUnseen(new Set(), { finished: ['a', 'b'], seen: new Set(['b']) })]).toEqual(['a']);
  });

  it('is cleared when the person looks at the agent', () => {
    const cur = new Set(['a', 'b']);
    expect([...nextUnseen(cur, { seen: new Set(['a']) })]).toEqual(['b']);
  });

  it('is cleared when the agent works again or waits on the person, and when it leaves the list', () => {
    const cur = new Set(['a', 'b', 'c']);
    expect([...nextUnseen(cur, { resumed: ['a'], gone: ['c'] })]).toEqual(['b']);
  });

  it('keeps the same set when nothing changed, so nothing redraws', () => {
    const cur = new Set(['a']);
    expect(nextUnseen(cur, { finished: ['a'], seen: new Set(['x']), gone: ['y'] })).toBe(cur);
  });

  it('is one store the tiles, rows and tabs read, and tells them when it changes', () => {
    const heard = vi.fn();
    const off = finishedMarks.subscribe(heard);
    finishedMarks.apply({ finished: ['a'] });
    expect(finishedMarks.has('a')).toBe(true);
    finishedMarks.apply({ finished: ['a'] });
    finishedMarks.apply({ seen: new Set(['a']) });
    expect(finishedMarks.has('a')).toBe(false);
    off();
    expect(heard).toHaveBeenCalledTimes(2);
  });
});
