import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import type { ApprovalModel, MachineModel } from '../net/view';
import { approvalRedirect, backOf, byMachine, dockOf, needsYou, phoneScreen } from './nav';

const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: id, accent: '', machine: id.split('/')[0] ?? '', machineId: id.split('/')[0], workspace: 'w', state: 'working', preview: '', ...over });
const approval = (id: string, agentId: string, expiresAtMs: number): ApprovalModel =>
  ({ id, node: id.split('/')[0] ?? '', localId: id.split('/')[1] ?? '', agentId, agentName: agentId, action: 'run', detail: '', expiresAtMs });
const machine = (id: string, online = true): MachineModel => ({ id, name: id, os: 'linux', online, agentsRunning: 0, lastSeenMs: 0 });

describe('phone navigation (ADR-0067 rung 2)', () => {
  it('reads the screen from the address', () => {
    expect(phoneScreen('')).toEqual({ kind: 'home' });
    expect(phoneScreen('#/')).toEqual({ kind: 'home' });
    expect(phoneScreen('#/agent/laptop%2Fa1')).toEqual({ kind: 'agent', id: 'laptop/a1' });
    expect(phoneScreen('#/approvals/laptop%2Fp%201')).toEqual({ kind: 'approval', id: 'laptop/p 1' });
    expect(phoneScreen('#/machines')).toEqual({ kind: 'page' });
    expect(phoneScreen('#/theme')).toEqual({ kind: 'theme' });
    // The side panel's deep subjects, each a screen of its own on a phone.
    expect(phoneScreen('#/board')).toEqual({ kind: 'details', tab: 'board' });
    expect(phoneScreen('#/messages')).toEqual({ kind: 'details', tab: 'messages' });
    expect(phoneScreen('#/usage')).toEqual({ kind: 'details', tab: 'usage' });
    expect(phoneScreen('#/device')).toEqual({ kind: 'page' });
    // Anything unknown, or an address that does not decode, is the list.
    expect(phoneScreen('#/nope')).toEqual({ kind: 'home' });
    expect(phoneScreen('#/agent/%E0%A4%A')).toEqual({ kind: 'home' });
    expect(phoneScreen('#/agent/')).toEqual({ kind: 'home' });
  });

  it('goes back to the list from an agent or a page, and nowhere from the list', () => {
    expect(backOf({ kind: 'agent', id: 'x' })).toBe('#/');
    expect(backOf({ kind: 'page' })).toBe('#/');
    expect(backOf({ kind: 'theme' })).toBe('#/');
    expect(backOf({ kind: 'home' })).toBeUndefined();
  });

  it('opens the agent a notice is about, or the list once it is answered', () => {
    const list = [approval('laptop/p1', 'laptop/a1', 5)];
    const screen = phoneScreen('#/approvals/laptop%2Fp1');
    expect(approvalRedirect(screen, list, true)).toBe('#/agent/laptop%2Fa1');
    // Before the fleet arrives it waits; after, an approval that is gone opens the list.
    expect(approvalRedirect(screen, [], false)).toBeUndefined();
    expect(approvalRedirect(screen, [], true)).toBe('#/');
    expect(approvalRedirect({ kind: 'home' }, list, true)).toBeUndefined();
  });

  it('puts requests first (soonest to expire), then agents waiting without one (longest first)', () => {
    const fleet = [agent('m/a1', { state: 'needs-you', since: 30 }), agent('m/a2', { state: 'needs-you', since: 10 }), agent('m/a3', { state: 'needs-you', since: 20 }), agent('m/a4'),
      agent('m/a5', { state: 'needs-you', since: 1, closed: true })];
    const items = needsYou([approval('m/p2', 'm/a3', 9), approval('m/p1', 'm/a1', 4)], fleet);
    expect(items.map((i) => (i.kind === 'approval' ? `ask:${i.approval.id}` : `agent:${i.agent.id}`))).toEqual(['ask:m/p1', 'ask:m/p2', 'agent:m/a2']);
    // A request carries its agent when the fleet has it.
    expect(items[0]?.kind === 'approval' && items[0].agent?.id).toBe('m/a1');
  });

  it('groups agents by machine: online first, urgent first, closed last', () => {
    const fleet = [agent('b/a1', { closed: true, state: 'finished' }), agent('b/a2', { state: 'idle' }), agent('b/a3', { state: 'needs-you' }), agent('ghost/a9')];
    const groups = byMachine(fleet, [machine('a', false), machine('b'), machine('c')]);
    expect(groups.map((g) => g.id)).toEqual(['b', 'c', 'ghost', 'a']);
    expect(groups[0]?.agents.map((a) => a.id)).toEqual(['b/a3', 'b/a2', 'b/a1']);
    expect(groups.find((g) => g.id === 'c')?.agents).toEqual([]);
  });
});

// The install manifest (served by the hub) names these files; each must be the size it says.
describe('install icons', () => {
  const size = (file: string) => { const b = readFileSync(`public/${file}`); return [b.readUInt32BE(16), b.readUInt32BE(20)]; };
  it('has the PNG sizes the manifest and the page name', () => {
    expect(size('icon-192.png')).toEqual([192, 192]);
    expect(size('icon-512.png')).toEqual([512, 512]);
    expect(size('icon-maskable-512.png')).toEqual([512, 512]);
    expect(size('apple-touch-icon.png')).toEqual([180, 180]);
    expect(readFileSync('index.html', 'utf8')).toContain('href="/apple-touch-icon.png"');
    expect(readFileSync('../../crates/agora-hub/src/web.rs', 'utf8')).toContain('/icon-maskable-512.png');
  });
});

describe('the bar for what needs you', () => {
  const plain = (id: string, over: Partial<ApprovalModel> = {}): ApprovalModel => ({ ...approval(id, 'n/a1', 5), action: 'Edit src/auth.rs', detail: '+12 -3', ...over });
  const dockFor = (a: ApprovalModel, pairs = 0) => dockOf(needsYou([a], [agent('n/a1')]), pairs);
  it('answers a short plain request in place', () => {
    expect(dockFor(plain('n/1')).dock?.kind).toBe('answer');
  });
  it('only reviews a request to start an agent, long, multi-line or hidden-character text', () => {
    expect(dockFor({ ...plain('n/2'), start: { agent: 'a1' } as ApprovalModel['start'] }).dock?.kind).toBe('review');
    expect(dockFor(plain('n/3', { detail: 'x'.repeat(200) })).dock?.kind).toBe('review');
    expect(dockFor(plain('n/4', { detail: 'one\ntwo' })).dock?.kind).toBe('review');
    expect(dockFor(plain('n/5', { action: 'rm‮ -rf' })).dock?.kind).toBe('review');
  });
  it('an agent that waits is opened, a machine that asks to join is reviewed (never answered), and the count is all of them', () => {
    expect(dockOf(needsYou([], [agent('n/a1', { state: 'needs-you' })]), 1)).toMatchObject({ dock: { kind: 'agent' }, count: 2 });
    expect(dockOf([], 2)).toEqual({ dock: { kind: 'pair', count: 2 }, count: 2 });
    expect(dockOf([], 0)).toEqual({ dock: undefined, count: 0 });
  });
});

describe('one ordering for what needs you', () => {
  it('a request without an expiry never takes the bar from one that is about to lapse', () => {
    const soon = approval('n/a', 'n/a1', 5000);
    const never = approval('n/b', 'n/a2', 0);
    expect(needsYou([never, soon], []).map((x) => (x.kind === 'approval' ? x.approval.id : ''))).toEqual(['n/a', 'n/b']);
    expect(dockOf(needsYou([never, soon], [])).dock).toMatchObject({ approval: { id: 'n/a' } });
  });
  it('whitespace tricks are not a plain request', () => {
    const plain = (over: Partial<ApprovalModel>) => dockOf(needsYou([{ ...approval('n/w', 'n/a1', 5), action: 'Edit a', detail: 'b', ...over }], [])).dock?.kind;
    expect(plain({})).toBe('answer');
    for (const ch of ['\t', '\u00a0', '\u3000', '\u200a', '\u2009', '\u202f']) {
      expect(plain({ action: `Edit${ch}a` }), JSON.stringify(ch)).toBe('review');
      expect(plain({ detail: `b${ch}c` }), JSON.stringify(ch)).toBe('review');
    }
  });
});
