import { memo } from 'react';
import { t } from '../i18n';
import { type AgentTileModel, endedWithError, shownExitCode } from '../model';
import { lastLine, span, type GroupBy, type ProjectSpread } from '../net/fleet';
import { ProjectChip } from './ProjectMark';
import { ago } from '../net/time';
import { Icon, type IconName } from './Icon';
import { AgentAvatar, StatusPill, Tag } from './kit';
import { NoAskBadge } from './NoAskBadge';
import { AsksLessChip } from './AllowAlways';
import { LimitChip } from './Quota';
import { FinishedMark, useFinishedUnseen } from './FinishedMark';
import { visible } from '../net/hidden';

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

// The kind the avatar's letter comes from: what the node reported, else the first part of the generated name ("claude · amber").
export const kindOf = (a: { kind?: string; name: string }): string => a.kind || a.name.split(' · ')[0] || '?';

// Who it is and how it is: the avatar (its colour lives here, never on an edge of the tile), the name, and the state as a pill. Failed is
// the red pill with a cross in a ring (ADR-0102).
function Head({ agent, failed }: { agent: AgentTileModel; failed: boolean }) {
  return (
    <span className="tile__head">
      <AgentAvatar accent={agent.accent} kind={kindOf(agent)} />
      <span className="tile__name">{visible(agent.name)}</span>
      <FinishedMark agentId={agent.id} />
      <StatusPill state={agent.state} failed={failed} />
    </span>
  );
}

function Tile({ agent, onOpen, context }: { agent: AgentTileModel; onOpen?: (id: string) => void; context?: TileContext }) {
  const failed = endedWithError(agent);
  // The same word as the pill, so the tile's name holds what it shows, and the finished mark's words while it has one.
  const stateWord = t(failed ? 'fleet-failed' : `state-${agent.state}`);
  const unseen = useFinishedUnseen(agent.id);
  const said = unseen ? `${stateWord}, ${t('finished-unseen')}` : stateWord;
  if (!context) {
    return (
      <button
        type="button"
        className={`tile tile--${failed ? 'failed' : agent.state}`}
        onClick={() => onOpen?.(agent.id)}
        aria-label={`${visible(agent.name)}, ${said}${agent.skipsPermissions ? `, ${t('agent-does-not-ask')}` : ''}`}
      >
        <Head agent={agent} failed={failed} />
        <NoAskBadge agent={agent} nested />
        <AsksLessChip agent={agent} nested />
        <span className="muted">
          {t('agent-on-machine', { machine: visible(agent.machine) })} · {t('agent-workspace', { name: visible(agent.workspace) })}
        </span>
        {agent.parentName ? <span className="muted">{t('helper-started-by', { name: visible(agent.parentName) })}</span> : null}
        {agent.preview ? <span className="tile__preview" aria-hidden="true">{visible(agent.preview)}</span> : null}
      </button>
    );
  }
  const line = activity(agent, context);
  // Where it is, as grey tags: the side the group does not already name, then the branch.
  const machine: Fact = { icon: 'machine', text: agent.machine };
  const folder: Fact = { icon: 'folder', text: agent.project || agent.workspace };
  const branch: Fact = { icon: 'branch', text: agent.branch, mono: true };
  const where = context.group === 'machine' ? [folder, branch] : context.group === 'project' ? [machine, branch] : [machine, folder, branch];
  const changed = agent.filesChanged ?? 0;
  const when = context.now === undefined ? undefined
    : agent.closed ? (agent.endedMs || agent.since ? t('fleet-tile-ended', { when: ago((agent.endedMs || agent.since) as number, context.now) }) : undefined)
      : agent.since ? { text: span(context.now - agent.since), title: t('fleet-tile-state-for', { state: stateWord, time: span(context.now - agent.since) }) } : undefined;
  return (
    <button
      type="button"
      className={`tile tile--compact tile--${failed ? 'failed' : agent.state}${agent.closed ? ' tile--closed' : ''}`}
      onClick={() => onOpen?.(agent.id)}
      aria-label={t('fleet-open-for', { name: `${visible(agent.name)}, ${said}${agent.skipsPermissions ? `, ${t('agent-does-not-ask')}` : ''}` })}
    >
      <Head agent={agent} failed={failed} />
      {line ? <span className="tile__activity" dir="auto">{visible(line)}</span> : null}
      <span className="tile__foot">
        <NoAskBadge agent={agent} nested />
        <AsksLessChip agent={agent} nested />
        <LimitChip agent={agent} />
        {context.same && <ProjectChip spread={context.same} />}
        {where.filter((x) => x.text).map((x) => <Tag key={x.icon} mono={x.mono}><Icon name={x.icon} className="size-3" /><span className="tile__where">{visible(x.text ?? '')}</span></Tag>)}
        {changed > 0 ? <Tag>{t('changes-files', { count: changed })}</Tag> : null}
        {typeof when === 'string' ? <span className="tile__when">{when}</span> : when ? <span className="tile__when" title={when.title}><Icon name="clock" className="size-3" />{when.text}</span> : null}
      </span>
    </button>
  );
}

type Fact = { icon: IconName; text?: string; mono?: boolean };

// A tile is redrawn only when what it shows changed: with hundreds of them, a filter or a regrouping redraws the ones that come or go,
// not the ones that stay. (`context` is a new object on every render of the console, so it is compared by its parts.)
export const AgentTile = memo(Tile, (a, b) => a.agent === b.agent && a.onOpen === b.onOpen && a.context?.group === b.context?.group && a.context?.ask === b.context?.ask
 && a.context?.now === b.context?.now && a.context?.same === b.context?.same && !a.context === !b.context);
