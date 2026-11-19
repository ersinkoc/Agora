import { memo } from 'react';
import { t } from '../i18n';
import { type AgentTileModel, endedWithError, shownExitCode } from '../model';
import { lastLine, span, type GroupBy, type ProjectSpread } from '../net/fleet';
import { ProjectChip } from './ProjectMark';
import { ago } from '../net/time';
import { Icon } from './Icon';
import { StateBadge } from './StateBadge';
import { NoAskBadge } from './NoAskBadge';

// What the fleet console adds to a tile: where it is drawn (the group already names the machine or the project, so the tile names the
// other one), what the agent asks for, and the time. Without it the tile is the plain one of the gallery.
export interface TileContext {
  group: GroupBy;
  ask?: string; // what the agent asks permission for, when it waits on a request
  now?: number;
  same?: ProjectSpread; // the project is open on more than one machine (net/fleet.ts, `sharedAcross`)
}

// The one line that says what the agent is doing, from what the window has: the last line of its terminal when it has a preview,
// else what it asks for, else why it stopped. Nothing is made up: with none of these the tile has no line.
function activity(a: AgentTileModel, ctx: TileContext): string {
  const line = lastLine(a.preview);
  if (ctx.ask && a.state === 'needs-you' && !a.closed) return t('fleet-tile-asks', { what: ctx.ask });
  if (line && !a.closed) return line;
  if (a.interrupted) return t('fleet-tile-stopped');
  // The badge already says it ended with an error; the line adds what the badge does not: the exit code.
  if (endedWithError(a)) { const code = shownExitCode(a); return code === undefined ? '' : t('fleet-exit-code', { code }); }
  return line;
}

function Tile({ agent, onOpen, context }: { agent: AgentTileModel; onOpen?: (id: string) => void; context?: TileContext }) {
  const failed = endedWithError(agent);
  const stateWord = t(failed ? 'state-failed' : `state-${agent.state}`);
  if (!context) {
    return (
      <button
        type="button"
        className={`tile tile--${agent.state}`}
        style={{ ['--tile-accent' as string]: agent.accent }}
        onClick={() => onOpen?.(agent.id)}
        aria-label={`${agent.name}, ${stateWord}`}
      >
        <span className="tile__name">{agent.name}</span>
        <StateBadge state={agent.state} failed={failed} />
        <NoAskBadge agent={agent} nested />
        <span className="muted">
          {t('agent-on-machine', { machine: agent.machine })} · {t('agent-workspace', { name: agent.workspace })}
        </span>
        {agent.parentName ? <span className="muted">{t('helper-started-by', { name: agent.parentName })}</span> : null}
        {agent.preview ? <span className="tile__preview" aria-hidden="true">{agent.preview}</span> : null}
      </button>
    );
  }
  const line = activity(agent, context);
  const where = context.group === 'machine' ? [agent.project || agent.workspace, agent.branch] : context.group === 'project' ? [agent.machine, agent.branch] : [agent.machine, agent.project || agent.workspace, agent.branch];
  const changed = agent.filesChanged ?? 0;
  const when = context.now === undefined ? undefined
    : agent.closed ? (agent.endedMs || agent.since ? t('fleet-tile-ended', { when: ago((agent.endedMs || agent.since) as number, context.now) }) : undefined)
      : agent.since ? { text: span(context.now - agent.since), title: t('fleet-tile-state-for', { state: stateWord, time: span(context.now - agent.since) }) } : undefined;
  return (
    <button
      type="button"
      className={`tile tile--compact tile--${failed ? 'failed' : agent.state}${agent.closed ? ' tile--closed' : ''}`}
      style={{ ['--tile-accent' as string]: agent.accent }}
      onClick={() => onOpen?.(agent.id)}
      aria-label={t('fleet-open-for', { name: `${agent.name}, ${stateWord}` })}
    >
      <span className="tile__head">
        <span className="tile__name">{agent.name}</span>
        <StateBadge state={agent.state} failed={failed} />
      </span>
      <NoAskBadge agent={agent} nested />
      {context.same && <ProjectChip spread={context.same} />}
      {line ? <span className={`tile__activity${agent.preview && !agent.closed && !(context.ask && agent.state === 'needs-you') ? ' tile__activity--line' : ''}${agent.state === 'needs-you' && !agent.closed ? ' tile__activity--ask' : ''}`} dir="auto">{line}</span> : null}
      <span className="tile__meta">
        {where.filter(Boolean).map((x, i) => <span key={i} className="tile__where">{x}</span>)}
        {changed > 0 ? <span>{t('changes-files', { count: changed })}</span> : null}
        {typeof when === 'string' ? <span>{when}</span> : when ? <span className="tile__when" title={when.title}><Icon name="clock" className="size-3" />{when.text}</span> : null}
      </span>
    </button>
  );
}

// A tile is redrawn only when what it shows changed: with hundreds of them, a filter or a regrouping redraws the ones that come or go,
// not the ones that stay. (`context` is a new object on every render of the console, so it is compared by its parts.)
export const AgentTile = memo(Tile, (a, b) => a.agent === b.agent && a.onOpen === b.onOpen && a.context?.group === b.context?.group && a.context?.ask === b.context?.ask
 && a.context?.now === b.context?.now && a.context?.same === b.context?.same && !a.context === !b.context);
