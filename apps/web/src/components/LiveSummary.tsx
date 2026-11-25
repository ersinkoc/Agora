import type { AgentTileModel } from '../model';
import { t } from '../i18n';
import { duration } from '../net/time';
import { Icon } from './Icon';
import { StateBadge } from './StateBadge';
import { shortCount } from './sessionChoice';
import { visible } from '../net/hidden';

// The last thing on a terminal agent's screen: its last line that says something, cut to what fits a strip. Only what the window already
// has (the preview it keeps from the node's cell snapshot); nothing is read from the viewport and nothing is invented.
export function lastScreenLine(preview: string, max = 200): string {
  const lines = preview.split('\n').map((l) => l.trim()).filter(Boolean);
  const last = lines[lines.length - 1] ?? '';
  return last.length > max ? `${last.slice(0, max - 1)}…` : last;
}

// A compact strip above a terminal agent's terminal: its state, its last line, how many files it changed (one click opens the Changes
// panel), how long it has run and how many tokens it used. A terminal agent has no structured stream, so nothing else is shown.
export function LiveSummary({ agent, now, onChanges }: { agent: AgentTileModel; now: number; onChanges?: () => void }) {
  const line = lastScreenLine(agent.preview);
  const changed = agent.filesChanged ?? 0;
  const elapsed = agent.startedMs ? Math.max(0, now - agent.startedMs) : undefined;
  if (!line && changed === 0 && elapsed === undefined && !agent.tokens) return null;
  return (
    <div className="flex min-w-0 flex-none flex-wrap items-center gap-x-3 gap-y-1 border-b border-line bg-surface-1 px-3 py-1.5 text-xs" role="group" aria-label={t('live-summary')}>
      <StateBadge state={agent.state} />
      <span dir="auto" className="min-w-0 flex-1 basis-48 truncate font-mono text-muted" title={line ? visible(line) : undefined}>{visible(line)}</span>
      <span className="flex flex-none flex-wrap items-center gap-x-2 gap-y-1">
        {elapsed !== undefined && <span className="ide-chip" title={t('session-elapsed-hint')}><Icon name="clock" className="size-3" />{duration(elapsed)}</span>}
        {!!agent.tokens && <span className="ide-chip">{t('agent-tokens', { count: shortCount(agent.tokens) })}</span>}
        {changed > 0 && (onChanges
          ? <button type="button" className="ide-chip transition-colors hover:border-accent hover:text-ink" onClick={onChanges} title={t('live-open-changes')}><Icon name="diff" className="size-3" />{t('changes-files', { count: changed })}</button>
          : <span className="ide-chip">{t('changes-files', { count: changed })}</span>)}
      </span>
    </div>
  );
}
