// Estimated cost, as the hub reports it (ADR-0033, `usage.report`). The hub does the arithmetic from its price table;
// this file only reads the answer and formats money. An agent with no price has no number: never shown as zero.
import type { agora } from '../gen/agora';

export interface UsageSummary {
  perAgent: Map<string, number>; // agent key `<node>/<agent>` → micro-USD, only where the hub had a price
  perNode: Map<string, number>; // node id → micro-USD
  totalMicroUsd: number;
  pricesAtMs: number; // 0: the table that came with the hub's release
}

export function summarize(r: agora.v1.UsageReportResult.$Properties): UsageSummary {
  const perAgent = new Map<string, number>();
  const perNode = new Map<string, number>();
  for (const a of r.agents ?? []) {
    if (a.microUsd === undefined || a.microUsd === null) continue;
    const micro = Number(a.microUsd);
    const id = a.agentId ?? '';
    perAgent.set(id, micro);
    const at = id.indexOf('/');
    if (at > 0) perNode.set(id.slice(0, at), (perNode.get(id.slice(0, at)) ?? 0) + micro);
  }
  return { perAgent, perNode, totalMicroUsd: Number(r.totalMicroUsd ?? 0), pricesAtMs: Number(r.pricesAtMs ?? 0) };
}

// "$1.24"; amounts under a cent say so instead of rounding to $0.00.
export function money(microUsd: number, locale?: string): string {
  const f = new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD' });
  if (microUsd > 0 && microUsd < 10_000) return `<${f.format(0.01)}`;
  return f.format(microUsd / 1_000_000);
}
