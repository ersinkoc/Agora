import { describe, expect, it } from 'vitest';
import { agora } from '../gen/agora.js';
import { chatLine, isRich, mergeLines, queuedNote, queueState, type ChatLine } from './chat';
import { callMs, currentStep, outputTail, planProgress, planSteps } from './chatView';
import { filterCommands, insertText, slashQuery, stepIndex } from './slash';
import { actionOf, asksBefore, endedGroups, failedCalls, flagOf, ordered, stepsSummary, transcript } from './transcript';
import { applyEvent, commandsOf, configOf, type View } from './view';
import { fromSnapshot } from './view';

const V1 = agora.v1;
const L = (id: number, over: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: 1000 + id, role: 'agent', kind: 'text', text: `m${id}`, path: '', ...over });
const call = (id: number, over: Partial<ChatLine> = {}): ChatLine => L(id, { kind: 'tool', text: '{"tool":"Read","arguments":{"file_path":"a.rs"}}', callId: `c${id}`, ...over });

describe('reading the new entry fields', () => {
  it('reads a tool call with its kind, status, places, output and times', () => {
    const l = chatLine(V1.ChatEntry.create({
      entryId: 7, kind: V1.ChatKind.CHAT_KIND_TOOL_CALL, callId: 'c1', toolKind: V1.ToolKind.TOOL_KIND_EXECUTE, status: V1.ToolStatus.TOOL_STATUS_FAILED,
      paths: [{ path: 'a.rs', line: 3 }, { path: '' }], output: 'boom', outputTruncated: true, startedMs: 100, endedMs: 2600,
    }));
    expect(l).toMatchObject({ id: 7, kind: 'tool', callId: 'c1', callKind: 'run', status: 'failed', locations: [{ path: 'a.rs', line: 3 }], output: 'boom', outputCut: true, startedMs: 100, endedMs: 2600 });
    expect(callMs(l)).toBe(2500);
  });
  it('reads thinking, a plan with step status, a permission record and a queued prompt', () => {
    expect(chatLine(V1.ChatEntry.create({ kind: V1.ChatKind.CHAT_KIND_THOUGHT, text: 'hm' })).kind).toBe('thought');
    const plan = chatLine(V1.ChatEntry.create({ kind: V1.ChatKind.CHAT_KIND_PLAN, text: 'a\nb', planSteps: [{ text: 'a', status: V1.ToolStatus.TOOL_STATUS_COMPLETED, priority: V1.PlanPriority.PLAN_PRIORITY_HIGH }, { text: 'b', status: V1.ToolStatus.TOOL_STATUS_IN_PROGRESS }] }));
    expect(plan.steps).toEqual([{ text: 'a', status: 'completed', priority: 'high' }, { text: 'b', status: 'in_progress' }]);
    const p = chatLine(V1.ChatEntry.create({ kind: V1.ChatKind.CHAT_KIND_PERMISSION, text: 'run_command', approvalId: 'ap-1', permissionOutcome: V1.PermissionOutcome.PERMISSION_OUTCOME_DENIED }));
    expect(p).toMatchObject({ kind: 'permission', approvalId: 'ap-1', outcome: 'denied' });
    const q = chatLine(V1.ChatEntry.create({ role: V1.ChatRole.CHAT_ROLE_USER, kind: V1.ChatKind.CHAT_KIND_TEXT, queued: true }));
    expect(q.queued).toBe(true);
  });
  it('an entry from an old node reads exactly as before: none of the new fields is set', () => {
    const l = chatLine(V1.ChatEntry.create({ entryId: 3, atMs: 9, kind: V1.ChatKind.CHAT_KIND_TOOL_CALL, text: '{"tool":"x"}' }));
    expect(l).toEqual({ id: 3, atMs: 9, role: 'agent', kind: 'tool', text: '{"tool":"x"}', path: '' });
    expect(isRich([l])).toBe(false);
  });
  it('an unknown kind or status from a newer node reads as plain text or "not said"', () => {
    const l = chatLine(V1.ChatEntry.create({ kind: 99 as never, status: 99 as never, toolKind: 99 as never }));
    expect([l.kind, l.status, l.callKind]).toEqual(['text', undefined, undefined]);
  });
  it('knows a rich node by what it sent', () => {
    expect(isRich([call(1)])).toBe(true);
    expect(isRich([L(1, { kind: 'thought' })])).toBe(true);
    expect(isRich([L(1, { role: 'user', queued: true })])).toBe(true);
    expect(isRich([L(1)])).toBe(false);
  });
});

describe('an entry that changes in place', () => {
  it('replaces the held one in its place: one row, never two, and nothing else moves', () => {
    const held = [L(1), call(2, { status: 'in_progress' }), L(3)];
    const done = call(2, { status: 'completed', endedMs: 5 });
    const merged = mergeLines(held, [done]);
    expect(merged.map((l) => l.id)).toEqual([1, 2, 3]);
    expect(merged[1]).toBe(done);
    expect(merged[0]).toBe(held[0]);
    expect(merged[2]).toBe(held[2]);
  });
  it('keeps the very same array and objects for a repeat that says nothing new (no redraw)', () => {
    const held = [call(1, { status: 'completed' })];
    expect(mergeLines(held, [call(1, { status: 'completed' })])).toBe(held);
  });
  it('appends the new, places an older unseen id by its number, and takes an update of an old entry', () => {
    const held = [L(1), L(2), L(5)];
    expect(mergeLines(held, [L(3)]).map((l) => l.id)).toEqual([1, 2, 3, 5]); // older and not held: never dropped (ids restart with the node), placed by id
    expect(mergeLines(held, [L(6)]).map((l) => l.id)).toEqual([1, 2, 5, 6]);
    const upd = mergeLines(held, [L(2, { text: 'changed' })]);
    expect(upd.map((l) => l.text)).toEqual(['m1', 'changed', 'm5']);
  });
  it('replaces a plan with its newer steps and a queued prompt that was sent', () => {
    const a = L(1, { kind: 'plan', steps: [{ text: 'x', status: 'in_progress' }] });
    const b = L(1, { kind: 'plan', steps: [{ text: 'x', status: 'completed' }] });
    expect(mergeLines([a], [b])[0]).toBe(b);
    const q = L(2, { role: 'user', queued: true });
    expect(mergeLines([q], [L(2, { role: 'user' })])[0]?.queued).toBeUndefined();
  });
  it('a transcript has ONE group row for a call however often it is updated', () => {
    let held: ChatLine[] = [];
    for (const status of ['pending', 'in_progress', 'in_progress', 'completed'] as const) held = mergeLines(held, [call(1, { status })]);
    const items = transcript(held);
    expect(items.filter((i) => i.kind === 'steps').map((i) => (i.kind === 'steps' ? i.lines.length : 0))).toEqual([1]);
  });
});

describe('the transcript with the new kinds', () => {
  it('joins thought chunks, measures them to the next thing said, and draws a permission record as its own item', () => {
    const items = transcript([L(1, { kind: 'thought', atMs: 10_000, text: 'a ' }), L(2, { kind: 'thought', atMs: 11_000, text: 'b' }), L(3, { atMs: 19_000 }), L(4, { kind: 'permission', approvalId: 'x', outcome: 'pending' })]);
    expect(items.map((i) => i.kind)).toEqual(['date', 'thought', 'reply', 'permission']);
    const th = items[1];
    expect(th?.kind === 'thought' && [th.text, th.atMs, th.endMs]).toEqual(['a b', 10_000, 19_000]);
  });
  it('a call with the node\'s kind takes it over what the arguments suggest', () => {
    expect(actionOf(call(1, { callKind: 'search' }), 'read')).toBe('search');
    expect(actionOf(call(1, { callKind: 'other' }), 'read')).toBe('read');
    expect(actionOf(call(1), 'edit')).toBe('edit');
  });
  it('the span of a group is the node\'s own measure when the calls carry one', () => {
    const s = stepsSummary([call(1, { startedMs: 1000, endedMs: 2000, atMs: 0 }), call(2, { startedMs: 2000, endedMs: 5500, atMs: 0 })]);
    expect(s.spanMs).toBe(4500);
    // A call still running has no end yet: no span is claimed.
    expect(stepsSummary([call(1, { startedMs: 1000, endedMs: 2000 }), call(2, { startedMs: 2000 })]).spanMs).toBeUndefined();
  });
});

describe('a folded group is never hidden when something is wrong (ANY group, not only the newest)', () => {
  const calm = { state: 'working' };
  it('a failed call flags its group wherever it sits', () => {
    const lines = [call(1, { status: 'completed' }), call(2, { status: 'failed' })];
    expect(failedCalls(lines)).toBe(1);
    expect(flagOf(calm, false, lines)).toBe('call-failed');
    expect(flagOf(calm, true, lines)).toBe('call-failed');
  });
  it('a group right before a request that waits for you is flagged as needing you', () => {
    const lines = [call(1, { status: 'in_progress' })];
    const pending = transcript([L(2, { kind: 'permission', outcome: 'pending' })])[1];
    const answered = transcript([L(2, { kind: 'permission', outcome: 'approved' })])[1];
    expect(asksBefore(pending)).toBe(true);
    expect(asksBefore(answered)).toBe(false);
    expect(flagOf(calm, false, lines, pending)).toBe('needs-you');
    expect(flagOf(calm, false, lines, answered)).toBeUndefined();
  });
  it('the agent\'s own state still flags the newest group, as before, and a calm group has none', () => {
    expect(flagOf({ state: 'needs-you' }, true, [call(1, { status: 'completed' })])).toBe('needs-you');
    expect(flagOf({ state: 'needs-you' }, false, [call(1, { status: 'completed' })])).toBeUndefined();
    expect(flagOf({ state: 'finished', closed: true, exitCode: 1 }, true, [])).toBe('failed');
    expect(flagOf(calm, true, [call(1, { status: 'completed' })])).toBeUndefined();
  });
});

describe('the plan', () => {
  it('draws a node\'s steps with their status, and an old node\'s lines without', () => {
    expect(planSteps(L(1, { kind: 'plan', text: 'a\n\nb', steps: [{ text: 'a', status: 'completed' }, { text: 'b' }] }))).toEqual([{ text: 'a', status: 'completed' }, { text: 'b', status: undefined }]);
    expect(planSteps(L(1, { kind: 'plan', text: 'a\n\nb' }))).toEqual([{ text: 'a' }, { text: 'b' }]);
  });
  it('marks the step in hand, and none for a plan nothing has started or one that is done', () => {
    expect(currentStep(['completed', 'in_progress', 'pending'])).toBe(1);
    expect(currentStep(['completed', 'pending', 'pending'])).toBe(1);
    expect(currentStep(['pending', 'pending'])).toBeUndefined();
    expect(currentStep(['completed', 'completed'])).toBeUndefined();
    expect(currentStep([undefined, undefined])).toBeUndefined();
  });
  it('counts progress as before', () => expect(planProgress(['completed', 'in_progress', 'pending', 'pending'])).toEqual({ done: 1, total: 4, percent: 25 }));
});

describe('output and time of a call', () => {
  it('keeps the tail of a long output and says how much is left out', () => {
    const text = Array.from({ length: 30 }, (_, i) => `l${i + 1}`).join('\n');
    const t = outputTail(text, 12);
    expect(t.hidden).toBe(18);
    expect(t.text.split('\n')[0]).toBe('l19');
    expect(outputTail('a\nb\n', 12)).toEqual({ text: 'a\nb', hidden: 0 });
  });
  it('has a duration only once both ends were measured', () => {
    expect(callMs({ startedMs: 10, endedMs: 40 })).toBe(30);
    expect(callMs({ startedMs: 10 })).toBeUndefined();
    expect(callMs({ startedMs: 50, endedMs: 40 })).toBeUndefined();
  });
});

describe('prompts waiting behind a turn', () => {
  const q = (id: number) => L(id, { role: 'user', queued: true });
  it('lists the waiting ones and, after the dropped notice, marks them not sent instead of waiting for ever', () => {
    expect(queueState([q(1), q(2)]).waiting.map((l) => l.id)).toEqual([1, 2]);
    const s = queueState([q(1), q(2), L(3, { kind: 'notice', code: 'adapter.queue_dropped' }), q(4)]);
    expect(s.waiting.map((l) => l.id)).toEqual([4]);
    expect([...s.notSent]).toEqual([1, 2]);
  });
  it('a prompt that was sent (the entry sent again without the flag) no longer waits', () => {
    let held = mergeLines([], [q(1)]);
    held = mergeLines(held, [L(1, { role: 'user' })]);
    expect(queueState(held).waiting).toEqual([]);
  });
});

describe('the slash menu', () => {
  const cmds = [{ name: 'review', description: 'Review changes', inputHint: '' }, { name: 'compact', description: 'Free context', inputHint: 'what to keep' }, { name: '/init', description: 'Write a guide for review', inputHint: '' }];
  it('opens only while the first word starts with a slash', () => {
    expect(slashQuery('/', 1)).toBe('');
    expect(slashQuery('/rev', 4)).toBe('rev');
    expect(slashQuery('/rev', 2)).toBe('r'); // the caret inside the word
    expect(slashQuery('/review now', 11)).toBeUndefined(); // past the first word
    expect(slashQuery('hello /x', 8)).toBeUndefined();
    expect(slashQuery('', 0)).toBeUndefined();
    expect(slashQuery('/a/b', 4)).toBeUndefined();
  });
  it('lists names that start with it first, then ones that contain it, bounded', () => {
    expect(filterCommands(cmds, 'rev').map((c) => c.name)).toEqual(['review', '/init']);
    expect(filterCommands(cmds, '').map((c) => c.name)).toEqual(['review', 'compact', '/init']);
    expect(filterCommands(cmds, 'zzz')).toEqual([]);
    expect(filterCommands(cmds, '', 2)).toHaveLength(2);
  });
  it('puts only the command in the box, with one slash, whatever the agent wrote', () => {
    expect(insertText(cmds[0] as never)).toBe('/review ');
    expect(insertText(cmds[2] as never)).toBe('/init ');
  });
  it('wraps the highlight', () => {
    expect(stepIndex(0, -1, 3)).toBe(2);
    expect(stepIndex(2, 1, 3)).toBe(0);
    expect(stepIndex(0, 1, 0)).toBe(0);
  });
});

describe('an agent\'s settings and commands in the window\'s copy of the hub view', () => {
  const view = (): View => fromSnapshot(V1.Snapshot.create({ pos: 5, agents: [{ agentId: 'n1/a', nodeId: 'n1', running: true }] }));
  const ev = (body: Record<string, unknown>) => ({ producerId: 'n1', seq: 1, atMs: 1, ...body, body: Object.keys(body)[0] }) as never;
  const wire = (id: string, over: Record<string, unknown> = {}) => ({ id, name: id, category: 4, type: 1, current: 'high', choices: [{ id: 'low' }, { id: 'high', skipsPermissions: false }], ...over });

  it('reads options (category, type, choices, the node\'s skips flag) and commands, bounded', () => {
    const o = configOf([wire('effort'), wire('auto', { type: 2, current: 'true', choices: [], skipsPermissions: true }), { id: '' }]);
    expect(o.map((x) => [x.id, x.category, x.type, x.current, x.skipsPermissions])).toEqual([['effort', 'thought_level', 'select', 'high', false], ['auto', 'thought_level', 'boolean', 'true', true]]);
    expect(configOf(Array.from({ length: 100 }, (_, i) => wire(`o${i}`)))).toHaveLength(64);
    expect(commandsOf([{ name: 'a', description: 'd', inputHint: 'h' }, { name: '' }])).toEqual([{ name: 'a', description: 'd', inputHint: 'h' }]);
    expect(commandsOf(Array.from({ length: 150 }, (_, i) => ({ name: `c${i}` })))).toHaveLength(100);
  });
  it('applies the two events, replacing the last; the hub\'s replay at the snapshot position is taken, any other repeat is not', () => {
    const v = view();
    applyEvent(v, 5, ev({ agentSessionConfig: { agentId: 'a', options: [wire('effort')] } })); // at the snapshot's own position
    expect(v.agents.get('n1/a')?.session?.config?.[0]?.id).toBe('effort');
    applyEvent(v, 5, ev({ agentCommands: { agentId: 'a', commands: [{ name: 'review' }] } }));
    expect(v.agents.get('n1/a')?.session?.commands?.map((c) => c.name)).toEqual(['review']);
    applyEvent(v, 6, ev({ agentSessionConfig: { agentId: 'a', options: [wire('model')] } }));
    expect(v.agents.get('n1/a')?.session?.config?.map((c) => c.id)).toEqual(['model']);
    expect(v.pos).toBe(6);
    applyEvent(v, 4, ev({ agentSessionConfig: { agentId: 'a', options: [wire('old')] } })); // older: ignored
    expect(v.agents.get('n1/a')?.session?.config?.map((c) => c.id)).toEqual(['model']);
    // Any other event at the window's own position was already seen and is still ignored.
    applyEvent(v, 6, ev({ agentStateChanged: { agentId: 'a', state: 2 } }));
    expect(v.agents.get('n1/a')?.state).not.toBe(2);
  });
  it('is dropped when the agent exits, and never invents an agent it does not know', () => {
    const v = view();
    applyEvent(v, 6, ev({ agentCommands: { agentId: 'a', commands: [{ name: 'x' }] } }));
    applyEvent(v, 7, ev({ agentCommands: { agentId: 'ghost', commands: [{ name: 'x' }] } }));
    expect(v.agents.has('n1/ghost')).toBe(false);
    applyEvent(v, 8, ev({ agentExited: { agentId: 'a', exitCode: 0 } }));
    expect(v.agents.get('n1/a')?.session?.commands).toBeUndefined();
  });
  it('an old hub or node never sends them and the session stays as it was', () => {
    const v = view();
    expect(v.agents.get('n1/a')?.session).toBeUndefined();
  });
});

describe('the order a person reads, and turns that are over (#908)', () => {
  const ids = (lines: ChatLine[]) => ordered(lines).map((l) => l.id);
  it('places a sent prompt by its time, before the first other entry that is not older; ties go to the prompt', () => {
    // 3 and 4 waited behind the turn (smaller ids than the answers 5 and 6) and were sent at 20 and 22.
    const lines = [L(1, { role: 'user', atMs: 10 }), call(2, { atMs: 11 }), L(3, { role: 'user', atMs: 20 }), L(4, { role: 'user', atMs: 22 }), L(5, { atMs: 20 }), L(6, { atMs: 22 })];
    expect(ids(lines)).toEqual([1, 2, 3, 5, 4, 6]);
    const replies = transcript(lines).filter((i) => i.kind === 'reply');
    expect(replies).toHaveLength(2);
  });
  it('keeps the node\'s order when the times agree with it (every normal conversation), and for entries without a time', () => {
    const lines = [L(1, { role: 'user' }), L(2), L(3, { role: 'user' }), L(4), L(5)];
    expect(ids(lines)).toEqual([1, 2, 3, 4, 5]);
    expect(ids([L(1, { role: 'user', atMs: 0 }), L(2, { atMs: 0 }), L(3, { role: 'user', atMs: 0 }), L(4, { atMs: 0 })])).toEqual([1, 2, 3, 4]);
  });
  it('draws a waiting prompt after everything, and a prompt the node will not send where it was', () => {
    expect(ids([L(1, { role: 'user' }), L(2, { role: 'user', queued: true }), L(3), L(4)])).toEqual([1, 3, 4, 2]);
    const dropped = [L(1, { role: 'user' }), L(2, { role: 'user', queued: true }), L(3), L(4, { kind: 'notice', code: 'adapter.queue_dropped' })];
    expect(ids(dropped)).toEqual([1, 2, 3, 4]);
  });
  it('a group of calls is over when a later prompt went or the agent is in no turn; a waiting prompt starts no turn', () => {
    const items = transcript([L(1, { role: 'user' }), call(2), L(3, { role: 'user' }), call(4), L(5, { role: 'user', queued: true })]);
    const [first, second] = items.filter((i) => i.kind === 'steps').map((i) => i.id);
    expect([...endedGroups(items, true)]).toEqual([first]);
    expect(new Set(endedGroups(items, false))).toEqual(new Set([first, second]));
  });
  it('the composer\'s "waiting" note: while a prompt waits; before its entry arrived only while the agent works; never after it went', () => {
    expect(queuedNote('queued', 1, true, false)).toBe(true);
    expect(queuedNote('queued', 0, false, true)).toBe(true);
    expect(queuedNote('queued', 0, false, false)).toBe(false);
    expect(queuedNote('queued', 0, true, true)).toBe(false);
    expect(queuedNote('ok', 1, true, true)).toBe(false);
  });
});
