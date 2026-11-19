import { memo, useMemo, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { ChatLine } from '../net/chat';
import { parseHunks } from '../net/diff';
import { blocks, inlines, type Block, type Inline } from '../net/markdown';
import { titleSays, toolView, type ToolAction } from '../net/tool';
import { changedFiles, stepsSummary, type GroupFlag, type TranscriptItem } from '../net/transcript';
import { initial, planProgress, stepTone, type StepStatus, type StepTone } from '../net/chatView';
import { preciseDuration } from '../net/time';
import { useHighlightedCode } from '../highlight/useHighlight';
import { Icon, type IconName } from './Icon';

// How one conversation entry looks. Everything here was written by an agent or a person: it is drawn as React text, never as HTML
// (the one exception is the syntax highlighter's own output, which is escaped text inside <span class> tags).

const ICONS: Record<ToolAction, IconName> = {
  read: 'files', edit: 'pencil', run: 'terminal', search: 'search', fetch: 'globe', delete: 'trash', move: 'move', think: 'think', other: 'step',
};

function Inlines({ parts }: { parts: Inline[] }) {
  return <>{parts.map((p, i) => p.kind === 'code'
    ? <code key={i} className="rounded-xs bg-surface-2 px-1 py-px font-mono text-[0.92em]">{p.text}</code>
    : p.kind === 'strong' ? <strong key={i} className="font-semibold text-ink">{p.text}</strong>
      // `href` passed `safeLink` when it was read: http or https only. It opens without access to this window.
      : p.kind === 'link' && p.href ? <a key={i} href={p.href} target="_blank" rel="noopener noreferrer" className="text-accent underline underline-offset-2 hover:brightness-110">{p.text}</a>
        : <span key={i}>{p.text}</span>)}</>;
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
      <div className="flex h-7 items-center justify-between gap-2 border-b border-line ps-3 pe-1 text-xs text-muted"><span className="font-mono">{lang || t('chat-code')}</span><CopyButton text={text} /></div>
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
      <div className="overflow-x-auto rounded-s border border-line">
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
    <div className="group/msg relative max-w-3xl" title={fullStamp(atMs)} data-role="agent">
      <span className="sr-only">{t('chat-agent')}: </span>
      <div className="grid gap-2 leading-relaxed">{parsed.map((b, i) => <BlockView key={i} block={b} />)}</div>
      <CopyButton text={text} label="chat-copy-reply" className={`absolute end-0 top-0 ${HOVER}`} />
    </div>
  );
});

// What a person said: a tinted bubble on the right with its time under it.
export const UserText = memo(function UserText({ line }: { line: ChatLine }) {
  return (
    <div className="group/msg relative ms-auto max-w-[88%] rounded-l rounded-ee-xs border border-accent/30 bg-accent/15 px-3.5 py-2.5 max-md:max-w-full" title={fullStamp(line.atMs)} data-role="user">
      <div className="break-words whitespace-pre-wrap"><span className="sr-only">{t('chat-you')}: </span>{line.text}</div>
      {line.atMs > 0 && <div className="mt-1 flex justify-end text-xs text-muted">{clock(line.atMs)}</div>}
      <CopyButton text={line.text} label="chat-copy-message" className={`absolute -start-7 top-1 ${HOVER}`} />
    </div>
  );
});

// An agent's turn: its avatar tile, its name, the model it runs and when it began, and everything it said and did beside them.
export function Turn({ name, model, atMs, children }: { name: string; model?: string; atMs: number; children: ReactNode }) {
  return (
    <section className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-x-3 max-md:grid-cols-[minmax(0,1fr)]" data-turn aria-label={name}>
      <div aria-hidden className="grid size-7 place-items-center rounded-m border border-line bg-surface-1 text-xs font-semibold text-accent max-md:hidden">{initial(name)}</div>
      <div className="mb-2 flex min-w-0 items-center gap-2 text-xs text-muted">
        <b className="truncate text-[0.8125rem] font-semibold text-ink">{name}</b>
        {model && <span className="truncate">{model}</span>}
        {atMs > 0 && <span className="flex-none">{clock(atMs)}</span>}
      </div>
      <div className="col-start-2 flex min-w-0 flex-col gap-2.5 max-md:col-start-1">{children}</div>
    </section>
  );
}

// What the agent thought, folded: "Thought for 9 s" and, opened, the text. The node does not send thinking yet (ADR-0090), so nothing
// draws this today; it is here so that the entry kind, when it arrives, needs only a line in `Item`. The text is untrusted: plain text.
export function Thought({ text, seconds }: { text: string; seconds?: number }) {
  return (
    <details className="group max-w-3xl rounded-m border border-dashed border-line" data-thought>
      <summary className="flex min-h-9 cursor-pointer list-none items-center gap-2.5 px-3 py-1.5 text-[0.8125rem] text-muted italic hover:bg-surface-2 [&::-webkit-details-marker]:hidden">
        <Icon name="think" className="size-3.5" />
        <span>{seconds && seconds > 0 ? t('chat-thought-for', { time: preciseDuration(seconds * 1000) }) : t('chat-act-think')}</span>
        <Icon name="chevron" className="ms-auto size-3 transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <div className="px-3.5 pt-0.5 pb-3 ps-9 text-[0.8125rem] break-words whitespace-pre-wrap text-muted italic">{text}</div>
    </details>
  );
}

// A title in backticks is a command or a name: it is shown as code. Inline backticks become inline code.
function inlineTitle(title: string): Inline[] {
  return /^`[^`]+`$/.test(title) ? [{ kind: 'code', text: title.slice(1, -1) }] : inlines(title);
}

const TONE: Record<StepTone, string> = { edit: 'text-accent', run: 'text-needs-you', plain: 'text-muted' };
const CHEVRON = 'size-3 flex-none text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!';

// The lines of a change: a hunk bar, then each line with its number; added and deleted lines say so by glyph and bar as well as colour.
function DiffLines({ lines, fallback }: { lines: ReturnType<typeof parseHunks>['lines']; fallback: string }) {
  if (lines.length === 0) return <pre dir="ltr" className="m-0 max-h-60 overflow-auto bg-surface-0 p-2 font-mono text-xs">{fallback}</pre>;
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
    </pre>
  );
}

const Counts = ({ added, removed }: { added: number; removed: number }) => (
  (added > 0 || removed > 0) ? <span className="flex-none font-mono text-xs"><span className="text-finished">+{added}</span> <span className="text-removed">−{removed}</span></span> : null
);

// One thing the agent did: an icon tile, a sentence, the file or command it was about as a chip, and the full input folded underneath.
// An edit that has its diff shows the change inside the row.
const ToolStep = memo(function ToolStep({ line, diff }: { line: ChatLine; diff?: ChatLine }) {
  const v = useMemo(() => toolView(line.text), [line.text]);
  const d = useMemo(() => (diff ? parseHunks(diff.text) : undefined), [diff]);
  const showsSubject = v.subject && !titleSays(v.title, v.subject);
  const heading = v.raw
    ? <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted">{v.raw}</span>
    : <>
      <span className="min-w-0 shrink-0 font-medium text-ink">{v.title ? <Inlines parts={inlineTitle(v.title)} /> : t(`tool-${v.action}`)}</span>
      {showsSubject && <code dir="ltr" className="min-w-0 truncate rounded-xs border border-line bg-surface-2 px-2 py-0.5 font-mono text-xs text-muted">{v.subject}</code>}
    </>;
  const row = 'flex min-h-9 min-w-0 items-center gap-2.5 px-3 py-1.5 text-[0.8125rem]';
  const icon = <span aria-hidden className={`grid size-5.5 flex-none place-items-center rounded-xs border border-line bg-surface-2 ${TONE[stepTone(v.action)]}`}><Icon name={ICONS[v.action]} className="size-3.5" /></span>;
  if (v.details.length === 0 && !v.raw && !d) return <div className={row} title={fullStamp(line.atMs)}>{icon}{heading}</div>;
  return (
    <details className="group" open={d && d.lines.length <= 24 ? true : undefined} data-diff-path={diff?.path}>
      <summary className={`${row} cursor-pointer list-none hover:bg-surface-2 [&::-webkit-details-marker]:hidden`} title={fullStamp(line.atMs)}>
        {icon}{heading}
        <span className="flex-1" />
        {d && <Counts added={d.added} removed={d.removed} />}
        <Icon name="chevron" className={CHEVRON} />
      </summary>
      {d && diff ? <div className="border-t border-line"><DiffLines lines={d.lines} fallback={diff.text} /></div> : (
        <div className="border-t border-line bg-surface-0 px-3.5 py-2">
          {v.raw ? <pre dir="ltr" className="m-0 max-h-60 overflow-auto font-mono text-xs break-all whitespace-pre-wrap">{v.raw}</pre> : <>
            {v.action === 'run' && v.subject && <pre dir="ltr" className="m-0 mb-2 max-h-52 overflow-auto font-mono text-xs leading-6 break-all whitespace-pre-wrap text-muted"><span aria-hidden className="select-none">$ </span>{v.subject}</pre>}
            <dl dir="ltr" className="m-0 grid max-h-72 grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 overflow-auto font-mono text-xs">
              {v.details.map(([k, val], i) => <div key={i} className="contents">
                <dt className="text-muted">{k}</dt>
                <dd className="m-0 break-words whitespace-pre-wrap">{val}</dd>
              </div>)}
            </dl>
          </>}
        </div>
      )}
    </details>
  );
});

// Tool calls in a row are one bordered group, with a header that says how many steps and, when the entries carry times, how long they took;
// then what kinds. The newest group is open (the caller says so); an older one is folded. Opening it lists every step, each foldable on
// its own. A group that waits on you or failed is always open: that is never hidden in a folded group.
const SHOWN_KINDS = 3;
export const ToolSteps = memo(function ToolSteps({ lines, running = false, flag, open = false, diffs }: { lines: ChatLine[]; running?: boolean; flag?: GroupFlag; open?: boolean; diffs?: ReadonlyMap<number, ChatLine> }) {
  const sum = useMemo(() => stepsSummary(lines), [lines]);
  const kinds = sum.actions.slice(0, SHOWN_KINDS).map((a) => t(`chat-act-${a.action}`, { count: a.count }));
  const more = sum.actions.length - SHOWN_KINDS;
  return (
    <details open={flag || open ? true : undefined} className={`group max-w-3xl overflow-hidden rounded-m border bg-surface-1 ${flag ? 'border-needs-you/70' : 'border-line'}`} data-steps={sum.total} data-flag={flag}>
      <summary className="flex min-h-9 min-w-0 cursor-pointer list-none items-center gap-2 bg-surface-2 px-3 py-1.5 text-xs text-muted hover:text-ink [&::-webkit-details-marker]:hidden" aria-label={`${t('chat-steps-open')}: ${t('chat-steps-count', { count: sum.total })}`}>
        {running && !flag
          ? <span role="img" aria-label={t('chat-working')} className="size-3.5 flex-none animate-spin rounded-full border-2 border-line border-t-working motion-reduce:animate-none" />
          : <Icon name="board" className="size-3.5 flex-none" />}
        <span className="flex-none font-medium text-ink">{t('chat-steps-count', { count: sum.total })}{sum.spanMs ? ` · ${preciseDuration(sum.spanMs)}` : ''}</span>
        <span className="min-w-0 flex-1 truncate max-md:hidden">{kinds.join(' · ')}{more > 0 ? ` · +${more}` : ''}{sum.last ? ` — ${sum.last}` : ''}</span>
        <span className="flex-1 md:hidden" />
        {flag && <span className="flex flex-none items-center gap-1 text-xs font-medium text-needs-you"><Icon name="warn" className="size-3" />{t(flag === 'failed' ? 'chat-steps-failed' : 'chat-steps-needs-you')}</span>}
        <Icon name="chevron" className={CHEVRON} />
      </summary>
      <div className="grid grid-cols-1 divide-y divide-line border-t border-line">
        {lines.map((l) => <ToolStep key={l.id} line={l} diff={diffs?.get(l.id)} />)}
      </div>
    </details>
  );
}, (a, b) => a.running === b.running && a.flag === b.flag && a.open === b.open && a.lines.length === b.lines.length && a.lines[0]?.id === b.lines[0]?.id && (a.diffs?.size ?? 0) === (b.diffs?.size ?? 0));

// A file the agent changed that no step claimed: a card with its name and how much changed, the lines numbered and coloured; a long
// change starts folded.
const DiffEntry = memo(function DiffEntry({ line }: { line: ChatLine }) {
  const d = useMemo(() => parseHunks(line.text), [line.text]);
  const cut = line.path.replace(/\\/g, '/').lastIndexOf('/');
  const name = line.path.slice(cut + 1);
  const dir = cut > 0 ? line.path.slice(0, cut + 1) : '';
  return (
    <details open={d.lines.length <= 24} className="group max-w-3xl overflow-hidden rounded-m border border-line bg-surface-1" data-diff-path={line.path}>
      <summary className="flex min-h-9 min-w-0 cursor-pointer list-none items-center gap-2.5 px-3 py-1.5 text-[0.8125rem] hover:bg-surface-2 [&::-webkit-details-marker]:hidden" title={fullStamp(line.atMs)}>
        <span aria-hidden className="grid size-5.5 flex-none place-items-center rounded-xs border border-line bg-surface-2 text-accent"><Icon name="pencil" className="size-3.5" /></span>
        <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs"><span className="text-muted">{dir}</span><span className="font-semibold text-ink">{name}</span></span>
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
    <div className="flex w-full flex-none flex-wrap items-center gap-x-3 gap-y-2 rounded-m border border-line bg-surface-1 px-3.5 py-2.5" data-files-changed>
      <Icon name="diff" className="size-4 flex-none text-muted" />
      <b className="font-semibold text-ink">{t('chat-files-title', { count: files.length })}</b>
      <Counts added={added} removed={removed} />
      <ul className="m-0 flex min-w-0 list-none flex-wrap gap-1.5 p-0">
        {files.slice(0, SHOWN_FILES).map((f) => <li key={f.path} className="min-w-0">
          <button type="button" dir="ltr" title={f.path} className="inline-flex h-6 max-w-full items-center rounded-xs border border-line bg-surface-2 px-2 font-mono text-xs text-muted hover:text-ink" onClick={() => onGo?.(f.path)}>
            <span className="truncate">{files.some((o) => o !== f && base(o.path) === base(f.path)) ? f.path : base(f.path)}</span>
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
  return (
    <div className="max-w-3xl rounded-m border border-line bg-surface-1 px-3.5 py-3 text-[0.8125rem]" data-plan>
      <div className="mb-2 flex items-center gap-2 font-semibold text-ink">
        <Icon name="board" className="size-4 text-accent" />{label}
        {progress && <small className="font-normal text-muted">{t('chat-plan-progress', { done: progress.done, total: progress.total })}</small>}
      </div>
      {progress && <div role="progressbar" aria-label={t('chat-plan-label')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress.percent} className="mb-2.5 h-1 overflow-hidden rounded-full bg-surface-2"><div className="h-full rounded-full bg-finished" style={{ width: `${progress.percent}%` }} /></div>}
      <ol className="m-0 grid list-none gap-1.5 p-0">{steps.map((p, i) => {
        const s = status?.[i];
        return (
          <li key={i} className={`flex items-start gap-2.5 ${s === 'completed' ? 'text-muted line-through' : s === 'in_progress' ? 'font-medium text-ink' : s ? 'text-muted' : ''}`} data-status={s}>
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

const PlanEntry = memo(function PlanEntry({ line, latest }: { line: ChatLine; latest: boolean }) {
  const steps = line.text.split('\n').filter((s) => s.trim());
  if (!latest) {
    return (
      <details className="max-w-3xl text-xs text-muted">
        <summary className="cursor-pointer">{t('chat-plan-earlier', { count: steps.length })}</summary>
        <div className="mt-1 ps-1"><PlanCard steps={steps} label={t('chat-plan-steps', { count: steps.length })} /></div>
      </details>
    );
  }
  return <div title={fullStamp(line.atMs)}><PlanCard steps={steps} label={t('chat-plan-steps', { count: steps.length })} /></div>;
});

// Where the day changes: a line with the date, in the reader's own format.
function DateRule({ atMs }: { atMs: number }) {
  const label = new Date(atMs).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' });
  return <div role="separator" aria-label={label} className="flex items-center gap-3 py-1 text-xs text-muted"><span className="h-px flex-1 bg-line" /><span>{label}</span><span className="h-px flex-1 bg-line" /></div>;
}

export function Item({ item, running, flag, open, diffs, notice }: { item: TranscriptItem; running?: boolean; flag?: GroupFlag; open?: boolean; diffs?: ReadonlyMap<number, ChatLine>; notice: (l: ChatLine) => ReactNode }) {
  switch (item.kind) {
    case 'date': return <DateRule atMs={item.atMs} />;
    case 'user': return <UserText line={item.line} />;
    case 'reply': return <Reply text={item.text} atMs={item.atMs} />;
    case 'steps': return <ToolSteps lines={item.lines} running={running} flag={flag} open={open} diffs={diffs} />;
    case 'plan': return <PlanEntry line={item.line} latest={item.latest} />;
    case 'diff': return <DiffEntry line={item.line} />;
    case 'notice': return <>{notice(item.line)}</>;
  }
}
