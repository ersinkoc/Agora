// The audit trail (ADR-0049): who did what, where, and how it ended. The hub serves it to its owner only; this file asks for it and
// turns a row into words the page can show. Pure apart from the one fetch, which a test replaces.

export interface AuditRow { id: number; atMs: number; who: string; action: string; node: string; outcome: string }

export type AuditResult = { ok: true; rows: AuditRow[] } | { ok: false; reason: 'owner-only' | 'signed-out' | 'failed' };

export async function loadAudit(token: string, who: string, fetcher: typeof fetch = (...a) => fetch(...a)): Promise<AuditResult> {
  const query = /^[a-z0-9-]+$/i.test(who) ? `?limit=500&who=${who}` : '?limit=500';
  let res: Response;
  try {
    res = await fetcher(`/api/audit${query}`, { headers: { authorization: `Bearer ${token}` } });
  } catch {
    return { ok: false, reason: 'failed' };
  }
  if (res.status === 403) return { ok: false, reason: 'owner-only' };
  if (res.status === 401) return { ok: false, reason: 'signed-out' };
  if (!res.ok) return { ok: false, reason: 'failed' };
  try {
    const raw = (await res.json()) as { id: number; at_ms: number; who: string; action: string; node: string; outcome: string }[];
    // Newest first: that is what someone looking at a trail wants to see.
    return { ok: true, rows: raw.map((r) => ({ id: r.id, atMs: r.at_ms, who: r.who, action: r.action, node: r.node, outcome: r.outcome })).reverse() };
  } catch {
    return { ok: false, reason: 'failed' };
  }
}

/** The locale key for an action's name: `AgentStop` → `audit-action-agentstop`, `user.add` → `audit-action-user-add`. */
export const actionKey = (action: string): string => `audit-action-${action.replace(/[._]/g, '-').toLowerCase()}`;

/** How a row ended, as a locale key: done, still on its way, or refused/failed with a code the page shows as it is. */
export const outcomeKind = (outcome: string): 'done' | 'sent' | 'problem' => (outcome === 'ok' ? 'done' : outcome === 'sent' ? 'sent' : 'problem');
