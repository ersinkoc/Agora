// @vitest-environment jsdom
// What the window asks the hub about cost (ADR-0033) on each connection: only the owner's window asks (#1044). The hub answers
// `UsageReport` to the owner alone, and each refusal is a row in the owner's Activity, so a member's or viewer's window must not ask.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CommandError, type Live } from './live';
import { usePairing } from './usePairing';
import { useUsageReport } from './useUsage';
import type { PersonRole } from './role';
import type { UsageSummary } from './usage';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { vi.useFakeTimers(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); vi.useRealTimers(); });
const wait = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });

/**
 * A hub whose `PairStatus` answer says who this session is: the owner, someone else, an older hub without pairing, or no answer yet.
 * `role` is what its `Welcome` named (ADR-0106); `unspecified` for a hub that predates it.
 */
const harness = (who: 'owner' | 'guest' | 'older' | 'waiting', role: PersonRole = 'unspecified') => {
  const sent: string[] = [];
  const api = {
    command: (_node: string, body: Record<string, unknown>) => {
      const name = Object.keys(body)[0] ?? '';
      sent.push(name);
      if (name === 'pairStatus') {
        if (who === 'waiting') return new Promise(() => {});
        if (who === 'guest') return Promise.reject(new CommandError('auth.forbidden', { action: 'pair.status' }));
        if (who === 'older') return Promise.reject(new CommandError('protocol.capability_missing', { capability: 'pair' }));
        return Promise.resolve({ pairStatus: { open: false, closesAtMs: 0, pending: 0 } });
      }
      if (name === 'usageReport') return who === 'guest' ? Promise.reject(new CommandError('auth.forbidden', { action: 'UsageReport' })) : Promise.resolve({ usageReport: { agents: [], totalMicroUsd: 1500 } });
      return Promise.resolve({});
    },
  } as unknown as Live;
  let usage: UsageSummary | undefined;
  let owner: string | undefined;
  const Probe = () => { const p = usePairing(api, 'live', role); owner = p.owner; usage = useUsageReport(api, true, p.owner); return null; };
  act(() => root.render(<Probe />));
  return { asked: () => sent.filter((n) => n === 'usageReport').length, usage: () => usage, statusAsked: () => sent.filter((n) => n === 'pairStatus').length, owner: () => owner };
};

describe('the cost the window asks the hub for (#1044)', () => {
  it('a member\'s or viewer\'s window never asks: no refused request, so no row in the owner\'s Activity', async () => {
    const h = harness('guest');
    await wait(0);
    await wait(120_000);
    expect(h.asked()).toBe(0);
    expect(h.usage()).toBeUndefined();
  });

  it('waits for the hub to say whose window this is before asking', async () => {
    const h = harness('waiting');
    await wait(60_000);
    expect(h.asked()).toBe(0);
  });

  it('the owner\'s window asks on connect and again every half minute', async () => {
    const h = harness('owner');
    await wait(0);
    expect(h.asked()).toBe(1);
    expect(h.usage()?.totalMicroUsd).toBe(1500);
    await wait(30_000);
    expect(h.asked()).toBe(2);
  });

  it('an older hub that does not know pairing is asked as before', async () => {
    const h = harness('older');
    await wait(0);
    expect(h.asked()).toBe(1);
    expect(h.usage()?.totalMicroUsd).toBe(1500);
  });
});

describe('a hub that names the role in its Welcome (ADR-0106)', () => {
  it('a member\'s or viewer\'s window asks neither PairStatus nor UsageReport, so it writes no refused row', async () => {
    for (const role of ['member', 'viewer'] as const) {
      const h = harness('guest', role);
      await wait(0);
      await wait(120_000);
      expect([h.statusAsked(), h.asked(), h.owner()]).toEqual([0, 0, 'no']);
      act(() => root.unmount());
      root = createRoot(host);
    }
  });

  it('the owner\'s window asks whether pairing is on, then for the cost', async () => {
    const h = harness('owner', 'owner');
    await wait(0);
    expect([h.statusAsked(), h.asked(), h.owner()]).toEqual([1, 1, 'yes']);
  });

  it('the owner of a hub with pairing off still gets the cost', async () => {
    const h = harness('older', 'owner');
    await wait(0);
    expect([h.statusAsked(), h.asked(), h.owner()]).toEqual([1, 1, 'unavailable']);
  });
});
