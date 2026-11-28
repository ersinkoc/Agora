// @vitest-environment jsdom
// ADR-0131 in the window: the hub's marks and pickups on the board (the hub's word only, kept through a task's moves and the snapshot),
// the owner's settings on the board card (saved machines and kinds always offered, why it is paused), and the mark on an open card.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { BoardPickup, BoardPickupContext, PickupMark, type BoardPickupApi } from '../components/BoardPickup';
import { savePickup, type PickupSettings } from './pickup';
import { applyEvent, boards, emptyView, fromSnapshot } from './view';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const hub = (seq: number, body: Record<string, unknown>) => ({ producerId: 'hub', seq, ...body }) as never;
const task = (seq: number, owner = '') => hub(seq, { body: 'taskChanged', taskChanged: { taskId: 't1', projectId: 'p', title: 'Do it', claimedByAgentId: owner, done: false } });
const card = (v: ReturnType<typeof emptyView>) => boards(v).find((b) => b.projectId === 'p')?.tasks.find((t) => t.taskId === 't1');

describe('marks and pickups on the board', () => {
  it('come from the hub only and stay through the task\'s moves', () => {
    const v = emptyView();
    applyEvent(v, 1, task(1));
    applyEvent(v, 2, { producerId: 'n1', seq: 1, body: 'taskPickupEligible', taskPickupEligible: { projectId: 'p', taskId: 't1', eligible: true } } as never);
    expect(card(v)?.pickup).toBeUndefined();
    applyEvent(v, 3, hub(2, { body: 'taskPickupEligible', taskPickupEligible: { projectId: 'p', taskId: 't1', eligible: true } }));
    expect(card(v)?.pickup).toBe(true);
    applyEvent(v, 4, task(3, 'n2/a'));
    applyEvent(v, 5, hub(4, { body: 'taskPickedUp', taskPickedUp: { projectId: 'p', taskId: 't1', nodeId: 'n2', agentId: 'n2/a', runId: new Uint8Array(16), atMs: 9, rule: 'take_next' } }));
    expect(card(v)).toMatchObject({ pickup: true, pickedUp: { agent: 'n2/a', atMs: 9 }, status: 'claimed' });
    applyEvent(v, 6, task(5, ''));
    expect(card(v)?.pickedUp?.atMs, 'a later move keeps the record').toBe(9);
    applyEvent(v, 7, hub(6, { body: 'taskPickupEligible', taskPickupEligible: { projectId: 'p', taskId: 't1', eligible: false } }));
    expect(card(v)?.pickup).toBeUndefined();
    const s = fromSnapshot({ pos: 1, tasks: [{ taskId: 't1', projectId: 'p', title: 'x', pickupEligible: true, pickedUp: { agentId: 'n2/a', atMs: 3 } }] } as never);
    expect(card(s)).toMatchObject({ pickup: true, pickedUp: { agent: 'n2/a', atMs: 3 } });
  });
});

describe('the owner\'s pickup settings', () => {
  it('save only what the hub takes and read it back', async () => {
    const sent: unknown[] = [];
    const command = async (_n: string, body: Record<string, unknown>) => {
      sent.push(body);
      return body.taskPickupSettingsGet ? { taskPickupSettings: { enabled: true, nodeIds: ['n1'], agentKinds: [], active: true } } : {};
    };
    const got = await savePickup(command as never, 'p', { enabled: true, nodeIds: ['n1', 'n1', 'a b'], agentKinds: ['k'.repeat(65)] });
    expect(sent[0]).toEqual({ taskPickupSettingsSet: { projectId: 'p', enabled: true, nodeIds: ['n1'], agentKinds: [] } });
    expect(got).toEqual({ enabled: true, nodeIds: ['n1'], agentKinds: [], active: true, inactiveReason: '' });
  });

  it('offer the project\'s machines and kinds and the saved ones, say why it is paused, and save each change', async () => {
    const read: PickupSettings = { enabled: true, nodeIds: ['gone'], agentKinds: [], active: false, inactiveReason: 'not_shared' };
    const saved: unknown[] = [];
    const api: BoardPickupApi = {
      machines: () => [{ id: 'n1', name: 'laptop' }],
      kinds: () => ['codex'],
      read: async () => read,
      save: async (_p, next) => { saved.push(next); return { ...read, ...next, active: true, inactiveReason: '' }; },
    };
    act(() => root.render(<BoardPickupContext.Provider value={api}><BoardPickup projectId="p" /></BoardPickupContext.Provider>));
    await settle();
    const boxes = [...host.querySelectorAll<HTMLInputElement>('input[type=checkbox]')];
    expect(boxes.map((b) => b.parentElement?.textContent)).toEqual(['laptop', 'gone', 'codex']);
    expect(boxes.map((b) => b.checked)).toEqual([false, true, false]);
    expect(host.textContent).toContain('one machine');
    await act(async () => { boxes[2]!.click(); });
    await settle();
    expect(saved).toEqual([{ enabled: true, nodeIds: ['gone'], agentKinds: ['codex'] }]);
    expect(host.textContent).not.toContain('Paused');
    await act(async () => { host.querySelector<HTMLButtonElement>('[role=switch]')!.click(); });
    await settle();
    expect(saved[1]).toMatchObject({ enabled: false });
    expect(host.querySelectorAll('input[type=checkbox]')).toHaveLength(0);
  });

  it('cannot be changed without a live link, and shows nothing when the hub does not answer', async () => {
    const off: PickupSettings = { enabled: false, nodeIds: [], agentKinds: [], active: false, inactiveReason: '' };
    act(() => root.render(<BoardPickupContext.Provider value={{ machines: () => [], kinds: () => [], read: async () => off }}><BoardPickup projectId="p" /></BoardPickupContext.Provider>));
    await settle();
    expect(host.querySelector<HTMLButtonElement>('[role=switch]')?.disabled).toBe(true);
    act(() => root.render(<BoardPickupContext.Provider value={{ machines: () => [], kinds: () => [], read: () => Promise.reject(new Error('auth.forbidden')) }}><BoardPickup projectId="q" /></BoardPickupContext.Provider>));
    await settle();
    expect(host.querySelector('[data-board-pickup]')).toBeNull();
  });
});

describe('the mark on an open card', () => {
  it('offers and stops offering, and only says it without a live link', async () => {
    const marks: unknown[] = [];
    const api: BoardPickupApi = { machines: () => [], kinds: () => [], read: async () => ({ enabled: false, nodeIds: [], agentKinds: [], active: false, inactiveReason: '' }), mark: async (...a) => { marks.push(a); } };
    act(() => root.render(<BoardPickupContext.Provider value={api}><PickupMark projectId="p" taskId="t1" marked={false} /></BoardPickupContext.Provider>));
    await act(async () => { host.querySelector('button')!.click(); });
    expect(marks).toEqual([['p', 't1', true]]);
    act(() => root.render(<BoardPickupContext.Provider value={api}><PickupMark projectId="p" taskId="t1" marked /></BoardPickupContext.Provider>));
    expect(host.textContent).toContain('Offered to idle agents');
    await act(async () => { host.querySelector('button')!.click(); });
    expect(marks[1]).toEqual(['p', 't1', false]);
    act(() => root.render(<BoardPickupContext.Provider value={{ ...api, mark: undefined }}><PickupMark projectId="p" taskId="t1" marked /></BoardPickupContext.Provider>));
    expect(host.querySelector('button')).toBeNull();
    expect(host.textContent).toContain('Offered to idle agents');
  });
});
