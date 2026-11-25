import { describe, expect, it } from 'vitest';
import { actionKey, loadAudit, outcomeKind } from './audit';

const reply = (status: number, body: unknown = []) => (async () => new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;

describe('audit', () => {
  it('lists the newest row first and asks for one person only when the name is plain', async () => {
    let seen = '';
    const f = (async (url: string) => { seen = url; return new Response(JSON.stringify([
      { id: 1, at_ms: 10, who: 'owner', action: 'sign_in', node: '', outcome: 'ok' },
      { id: 2, at_ms: 20, who: 'eda', session: 'a1b2c3', action: 'AgentStop', node: 'n1', outcome: 'auth.forbidden' },
    ])); }) as unknown as typeof fetch;
    const r = await loadAudit('tok', 'eda', f);
    expect(seen).toBe('/api/audit?limit=500&who=eda');
    expect(r.ok && r.rows.map((x) => x.id)).toEqual([2, 1]);
    // A row from a hub that predates sessions in the trail has none.
    expect(r.ok && r.rows.map((x) => x.session)).toEqual(['a1b2c3', '']);
    await loadAudit('tok', 'a b&c', f);
    expect(seen).toBe('/api/audit?limit=500');
  });
  it('says why it could not', async () => {
    expect(await loadAudit('t', '', reply(403))).toEqual({ ok: false, reason: 'owner-only' });
    expect(await loadAudit('t', '', reply(401))).toEqual({ ok: false, reason: 'signed-out' });
    expect(await loadAudit('t', '', reply(500))).toEqual({ ok: false, reason: 'failed' });
    expect(await loadAudit('t', '', (async () => { throw new Error('down'); }) as unknown as typeof fetch)).toEqual({ ok: false, reason: 'failed' });
  });
  it('makes audit row metadata visible', async () => {
    const r = await loadAudit('tok', '', reply(200, [{
      id: 1, at_ms: 10, who: 'Ow\u3164ner', session: 'se\u202ession', action: 'cu\u2066stom', node: 'no\u2800de', outcome: 'er\u034fror',
    }]));
    expect(r).toEqual({ ok: true, rows: [{
      id: 1, atMs: 10, who: 'Ow\\u{3164}ner', session: 'se\\u{202e}ssion', action: 'cu\\u{2066}stom', node: 'no\\u{2800}de', outcome: 'er\\u{34f}ror',
    }] });
  });
  it('verifies audit metadata across control and private-use characters', async () => {
    const r = await loadAudit('tok', '', reply(200, [{
      id: 2, at_ms: 20, who: 'İş\u0001', session: '会\ue000', action: 'a\u202eb', node: 'n\u2066', outcome: 'x\u3164',
    }]));
    expect(r.ok && r.rows[0]).toEqual({
      id: 2, atMs: 20, who: 'İş\\u{1}', session: '会\\u{e000}', action: 'a\\u{202e}b', node: 'n\\u{2066}', outcome: 'x\\u{3164}',
    });
  });
  it('names actions and outcomes', () => {
    expect(actionKey('AgentStop')).toBe('audit-action-agentstop');
    expect(actionKey('user.add')).toBe('audit-action-user-add');
    expect(['ok', 'sent', 'node.timeout'].map(outcomeKind)).toEqual(['done', 'sent', 'problem']);
  });
});
