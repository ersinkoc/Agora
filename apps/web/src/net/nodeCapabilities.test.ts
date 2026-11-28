// ADR-0132: what the hub publishes of each machine's current link reaches the window, only for that link, and a machine known not to
// speak `task.dispatch` is not offered tasks, while one not known is offered as before.
import { afterEach, describe, expect, it } from 'vitest';
import { applyEvent, emptyView, fromSnapshot, machines } from './view';
import { cannotTakeKey, forgetGiveSupport, noteCapabilities, offersGive } from './give';

afterEach(() => forgetGiveSupport());

const link = (seq: number, up: boolean, generation: number) =>
  ({ producerId: 'hub', seq, body: 'nodeLink', nodeLink: { nodeId: 'n1', up, generation, reason: up ? 1 : 3 } }) as never;
const caps = (seq: number, generation: number, names?: string[]) =>
  ({ producerId: 'hub', seq, body: 'nodeCapabilitiesChanged', nodeCapabilitiesChanged: { nodeId: 'n1', generation, capabilities: names ? { names } : undefined } }) as never;
const shown = (v: ReturnType<typeof emptyView>) => machines(v).find((m) => m.id === 'n1')?.capabilities;

describe('published node capabilities', () => {
  it('belong to the current link only', () => {
    const v = emptyView();
    applyEvent(v, 1, link(1, true, 1));
    expect(shown(v)).toBeUndefined();
    applyEvent(v, 2, caps(2, 1, ['task.dispatch']));
    expect(shown(v)).toEqual(['task.dispatch']);
    // A node cannot say it for itself.
    applyEvent(v, 3, { ...(caps(3, 1, []) as object), producerId: 'n1' } as never);
    expect(shown(v)).toEqual(['task.dispatch']);
    applyEvent(v, 4, link(4, true, 2));
    expect(shown(v)).toBeUndefined();
    applyEvent(v, 5, caps(5, 1, ['task.dispatch']));
    expect(shown(v), 'an older link').toBeUndefined();
    applyEvent(v, 6, caps(6, 2, []));
    expect(shown(v), 'known and empty').toEqual([]);
    applyEvent(v, 7, link(7, false, 2));
    expect(shown(v)).toBeUndefined();
    const s = fromSnapshot({ nodes: [{ nodeId: 'n1', hostname: 'a', online: true, capabilities: { names: ['task.dispatch'] } }, { nodeId: 'n2', online: true }] } as never);
    expect(machines(s).map((m) => m.capabilities)).toEqual([['task.dispatch'], undefined]);
  });

  it('decide whether a machine is offered tasks, before any answer', () => {
    noteCapabilities([{ id: 'new', capabilities: ['task.dispatch'] }, { id: 'old', capabilities: [] }, { id: 'unknown' }]);
    expect(offersGive('new')).toBe(true);
    expect(offersGive('old')).toBe(false);
    expect(offersGive('unknown'), 'not known: offered as before').toBe(true);
    expect(cannotTakeKey({ id: 'old/a', name: 'a', machineId: 'old' } as never)).toBe('give-machine-unsupported');
    noteCapabilities([{ id: 'old', capabilities: ['task.dispatch'] }]);
    expect(offersGive('old'), 'updated').toBe(true);
  });
});
