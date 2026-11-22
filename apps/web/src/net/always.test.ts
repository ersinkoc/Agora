// ADR-0101 step 4: "Allow always" in the window. The bytes are the node's (the shared vectors), the button's answer is ALWAYS only where the
// request offers it, and a request that does not offer it never sends it.
import { describe, expect, it } from 'vitest';
import { alwaysSignedMessage, approvalFailureKey, signAlways, thisDevice, toHex, type KeyStore } from './approval';
import { alwaysOffer, alwaysText, answerPlan, localAgentId, SCOPE_ALWAYS, SCOPE_ONCE } from './always';
import { applyEvent, approvalList, emptyView, fromSnapshot, tiles } from './view';
import { decidableInline } from './fleet';
import { chatLine } from './chat';
import { agora } from '../gen/agora.js';

type Vector = { name: string; node: string; approval_id: string; approved: boolean; agent_id: string; action: string; detail: string; always_label: string; always_option_id: string; hex: string };

describe('the always signature', () => {
  it('reproduces every vector of agora-approval-always-v1 byte for byte', async () => {
    const { readFileSync } = await import('node:fs');
    const vectors = JSON.parse(readFileSync('../../schema/fixtures/signing/approval-always-v1.json', 'utf8')) as { cases: Vector[] };
    expect(vectors.cases.length).toBeGreaterThanOrEqual(3);
    for (const c of vectors.cases) {
      expect(c.approved, c.name).toBe(true);
      const bytes = await alwaysSignedMessage(c.node, c.approval_id, c.agent_id, c.action, c.detail, c.always_label, c.always_option_id);
      expect(toHex(bytes), c.name).toBe(c.hex);
    }
  });

  it('signs those bytes with the device key, and they differ from a single answer and from another label or option', async () => {
    let saved: Awaited<ReturnType<KeyStore['load']>>;
    const d = await thisDevice({ load: async () => saved, save: async (x) => { saved = x; } });
    const sig = await signAlways(d, 'laptop', 'ap-1', 'agent-1', 'run_command', 'git status', 'Always allow', 'opt-1');
    expect(sig).toHaveLength(64);
    const msg = await alwaysSignedMessage('laptop', 'ap-1', 'agent-1', 'run_command', 'git status', 'Always allow', 'opt-1');
    const verify = (m: Uint8Array) => crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, d.keys.publicKey, sig as BufferSource, m as BufferSource);
    expect(await verify(msg)).toBe(true);
    expect(await verify(await alwaysSignedMessage('laptop', 'ap-1', 'agent-1', 'run_command', 'git status', 'Always allow', 'opt-2'))).toBe(false);
    expect(await verify(await alwaysSignedMessage('laptop', 'ap-1', 'agent-1', 'run_command', 'git status ', 'Always allow', 'opt-1'))).toBe(false);
  });
});

describe('what a request offers', () => {
  it('offers always only when the machine announced it, with the option id, and never for a request the machine makes', () => {
    expect(alwaysOffer('run_command', [], '', '')).toBeUndefined();
    expect(alwaysOffer('run_command', [SCOPE_ONCE], 'x', 'o')).toBeUndefined();
    expect(alwaysOffer('run_command', [SCOPE_ONCE, SCOPE_ALWAYS], 'Always', 'o')).toEqual({ label: 'Always', optionId: 'o', only: false });
    expect(alwaysOffer('run_command', [SCOPE_ALWAYS], '', 'o')).toEqual({ label: '', optionId: 'o', only: true });
    expect(alwaysOffer('run_command', [SCOPE_ALWAYS], 'x', '')).toBeUndefined();
    expect(alwaysOffer('agent.start', [SCOPE_ONCE, SCOPE_ALWAYS], 'x', 'o')).toBeUndefined();
    expect(alwaysOffer('agent.skip_permissions', [SCOPE_ALWAYS], 'x', 'o')).toBeUndefined();
    // A scope this window does not know is neither.
    expect(alwaysOffer('run_command', [7, SCOPE_ALWAYS], 'x', 'o')?.only).toBe(true);
  });

  it('names the agent by the node\'s own id, never by a key of another machine', () => {
    expect(localAgentId({ node: 'laptop', agentId: 'laptop/agent-1' })).toBe('agent-1');
    expect(localAgentId({ node: 'laptop', agentId: 'laptop/a/b' })).toBe('a/b');
    expect(localAgentId({ node: 'laptop', agentId: 'box/agent-1' })).toBeUndefined();
    expect(localAgentId({ node: '', agentId: '/agent-1' })).toBeUndefined();
    expect(localAgentId({ node: 'laptop', agentId: 'laptop/' })).toBeUndefined();
  });

  it('sends ALWAYS only for Allow on a request that offers it; Deny stays single; a plain Allow never goes to an always-only request', () => {
    const both = { node: 'n', agentId: 'n/a', always: { label: 'L', optionId: 'o', only: false } };
    const only = { node: 'n', agentId: 'n/a', always: { label: 'L', optionId: 'o', only: true } };
    const plain = { node: 'n', agentId: 'n/a' };
    expect(answerPlan(both, true, true)).toEqual({ scope: 'always', agentId: 'a', offer: both.always });
    expect(answerPlan(both, true, false)).toEqual({ scope: 'once' });
    expect(answerPlan(both, false, true)).toEqual({ scope: 'none' });
    expect(answerPlan(only, true, true).scope).toBe('always');
    expect(answerPlan(only, true, false)).toEqual({ scope: 'none' });
    expect(answerPlan(only, false, false)).toEqual({ scope: 'once' });
    expect(answerPlan(plain, true, true)).toEqual({ scope: 'none' });
    expect(answerPlan(plain, true, false)).toEqual({ scope: 'once' });
    expect(answerPlan({ ...both, start: {} }, true, true)).toEqual({ scope: 'none' });
    expect(answerPlan({ ...both, skip: {} }, true, true)).toEqual({ scope: 'none' });
    expect(answerPlan({ ...both, agentId: 'other/a' }, true, true)).toEqual({ scope: 'none' });
  });

  it('says "for this session" only for a kind known to drop the answer, which no kind is yet', () => {
    expect(alwaysText('approval-allow-always')).toBe('approval-allow-always-kept');
    expect(alwaysText('approval-allow-always', true)).toBe('approval-allow-always');
  });

  it('names the new refusals in the window\'s words', () => {
    expect(approvalFailureKey('approval.scope_forbidden')).toBe('approval-scope-forbidden');
    expect(approvalFailureKey('approval.scope_unavailable')).toBe('approval-scope-unavailable');
    expect(approvalFailureKey('approval.adapter_outdated')).toBe('approval-adapter-outdated');
  });
});

describe('the view', () => {
  const started = (v: ReturnType<typeof emptyView>, pos: number) => applyEvent(v, pos, { producerId: 'n', body: 'agentStarted', agentStarted: { agentId: 'a', nodeId: 'n', kind: 'wstack' } });

  it('reads the offer from a live request and from a snapshot, the texts exactly as they came', () => {
    const v = emptyView();
    started(v, 1);
    applyEvent(v, 2, { producerId: 'n', body: 'approvalRequested', approvalRequested: { approvalId: 'p', agentId: 'a', action: 'run_command', detail: ' git status ', scopes: [SCOPE_ONCE, SCOPE_ALWAYS], alwaysLabel: ' Always ', alwaysOptionId: 'opt' } });
    applyEvent(v, 3, { producerId: 'n', body: 'approvalRequested', approvalRequested: { approvalId: 'q', agentId: 'a', action: 'write', detail: 'x', scopes: [SCOPE_ALWAYS], alwaysLabel: '', alwaysOptionId: 'edits' } });
    applyEvent(v, 4, { producerId: 'n', body: 'approvalRequested', approvalRequested: { approvalId: 'r', agentId: 'a', action: 'read', detail: 'y' } });
    const by = new Map(approvalList(v).map((a) => [a.localId, a]));
    expect(by.get('p')?.always).toEqual({ label: ' Always ', optionId: 'opt', only: false });
    expect(by.get('p')?.detail).toBe(' git status ');
    expect(by.get('q')?.always?.only).toBe(true);
    expect(by.get('r')?.always).toBeUndefined();
    // An always-only request has no plain Allow: never answered from a one-line row.
    expect(decidableInline(by.get('q')!)).toBe(false);
    expect(decidableInline(by.get('p')!)).toBe(true);

    const s = fromSnapshot({ approvals: [{ approvalId: 'n/p', agentId: 'n/a', action: 'run_command', detail: 'd', scopes: [SCOPE_ALWAYS], alwaysLabel: 'L', alwaysOptionId: 'o' }] });
    const [a] = approvalList(s);
    expect(a?.always).toEqual({ label: 'L', optionId: 'o', only: true });
    expect(a && localAgentId(a)).toBe('a');
  });

  it('lists the always answers an agent holds, once each, at most 16, until it ends, and from a snapshot', () => {
    const v = emptyView();
    started(v, 1);
    let pos = 2;
    const allowed = (id: string, action: string) => applyEvent(v, pos++, { producerId: 'n', body: 'agentAlwaysAllowed', agentAlwaysAllowed: { agentId: 'a', approvalId: id, action } });
    allowed('p1', 'run_command');
    allowed('p1', 'run_command'); // said again after a restart
    allowed('p2', 'write_file');
    expect(tiles(v)[0]?.alwaysAllowed).toEqual(['run_command', 'write_file']);
    for (let i = 0; i < 20; i++) allowed(`x${i}`, `a${i}`);
    expect(tiles(v)[0]?.alwaysAllowed).toHaveLength(16);
    expect(tiles(v)[0]?.alwaysAllowed?.at(-1)).toBe('a19');
    applyEvent(v, pos++, { producerId: 'n', body: 'agentExited', agentExited: { agentId: 'a', exitCode: 0 } });
    expect(tiles(v)[0]?.alwaysAllowed).toBeUndefined();
    allowed('late', 'run_command'); // an ended agent holds nothing
    expect(tiles(v)[0]?.alwaysAllowed).toBeUndefined();
    started(v, pos++);
    expect(tiles(v)[0]?.alwaysAllowed).toBeUndefined();

    const s = fromSnapshot({ agents: [{ agentId: 'n/a', nodeId: 'n', kind: 'k', running: true, alwaysAllowed: ['run_command'] }] });
    expect(tiles(s)[0]?.alwaysAllowed).toEqual(['run_command']);
  });

  it('reads an approved always answer in the conversation', () => {
    const line = chatLine({ entryId: 1, kind: agora.v1.ChatKind.CHAT_KIND_PERMISSION, approvalId: 'p', permissionOutcome: agora.v1.PermissionOutcome.PERMISSION_OUTCOME_APPROVED, permissionAlways: true });
    expect(line.outcome).toBe('approved');
    expect(line.always).toBe(true);
    expect(chatLine({ entryId: 2, kind: agora.v1.ChatKind.CHAT_KIND_PERMISSION }).always).toBeUndefined();
  });
});
