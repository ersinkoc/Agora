// What the agents cost so far (ADR-0033), estimated by the hub from its price table and asked for again every half minute.
import { useEffect, useState } from 'react';
import type { Live } from './live';
import type { PairOwner } from './pairing';
import { summarize, type UsageSummary } from './usage';

/** How often the cost is asked for again while the window is connected. */
export const USAGE_POLL_MS = 30_000;

/**
 * The hub's cost report for this connection. The hub answers `UsageReport` to the owner only, and every refusal is a row in the owner's
 * Activity (ADR-0049), so the window asks only once `owner` (the hub's `PairStatus` answer) says this session is the owner's, or that the
 * hub predates pairing (`unavailable`, where it asks as it always did). A member's or viewer's window (`no`) never asks (#1044); with a
 * hub that names the role (ADR-0106) it is `no` at once, without asking `PairStatus` either.
 */
export function useUsageReport(api: Live | undefined, live: boolean, owner: PairOwner): UsageSummary | undefined {
  const [usage, setUsage] = useState<UsageSummary | undefined>();
  const asks = owner === 'yes' || owner === 'unavailable';
  useEffect(() => {
    if (!api || !live || !asks) return;
    let stop = false;
    const ask = () => { api.command('', { usageReport: {} }).then((ok) => { if (!stop && ok.usageReport) setUsage(summarize(ok.usageReport)); }).catch(() => {}); };
    ask();
    const timer = setInterval(ask, USAGE_POLL_MS);
    return () => { stop = true; clearInterval(timer); };
  }, [api, live, asks]);
  return usage;
}
