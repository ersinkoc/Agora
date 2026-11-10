import { describe, expect, it } from 'vitest';
import { actionKey, loadAudit, outcomeKind } from './audit';

const reply = (status: number, body: unknown = []) => (async () => new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;

describe('audit', () => {
  it('lists the newest row first and asks for one person only when the name is plain', async () => {
    let seen = '';
    const f = (async (url: string) => { seen = url; return new Response(JSON.stringify([
      { id: 1, at_ms: 10, who: 'owner', action: 'sign_in', node: '', outcome: 'ok' },
      { id: 2, at_ms: 20, who: 'eda', action: 'AgentStop', node: 'n1', outcome: 'auth.forbidden' },
    ])); }) as unknown as typeof fetch;
    const r = await loadAudit('tok', 'eda', f);
    expect(seen).toBe('/api/audit?limit=500&who=eda');
    expect(r.ok && r.rows.map((x) => x.id)).toEqual([2, 1]);
    await loadAudit('tok', 'a b&c', f);
    expect(seen).toBe('/api/audit?limit=500');
  });
  it('says why it could not', async () => {
    expect(await loadAudit('t', '', reply(403))).toEqual({ ok: false, reason: 'owner-only' });
    expect(await loadAudit('t', '', reply(401))).toEqual({ ok: false, reason: 'signed-out' });
    expect(await loadAudit('t', '', reply(500))).toEqual({ ok: false, reason: 'failed' });
    expect(await loadAudit('t', '', (async () => { throw new Error('down'); }) as unknown as typeof fetch)).toEqual({ ok: false, reason: 'failed' });
  });
  it('names actions and outcomes', () => {
    expect(actionKey('AgentStop')).toBe('audit-action-agentstop');
    expect(actionKey('user.add')).toBe('audit-action-user-add');
    expect(['ok', 'sent', 'node.timeout'].map(outcomeKind)).toEqual(['done', 'sent', 'problem']);
  });
});
