import { memo, useMemo, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { CallStatus, ChatLine } from '../net/chat';
import { parseHunks } from '../net/diff';
import { blocks, inlines, type Block, type Inline } from '../net/markdown';
import { titleSays, toolView, type ToolAction } from '../net/tool';
import { actionOf, changedFiles, stepsSummary, type GroupFlag, type TranscriptItem } from '../net/transcript';
import { callMs, currentStep, outputTail, planProgress, planSteps, stepTone, type StepStatus, type StepTone } from '../net/chatView';
import { preciseDuration } from '../net/time';
import { useHighlightedCode } from '../highlight/useHighlight';
import { Icon, type IconName } from './Icon';
import { AgentAvatar } from './kit';
import { alwaysText } from '../net/always';
import { visible } from '../net/hidden';

// How one conversation entry looks. Everything here was written by an agent or a person: it is drawn as React text, never as HTML
// (the one exception is the syntax highlighter's own output, which is escaped text inside <span class> tags).

const ICONS: Record<ToolAction, IconName> = {
  read: 'files', edit: 'pencil', run: 'terminal', search: 'search', fetch: 'globe', delete: 'trash', move: 'move', think: 'think', other: 'step',
};

function Inlines({ parts }: { parts: Inline[] }) {
  return <>{parts.map((p, i) => p.kind === 'code'
    ? <code key={i} className="rounded-xs bg-surface-2 px-1 py-px font-mono text-[0.92em]">{visible(p.text)}</code>
    : p.kind === 'strong' ? <strong key={i} className="font-semibold text-ink">{visible(p.text)}</strong>
      // `href` passed `safeLink` when it was read: http or https only. It opens without access to this window. On a touch screen it reaches
      // into the lines above and below (#1073); its text, in the inner span, stays above a neighbouring link's reach (`.tap-inline`).
      : p.kind === 'link' && p.href ? <a key={i} href={p.href} target="_blank" rel="noopener noreferrer" className="tap-inline text-accent underline underline-offset-2 hover:brightness-110"><span>{visible(p.text)}</span></a>
        : <span key={i}>{visible(p.text)}</span>)}</>;
}

// What a person copies: the text as written. Always reachable by keyboard (it shows on focus even where it hides until hover).
export function CopyButton({ text, label, className = '' }: { text: string; label?: string; className?: string }) {
  const [done, setDone] = useState(false);
  if (typeof navigator === 'undefined' || !navigator.clipboard) return null;
  const name = t(done ? 'chat-copied' : label ?? 'chat-copy');
  return (
    <button type="button" className={`ide-iconbtn size-6 ${className}`} aria-label={name} title={name}
      onClick={() => { void navigator.clipboard.writeText(text).then(() => { setDone(true); setTimeout(() => setDone(false), 1500); }, () => {}); }}>
      <Icon name={done ? 'check' : 'copy'} className="size-3.5" />
    </button>
  );
}
const HOVER = 'opacity-0 transition-opacity duration-150 group-hover/msg:opacity-100 focus-visible:opacity-100 max-md:opacity-100 pointer-coarse:opacity-100';

function CodeBlock({ lang, text }: { lang: string; text: string }) {
  const html = useHighlightedCode(lang, text);
  return (
    <div className="overflow-hidden rounded-s border border-line bg-surface-1">
      <div className="flex h-7 items-center justify-between gap-2 border-b border-line ps-3 pe-1 text-xs text-muted pointer-coarse:h-11"><span className="font-mono">{lang || t('chat-code')}</span><CopyButton text={text} /></div>
      <pre dir="ltr" className="m-0 max-h-96 overflow-auto p-3 font-mono text-xs leading-5">
        {html ? <code dangerouslySetInnerHTML={{ __html: html }} /> : <code>{text}</code>}
      </pre>
    </div>
  );
}

function BlockView({ block }: { block: Block }) {
  switch (block.kind) {
    case 'heading': return <p className="m-0 mt-1 text-[0.9375rem] font-semibold text-ink"><Inlines parts={block.inlines} /></p>;
    case 'list': {
      const items = block.items.map((it, i) => <li key={i} className="whitespace-pre-wrap"><Inlines parts={it} /></li>);
      return block.ordered ? <ol start={block.start} className="m-0 grid list-decimal gap-0.5 ps-5 marker:text-muted">{items}</ol> : <ul className="m-0 grid list-disc gap-0.5 ps-5 marker:text-muted">{items}</ul>;
    }
    case 'quote': return <blockquote className="m-0 border-s-2 border-line ps-3 whitespace-pre-wrap text-muted"><Inlines parts={block.inlines} /></blockquote>;
    case 'code': return <CodeBlock lang={block.lang} text={block.text} />;
    case 'table': return block.head && block.rows ? (
      // A link in its last row reaches past it to a finger on a touch screen (#1073): that must not make the table scroll up and down.
      <div className="overflow-x-auto rounded-s border border-line pointer-coarse:overflow-y-hidden">
        <table className="w-full border-collapse text-start text-[0.8125rem]">
          <thead className="bg-surface-1"><tr>{block.head.map((c, i) => <th key={i} className="border-b border-line px-3 py-1.5 text-start font-semibold text-ink"><Inlines parts={c} /></th>)}</tr></thead>
          <tbody>{block.rows.map((r, i) => <tr key={i} className="border-t border-line first:border-t-0">{r.map((c, j) => <td key={j} className="px-3 py-1.5 align-top"><Inlines parts={c} /></td>)}</tr>)}</tbody>
        </table>
      </div>
    ) : <pre dir="ltr" className="m-0 overflow-x-auto font-mono text-xs leading-5">{block.text}</pre>;
    default: return <p className="m-0 break-words whitespace-pre-wrap"><Inlines parts={block.inlines} /></p>;
  }
}

function fullStamp(atMs: number): string | undefined {
  return atMs > 0 ? new Date(atMs).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) : undefined;
}
const clock = (atMs: number): string => new Date(atMs).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });

// What the agent said, with the Markdown it writes drawn as such. It grows in place while it streams: the parsed blocks of text that did
// not change keep their elements, so nothing above the end moves or flickers.
const Reply = memo(function Reply({ text, atMs }: { text: string; atMs: number }) {
  const parsed = useMemo(() => blocks(text), [text]);
  return (
    <div className="group/msg relative isolate max-w-3xl" title={fullStamp(atMs)} data-role="agent">
      <span className="sr-only">{t('chat-agent')}: </span>
      <div className="grid gap-2 leading-relaxed">{parsed.map((b, i) => <BlockView key={i} block={b} />)}</div>
      {/* On a touch screen it is always shown at full size, so it sits under the reply rather than over its top corner, where it would
          cover a code block's own Copy or the end of the first line (#1073). */}
      <CopyButton text={text} label="chat-copy-reply" className={`absolute end-0 top-0 pointer-coarse:static pointer-coarse:ms-auto pointer-coarse:grid ${HOVER}`} />
    </div>
  );
});

// What a person said: a tinted bubble on the right with its time under it.
// ADR-0090: a prompt that waits behind the turn that runs says "Waiting", and one the node said it will not send says "Not sent"
// (`state`); either way it is drawn quieter, as what it is.
export const UserText = memo(function UserText({ line, state }: { line: ChatLine; state?: 'waiting' | 'not-sent' }) {
  return (
    <div className={`group/msg relative ms-auto max-w-[88%] rounded-l rounded-ee-xs border border-(--line-accent) bg-tint-accent px-3.5 py-2.5 max-md:max-w-full ${state ? 'border-dashed opacity-80' : ''}`} title={fullStamp(line.atMs)} data-role="user" data-queued={state}>
      <div className="break-words whitespace-pre-wrap"><span className="sr-only">{t('chat-you')}: </span>{visible(line.text)}</div>
      <div className="mt-1 flex items-center justify-end gap-2 text-xs text-muted">
        {state && <span className="ide-chip h-5 px-1.5">{t(state === 'waiting' ? 'chat-queued-mark' : 'chat-not-sent-mark')}</span>}
        {line.atMs > 0 && <span>{clock(line.atMs)}</span>}
        <CopyButton text={line.text} label="chat-copy-message" className={HOVER} />
      </div>
    </div>
  );
});

// An agent's turn: its avatar (the kit's agent avatar, in the agent's own colour, as on its tile and rows), its name, the model it runs and when
// it began, and everything it said and did beside them.
export function Turn({ name, accent, kind, model, atMs, children }: { name: string; accent?: string; kind?: string; model?: string; atMs: number; children: ReactNode }) {
  return (
    <section className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-x-3 max-md:grid-cols-[minmax(0,1fr)]" data-turn aria-label={name}>
      <span className="max-md:hidden" data-turn-avatar><AgentAvatar accent={accent ?? 'var(--agent-slate)'} kind={kind || name} /></span>
      <div className="mb-2 flex min-w-0 items-center gap-2 text-xs text-muted">
        <b className="truncate text-[0.8125rem] font-semibold text-ink">{name}</b>
        {model && <span className="truncate">{visible(model)}</span>}
        {atMs > 0 && <span className="flex-none">{clock(atMs)}</span>}
      </div>
      <div className="col-start-2 flex min-w-0 flex-col gap-2.5 max-md:col-start-1">{children}</div>
    </section>
  );
}

// What the agent thought, folded: "Thought for 9 s" and, opened, the text; "Thinking…" while it still thinks (ADR-0090). Collapsed by
// default and only opened by the reader. The text is untrusted (a model wrote it): plain text, never HTML, and it goes nowhere else.
export function Thought({ text, seconds, live = false }: { text: string; seconds?: number; live?: boolean }) {
  return (
    <details className="group max-w-3xl rounded-m border border-dashed border-line" data-thought={live ? 'live' : ''}>
      <summary className="flex min-h-9 pointer-coarse:min-h-11 cursor-pointer list-none items-center gap-2.5 px-3 py-1.5 text-[0.8125rem] text-muted italic hover:bg-surface-2 [&::-webkit-details-marker]:hidden">
        <Icon name="think" className="size-3.5" />
        <span>{live ? t('chat-thinking') : seconds && seconds > 0 ? t('chat-thought-for', { time: preciseDuration(seconds * 1000) }) : t('chat-act-think')}</span>
        <Icon name="chevron" className="ms-auto size-3 transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <div className="px-3.5 pt-0.5 pb-3 ps-9 text-[0.8125rem] break-words whitespace-pre-wrap text-muted italic">{visible(text)}</div>
    </details>
  );
}

// A title in backticks is a command or a name: it is shown as code. Inline backticks become inline code.
function inlineTitle(title: string): Inline[] {
  return /^`[^`]+`$/.test(title) ? [{ kind: 'code', text: title.slice(1, -1) }] : inlines(title);
}

const TONE: Record<StepTone, string> = { edit: 'text-accent', run: 'text-working', plain: 'text-muted' };
const CHEVRON = 'size-3 flex-none text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!';

// The lines of a change: a hunk bar, then each line with its number; added and deleted lines say so by glyph and bar as well as colour.
const DIFF_CAP = 200;
function DiffLines({ lines: all, fallback }: { lines: ReturnType<typeof parseHunks>['lines']; fallback: string }) {
  const [full, setFull] = useState(false);
  const lines = full ? all : all.slice(0, DIFF_CAP);
  if (all.length === 0) return <pre dir="ltr" className="m-0 max-h-60 overflow-auto bg-surface-0 p-2 font-mono text-xs">{fallback}</pre>;
  return (
    <pre dir="ltr" className="m-0 max-h-80 overflow-auto bg-surface-0 font-mono text-xs leading-5">
      {lines.map((l, i) => l.kind === 'hunk'
        ? <div key={i} className="bg-surface-2 px-3.5 py-1 text-muted">{l.text}</div>
        : (
          <div key={i} className={`grid grid-cols-[3rem_minmax(0,1fr)] ${l.kind === 'add' ? 'bg-finished/10 diff-bar-add' : l.kind === 'del' ? 'bg-removed/10 diff-bar-del' : ''}`}>
            <span aria-hidden className="pe-2.5 text-end text-muted select-none">{l.kind === 'del' ? l.oldNo : l.newNo}</span>
            <span className="whitespace-pre"><span aria-hidden className={l.kind === 'add' ? 'text-finished' : l.kind === 'del' ? 'text-removed' : ''}>{l.kind === 'add' ? '+ ' : l.kind === 'del' ? '− ' : '  '}</span>{l.text || ' '}</span>
          </div>
        ))}
      {lines.length < all.length && <button type="button" className="block w-full border-t border-line bg-surface-2 px-3.5 py-1.5 text-start text-muted hover:text-ink pointer-coarse:min-h-11" onClick={() => setFull(true)}>{t('chat-diff-show-all', { count: all.length })}</button>}
    </pre>
  );
}

const Counts = ({ added, removed }: { added: number; removed: number }) => (
  (added > 0 || removed > 0) ? <span className="flex-none font-mono text-xs"><span className="text-finished">+{added}</span> <span className="text-removed">−{removed}</span></span> : null
);

// Where a call is in its life, as a mark: a spinner while it runs (a still ring when motion is reduced), a hollow ring while it waits, a tick
// when it is done and a cross when it failed. Said in words for a screen reader. Only drawn when the node sends a status (ADR-0090).
// #908: a call whose turn is over but that never said it finished (Stop turn, or the agent moved on) is `stopped`: a still square.
type ShownStatus = CallStatus | 'stopped';
const MARKS: Record<ShownStatus, string> = { pending: 'tool-status-pending', in_progress: 'tool-status-running', completed: 'tool-status-done', failed: 'tool-status-failed', stopped: 'tool-status-stopped' };
/** What a call's mark says: its own status, or `stopped` when its turn is over and it still says it runs or waits. */
export const shownStatus = (status: CallStatus, turnOver: boolean): ShownStatus => (turnOver && (status === 'in_progress' || status === 'pending') ? 'stopped' : status);
export function CallMark({ status }: { status: ShownStatus }) {
  const label = t(MARKS[status]);
  const box = 'grid size-4 flex-none place-items-center';
  switch (status) {
    case 'in_progress': return <span role="img" aria-label={label} title={label} className={box}><span className="size-3.5 animate-spin rounded-full border-2 border-line border-t-working motion-reduce:animate-none" /></span>;
    case 'pending': return <span role="img" aria-label={label} title={label} className={box}><span className="size-3 rounded-full border-2 border-dashed border-muted" /></span>;
    case 'completed': return <span role="img" aria-label={label} title={label} className={`${box} text-finished`}><Icon name="check" className="size-3.5" /></span>;
    case 'failed': return <span role="img" aria-label={label} title={label} className={`${box} text-removed`}><Icon name="close" className="size-3.5" /></span>;
    case 'stopped': return <span role="img" aria-label={label} title={label} className={`${box} text-muted`}><svg aria-hidden viewBox="0 0 16 16" className="size-3 fill-current"><rect x="3" y="3" width="10" height="10" rx="1.5" /></svg></span>;
  }
}

// What a call printed: the last lines (the end of a run is what matters), the rest on request. Text from a tool: plain text.
const OUTPUT_TAIL = 12;
function OutputBlock({ text, cut }: { text: string; cut?: boolean }) {
  const [full, setFull] = useState(false);
  const tail = useMemo(() => outputTail(text, full ? Number.MAX_SAFE_INTEGER : OUTPUT_TAIL), [text, full]);
  return (
    <div data-output>
      <pre dir="ltr" className="m-0 max-h-60 overflow-auto font-mono text-xs leading-5 break-all whitespace-pre-wrap text-muted">{tail.text}</pre>
      {tail.hidden > 0 && <button type="button" className="mt-1 text-xs text-muted underline underline-offset-2 hover:text-ink pointer-coarse:min-h-11" onClick={() => setFull(true)}>{t('chat-diff-show-all', { count: tail.hidden + OUTPUT_TAIL })}</button>}
      {cut && <p className="m-0 mt-1 text-xs text-muted">{t('tool-output-cut')}</p>}
    </div>
  );
}

// The files a call touched, as chips (a path the node gave; never a link the window follows). The first two, then how many more.
const baseName = (p: string): string => p.slice(p.replace(/\\/g, '/').lastIndexOf('/') + 1) || p;
function PlaceChips({ places, skip }: { places: readonly { path: string; line: number }[]; skip?: string }) {
  const list = places.filter((p) => p.path !== skip);
  if (list.length === 0) return null;
  return <>
    {list.slice(0, 2).map((p, i) => <code key={`${p.path}:${p.line}`} dir="ltr" title={visible(p.line > 0 ? `${p.path}:${p.line}` : p.path)} data-place className={`min-w-0 max-w-40 truncate rounded-xs border border-line bg-surface-2 px-2 py-0.5 font-mono text-xs text-muted ${i > 0 ? 'max-md:hidden' : ''}`}>{visible(baseName(p.path))}{p.line > 0 ? `:${p.line}` : ''}</code>)}
    {list.length > 2 && <span className="flex-none text-xs text-muted max-md:hidden">+{list.length - 2}</span>}
  </>;
}

// One thing the agent did: an icon tile, a sentence, the file or command it was about as a chip, and the full input folded underneath.
// An edit that has its diff shows the change inside the row. With the node's data (ADR-0090) the row also has the call's status mark,
// how long it took, the files it touched and, folded, what it printed.
const ToolStep = memo(function ToolStep({ line, diff, ended = false }: { line: ChatLine; diff?: ChatLine; ended?: boolean }) {
  const v = useMemo(() => toolView(line.text), [line.text]);
  const d = useMemo(() => (diff ? parseHunks(diff.text) : undefined), [diff]);
  const action = actionOf(line, v.action);
  const took = callMs(line);
  const showsSubject = v.subject && !titleSays(v.title, v.subject);
  // A command that is running shows its output from the start (read once; afterwards the fold is the reader's).
  const [runOpen] = useState(() => line.status === 'in_progress' && !ended && !!line.output);
  const status = line.status && shownStatus(line.status, ended);
  const heading = v.raw
    ? <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted">{visible(v.raw)}</span>
    : <>
      <span className="isolate min-w-0 shrink-0 font-medium text-ink">{v.title ? <Inlines parts={inlineTitle(visible(v.title))} /> : t(`tool-${action}`)}</span>
      {showsSubject && <code dir="ltr" className="min-w-0 truncate rounded-xs border border-line bg-surface-2 px-2 py-0.5 font-mono text-xs text-muted">{visible(v.subject)}</code>}
      {line.locations && <PlaceChips places={line.locations} skip={showsSubject ? v.subject : undefined} />}
    </>;
  const row = 'flex min-h-9 pointer-coarse:min-h-11 min-w-0 items-center gap-2.5 px-3 py-1.5 text-[0.8125rem]';
  const icon = <span aria-hidden className={`grid size-5.5 flex-none place-items-center rounded-xs border border-line bg-surface-2 ${TONE[stepTone(action)]}`}><Icon name={ICONS[action]} className="size-3.5" /></span>;
  const tail = <>
    {took !== undefined && <span className="flex-none text-xs text-muted tabular-nums" data-took>{preciseDuration(took)}</span>}
    {status && <CallMark status={status} />}
  </>;
  if (v.details.length === 0 && !v.raw && !d && !line.output) return <div className={row} title={fullStamp(line.atMs)} data-status={status}>{icon}{heading}<span className="flex-1" />{tail}</div>;
  return (
    <details className="group" open={(d && d.lines.length <= 24) || runOpen ? true : undefined} data-diff-path={diff?.path} data-status={status}>
      <summary className={`${row} cursor-pointer list-none hover:bg-surface-2 [&::-webkit-details-marker]:hidden`} title={fullStamp(line.atMs)}>
        {icon}{heading}
        <span className="flex-1" />
        {d && <Counts added={d.added} removed={d.removed} />}
        {tail}
        <Icon name="chevron" className={CHEVRON} />
      </summary>
      {d && diff ? <div className="border-t border-line"><DiffLines lines={d.lines} fallback={diff.text} /></div> : (
        <div className="border-t border-line bg-surface-0 px-3.5 py-2">
          {v.raw ? <pre dir="ltr" className="m-0 max-h-60 overflow-auto font-mono text-xs break-all whitespace-pre-wrap">{visible(v.raw)}</pre> : <>
            {action === 'run' && v.subject && <pre dir="ltr" className="m-0 mb-2 max-h-52 overflow-auto font-mono text-xs leading-6 break-all whitespace-pre-wrap text-muted"><span aria-hidden className="select-none">$ </span>{visible(v.subject)}</pre>}
            {line.output && <div className={v.details.length ? 'mb-2 border-b border-line pb-2' : ''}><OutputBlock text={line.output} cut={line.outputCut} /></div>}
            <dl dir="ltr" className="m-0 grid max-h-72 grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 overflow-auto font-mono text-xs">
              {v.details.map(([k, val], i) => <div key={i} className="contents">
                <dt className="text-muted">{visible(k)}</dt>
                <dd className="m-0 break-words whitespace-pre-wrap">{visible(val)}</dd>
              </div>)}
            </dl>
          </>}
        </div>
      )}
    </details>
  );
});

// Tool calls in a row are one bordered group, with a header that says how many steps and, when the entries carry times, how long they took;
// then what kinds. `open` is only where a group starts (the newest when it appears): afterwards the reader owns the fold, so a newer
// group arriving never folds one under them. The data has no status per call, so a flag is the agent's own state (it waits on you, or
// ended with an error) on its newest group: a group that was flagged opens and stays open, and is never hidden in a fold meanwhile.
const SHOWN_KINDS = 3;
// `ended` (#908): the turn of this group is over, so a call in it that still says it runs is drawn stopped and nothing spins.
export const ToolSteps = memo(function ToolSteps({ lines, running = false, ended = false, flag, open = false, diffs }: { lines: ChatLine[]; running?: boolean; ended?: boolean; flag?: GroupFlag; open?: boolean; diffs?: ReadonlyMap<number, ChatLine> }) {
  const sum = useMemo(() => stepsSummary(lines), [lines]);
  // With the node's per-call status (ADR-0090) the spinner follows the calls themselves; without it, the agent's own state.
  const rich = lines.some((l) => l.status !== undefined);
  const spinning = rich ? !ended && lines.some((l) => l.status === 'in_progress') : running;
  const failed = lines.filter((l) => l.status === 'failed').length;
  const kinds = sum.actions.slice(0, SHOWN_KINDS).map((a) => t(`chat-act-${a.action}`, { count: a.count }));
  const more = sum.actions.length - SHOWN_KINDS;
  // Uncontrolled after the first draw: `open` is read once, and a flag keeps the group open from the moment it is set.
  const [start] = useState(open);
  const [flagged, setFlagged] = useState(!!flag);
  if (flag && !flagged) setFlagged(true);
  return (
    <details open={start || flagged || flag ? true : undefined} className={`group max-w-3xl overflow-hidden rounded-m border bg-surface-1 ${flag === 'needs-you' ? 'border-needs-you/70' : flag ? 'border-danger/70' : 'border-line'}`} data-steps={sum.total} data-flag={flag}>
      <summary className="flex min-h-9 pointer-coarse:min-h-11 min-w-0 cursor-pointer list-none items-center gap-2 bg-surface-2 px-3 py-1.5 text-xs text-muted hover:text-ink [&::-webkit-details-marker]:hidden" aria-label={`${t('chat-steps-open')}: ${t('chat-steps-count', { count: sum.total })}`}>
        {spinning && !flag
          ? <span role="img" aria-label={t('chat-working')} className="size-3.5 flex-none animate-spin rounded-full border-2 border-line border-t-working motion-reduce:animate-none" />
          : <Icon name="board" className="size-3.5 flex-none" />}
        <span className="flex-none font-medium text-ink">{t('chat-steps-count', { count: sum.total })}{sum.spanMs ? ` · ${preciseDuration(sum.spanMs)}` : ''}</span>
        <span className="min-w-0 flex-1 truncate max-md:hidden">{kinds.join(' · ')}{more > 0 ? ` · +${more}` : ''}{sum.last ? ` — ${visible(sum.last)}` : ''}</span>
        <span className="flex-1 md:hidden" />
        {flag && <span className={`flex flex-none items-center gap-1 text-xs font-medium ${flag === 'needs-you' ? 'text-needs-you' : 'text-danger'}`}><Icon name="warn" className="size-3" />{flag === 'call-failed' ? t('chat-steps-call-failed', { count: failed }) : t(flag === 'failed' ? 'chat-steps-failed' : 'chat-steps-needs-you')}</span>}
        <Icon name="chevron" className={CHEVRON} />
      </summary>
      <div className="grid grid-cols-1 divide-y divide-line border-t border-line">
        {lines.map((l) => <ToolStep key={l.id} line={l} diff={diffs?.get(l.id)} ended={ended} />)}
      </div>
    </details>
  );
  // An entry that changed in place is a new object under the same id (ADR-0090): compare the lines themselves, not only how many.
}, (a, b) => a.running === b.running && a.ended === b.ended && a.flag === b.flag && a.lines.length === b.lines.length && a.lines.every((l, i) => l === b.lines[i]) && (a.diffs?.size ?? 0) === (b.diffs?.size ?? 0));

// A file the agent changed that no step claimed: a card with its name and how much changed, the lines numbered and coloured; a long
// change starts folded.
const DiffEntry = memo(function DiffEntry({ line }: { line: ChatLine }) {
  const d = useMemo(() => parseHunks(line.text), [line.text]);
  const cut = line.path.replace(/\\/g, '/').lastIndexOf('/');
  const name = line.path.slice(cut + 1);
  const dir = cut > 0 ? line.path.slice(0, cut + 1) : '';
  return (
    <details open={d.lines.length <= 24} className="group max-w-3xl overflow-hidden rounded-m border border-line bg-surface-1" data-diff-path={line.path}>
      <summary className="flex min-h-9 pointer-coarse:min-h-11 min-w-0 cursor-pointer list-none items-center gap-2.5 px-3 py-1.5 text-[0.8125rem] hover:bg-surface-2 [&::-webkit-details-marker]:hidden" title={fullStamp(line.atMs)}>
        <span aria-hidden className="grid size-5.5 flex-none place-items-center rounded-xs border border-line bg-surface-2 text-accent"><Icon name="pencil" className="size-3.5" /></span>
        <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs"><span className="text-muted">{visible(dir)}</span><span className="font-semibold text-ink">{visible(name)}</span></span>
        <Counts added={d.added} removed={d.removed} />
        <Icon name="chevron" className={CHEVRON} />
      </summary>
      <div className="border-t border-line"><DiffLines lines={d.lines} fallback={line.text} /></div>
    </details>
  );
});

// The files this conversation changed, in one place: the count and totals, each file once as a chip (a click goes to its change in the
// conversation), and the way into the review screen when the window has one.
const SHOWN_FILES = 6;
export function FilesChanged({ lines, onGo, onReview }: { lines: readonly ChatLine[]; onGo?: (path: string) => void; onReview?: () => void }) {
  const files = useMemo(() => changedFiles(lines), [lines]);
  if (files.length === 0) return null;
  const added = files.reduce((n, f) => n + f.added, 0);
  const removed = files.reduce((n, f) => n + f.removed, 0);
  const base = (p: string) => p.slice(p.replace(/\\/g, '/').lastIndexOf('/') + 1);
  return (
    <div className="flex w-full flex-none flex-wrap items-center gap-x-3 gap-y-2 rounded-m border border-line bg-surface-1 px-3.5 py-2.5 pointer-coarse:gap-y-3" data-files-changed>
      <Icon name="diff" className="size-4 flex-none text-muted" />
      <b className="font-semibold text-ink">{t('chat-files-title', { count: files.length })}</b>
      <Counts added={added} removed={removed} />
      <ul className="m-0 flex min-w-0 list-none flex-wrap gap-1.5 p-0 pointer-coarse:gap-y-5">
        {files.slice(0, SHOWN_FILES).map((f) => <li key={f.path} className="min-w-0">
          <button type="button" dir="ltr" title={visible(f.path)} className="tap-target inline-flex h-6 max-w-full items-center rounded-xs border border-line bg-surface-2 px-2 font-mono text-xs text-muted hover:text-ink" onClick={() => onGo?.(f.path)}>
            <span className="truncate">{visible(files.some((o) => o !== f && base(o.path) === base(f.path)) ? f.path : base(f.path))}</span>
          </button>
        </li>)}
        {files.length > SHOWN_FILES && <li className="muted self-center text-xs">{t('chat-files-more', { count: files.length - SHOWN_FILES })}</li>}
      </ul>
      <span className="flex-1" />
      {onReview && <button type="button" className="button px-3 py-1 text-[0.8125rem]" onClick={onReview}>{t('chat-review-changes')}</button>}
    </div>
  );
}

// The agent's plan as a card with a checklist. Agents send the whole plan again each time it changes: the latest is shown, earlier copies
// stay folded. The node keeps the steps' text only (no done or not done), so by default every step is a plain numbered row and no
// progress bar is drawn: `status` (one per step) turns on the ticks and the bar once the node reports it (ADR-0090). Nothing is ever faked.
export function PlanCard({ steps, status, label }: { steps: readonly string[]; status?: readonly (StepStatus | undefined)[]; label: string }) {
  const progress = status ? planProgress(status) : undefined;
  // The step being worked on: marked for a screen reader (`aria-current`) and in weight, as well as by its dot.
  const now = status ? currentStep(status) : undefined;
  return (
    <div className="isolate max-w-3xl rounded-m border border-line bg-surface-1 px-3.5 py-3 text-[0.8125rem]" data-plan>
      <div className="mb-2 flex items-center gap-2 font-semibold text-ink">
        <Icon name="board" className="size-4 text-accent" />{label}
        {progress && <small className="font-normal text-muted">{t('chat-plan-progress', { done: progress.done, total: progress.total })}</small>}
      </div>
      {progress && <div role="progressbar" aria-label={t('chat-plan-label')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress.percent} className="mb-2.5 h-1 overflow-hidden rounded-full bg-surface-2"><div className="h-full rounded-full bg-finished" style={{ width: `${progress.percent}%` }} /></div>}
      <ol className="m-0 grid list-none gap-1.5 p-0">{steps.map((p, i) => {
        const s = status?.[i];
        return (
          <li key={i} aria-current={now === i ? 'step' : undefined} className={`flex items-start gap-2.5 ${s === 'completed' ? 'text-muted line-through' : s === 'in_progress' ? 'font-medium text-ink' : now === i ? 'font-medium text-ink' : s ? 'text-muted' : ''}`} data-status={s} data-current={now === i ? '' : undefined}>
            <span aria-hidden className={`mt-px grid size-5 flex-none place-items-center rounded-xs border text-xs ${s === 'completed' ? 'border-finished bg-finished text-on-accent' : s === 'in_progress' ? 'border-working text-working' : 'border-line bg-surface-0 text-muted'}`}>
              {s === 'completed' ? <Icon name="check" className="size-3" /> : s === 'in_progress' ? <span className="size-1.5 rounded-full bg-working" /> : s ? null : i + 1}
            </span>
            <span className="min-w-0 break-words"><Inlines parts={inlineTitle(p)} /></span>
          </li>
        );
      })}</ol>
    </div>
  );
}

// A plan: the node's steps with their status when it sends them (ADR-0090: a tick for done, a dot for the step in hand, a bar for how
// far), the joined lines of an old node otherwise. The same entry is sent again each time a step changes, and replaces this one.
const PlanEntry = memo(function PlanEntry({ line, latest }: { line: ChatLine; latest: boolean }) {
  const list = planSteps(line);
  const steps = list.map((s) => s.text);
  const status = list.some((s) => s.status !== undefined) ? list.map((s) => s.status) : undefined;
  if (!latest) {
    return (
      <details className="max-w-3xl text-xs text-muted">
        <summary className="cursor-pointer pointer-coarse:py-3.5">{t('chat-plan-earlier', { count: steps.length })}</summary>
        <div className="mt-1 ps-1"><PlanCard steps={steps} status={status} label={t('chat-plan-steps', { count: steps.length })} /></div>
      </details>
    );
  }
  return <div title={fullStamp(line.atMs)}><PlanCard steps={steps} status={status} label={t('chat-plan-steps', { count: steps.length })} /></div>;
});

// A request the agent made for permission, as a record in the conversation (ADR-0090). It is a VIEW of the approval: it says how that
// stands and never answers it; Allow and Deny stay in the pinned bar. While it waits, "Show the request" takes the reader there.
const OUTCOME_ICON: Record<NonNullable<ChatLine['outcome']>, IconName> = { pending: 'lock', approved: 'check', denied: 'close', cancelled: 'step' };
const OUTCOME_TONE: Record<NonNullable<ChatLine['outcome']>, string> = { pending: 'text-needs-you', approved: 'text-finished', denied: 'text-removed', cancelled: 'text-muted' };
const PermissionRecord = memo(function PermissionRecord({ line, onShow }: { line: ChatLine; onShow?: () => void }) {
  const outcome = line.outcome ?? 'pending';
  return (
    <div role="note" className={`flex max-w-3xl flex-wrap items-center gap-x-2.5 gap-y-1 rounded-m border px-3 py-1.5 text-[0.8125rem] ${outcome === 'pending' ? 'border-(--line-needs) bg-tint-needs' : 'border-line bg-surface-1'}`} data-permission={outcome} title={fullStamp(line.atMs)}>
      <span aria-hidden className={`grid size-5 flex-none place-items-center ${OUTCOME_TONE[outcome]}`}><Icon name={OUTCOME_ICON[outcome]} className="size-3.5" /></span>
      <span className="min-w-0 flex-1 break-words">
        {/* ADR-0101: an approval answered "always" says so (the agent remembers it). */}
        <span className="font-medium text-ink">{outcome === 'approved' && line.always ? t(alwaysText('permission-approved-always')) : t(`permission-${outcome}`)}</span>
        {line.text && <> <code dir="ltr" className="rounded-xs border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-xs text-muted">{visible(line.text)}</code></>}
      </span>
      {outcome === 'pending' && onShow && <button type="button" className="tap-target text-xs text-muted underline underline-offset-2 hover:text-ink" onClick={onShow}>{t('permission-show')}</button>}
    </div>
  );
});

// Where the day changes: a line with the date, in the reader's own format.
function DateRule({ atMs }: { atMs: number }) {
  const label = new Date(atMs).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' });
  return <div role="separator" aria-label={label} className="flex items-center gap-3 py-1 text-xs text-muted"><span className="h-px flex-1 bg-line" /><span>{label}</span><span className="h-px flex-1 bg-line" /></div>;
}

export function Item({ item, running, ended, flag, open, diffs, notice, queued, onShowRequest }: { item: TranscriptItem; running?: boolean; ended?: boolean; flag?: GroupFlag; open?: boolean; diffs?: ReadonlyMap<number, ChatLine>; notice: (l: ChatLine) => ReactNode; queued?: 'waiting' | 'not-sent'; onShowRequest?: () => void }) {
  switch (item.kind) {
    case 'date': return <DateRule atMs={item.atMs} />;
    case 'user': return <UserText line={item.line} state={queued} />;
    case 'reply': return <Reply text={item.text} atMs={item.atMs} />;
    case 'thought': return <Thought text={item.text} seconds={Math.round((item.endMs - item.atMs) / 1000)} live={running} />;
    case 'permission': return <PermissionRecord line={item.line} onShow={onShowRequest} />;
    case 'steps': return <ToolSteps lines={item.lines} running={running} ended={ended} flag={flag} open={open} diffs={diffs} />;
    case 'plan': return <PlanEntry line={item.line} latest={item.latest} />;
    case 'diff': return <DiffEntry line={item.line} />;
    case 'notice': return <>{notice(item.line)}</>;
  }
}
