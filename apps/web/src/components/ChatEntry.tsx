import { memo, useMemo, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { ChatLine } from '../net/chat';
import { parseHunks } from '../net/diff';
import { blocks, inlines, type Block, type Inline } from '../net/markdown';
import { titleSays, toolView, type ToolAction } from '../net/tool';
import { changedFiles, stepsSummary, type TranscriptItem } from '../net/transcript';
import { duration } from '../net/time';
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
// What the agent said, with the Markdown it writes drawn as such. It grows in place while it streams: the parsed blocks of text that did
// not change keep their elements, so nothing above the end moves or flickers.
const Reply = memo(function Reply({ text, atMs }: { text: string; atMs: number }) {
  const parsed = useMemo(() => blocks(text), [text]);
  return (
    <div className="group/msg relative max-w-3xl border-s-2 border-accent/40 ps-3" title={fullStamp(atMs)} data-role="agent">
      <span className="sr-only">{t('chat-agent')}: </span>
      <div className="grid gap-2 leading-relaxed">{parsed.map((b, i) => <BlockView key={i} block={b} />)}</div>
      <CopyButton text={text} label="chat-copy-reply" className={`absolute end-0 top-0 ${HOVER}`} />
    </div>
  );
});

export const UserText = memo(function UserText({ line }: { line: ChatLine }) {
  return (
    <div className="group/msg relative ms-auto max-w-[85%] rounded-m border border-accent/30 bg-accent/15 px-3 py-2 break-words whitespace-pre-wrap" title={fullStamp(line.atMs)} data-role="user">
      <span className="sr-only">{t('chat-you')}: </span>{line.text}
      <CopyButton text={line.text} label="chat-copy-message" className={`absolute -start-7 top-1 ${HOVER}`} />
    </div>
  );
});

// One thing the agent did: a sentence with its subject, and the full input folded underneath.
const ToolStep = memo(function ToolStep({ line }: { line: ChatLine }) {
  const v = useMemo(() => toolView(line.text), [line.text]);
  const heading = v.raw
    ? <span className="min-w-0 flex-1 truncate font-mono text-xs text-muted">{v.raw}</span>
    : <>
      <span className="min-w-0 shrink truncate text-ink">{v.title ? <Inlines parts={inlineTitle(v.title)} /> : t(`tool-${v.action}`)}</span>
      {v.subject && !titleSays(v.title, v.subject) && <code dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs text-muted">{v.subject}</code>}
    </>;
  const row = 'flex min-w-0 items-center gap-2 px-3 py-1.5 text-[0.8125rem]';
  const icon = <Icon name={ICONS[v.action]} className="size-3.5 flex-none text-muted" />;
  if (v.details.length === 0 && !v.raw) return <div className={row} title={fullStamp(line.atMs)}>{icon}{heading}</div>;
  return (
    <details className="group">
      <summary className={`${row} cursor-pointer list-none hover:bg-surface-2 [&::-webkit-details-marker]:hidden`} title={fullStamp(line.atMs)}>
        {icon}{heading}
        <span className="flex-1" />
        <Icon name="chevron" className="size-3 flex-none text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <div className="border-t border-line bg-surface-0 px-3 py-2">
        {v.raw ? <pre dir="ltr" className="m-0 max-h-60 overflow-auto font-mono text-xs break-all whitespace-pre-wrap">{v.raw}</pre> : (
          <dl dir="ltr" className="m-0 grid max-h-72 grid-cols-[max-content_minmax(0,1fr)] gap-x-3 gap-y-1 overflow-auto font-mono text-xs">
            {v.details.map(([k, val], i) => <div key={i} className="contents">
              <dt className="text-muted">{k}</dt>
              <dd className="m-0 break-words whitespace-pre-wrap">{val}</dd>
            </div>)}
          </dl>
        )}
      </div>
    </details>
  );
});

// A title in backticks is a command or a name: it is shown as code. Inline backticks become inline code.
function inlineTitle(title: string): Inline[] {
  return /^`[^`]+`$/.test(title) ? [{ kind: 'code', text: title.slice(1, -1) }] : inlines(title);
}

// Tool calls in a row are one group, folded by default, with one line that says what they did: what kinds, how many, the last one, and
// how long it took when the entries carry times. Opening it lists every step, each foldable on its own.
const SHOWN_KINDS = 3;
export const ToolSteps = memo(function ToolSteps({ lines, running = false }: { lines: ChatLine[]; running?: boolean }) {
  const sum = useMemo(() => stepsSummary(lines), [lines]);
  const kinds = sum.actions.slice(0, SHOWN_KINDS).map((a) => t(`chat-act-${a.action}`, { count: a.count }));
  const more = sum.actions.length - SHOWN_KINDS;
  const first = sum.actions[0]?.action ?? 'other';
  return (
    <details className="group max-w-3xl overflow-hidden rounded-s border border-line bg-surface-1" data-steps={sum.total}>
      <summary className="flex min-w-0 cursor-pointer list-none items-center gap-2 px-3 py-1.5 text-[0.8125rem] hover:bg-surface-2 [&::-webkit-details-marker]:hidden" aria-label={`${t('chat-steps-open')}: ${t('chat-steps-count', { count: sum.total })}`}>
        <Icon name={ICONS[first]} className="size-3.5 flex-none text-muted" />
        <span className="min-w-0 flex-1 truncate">
          <span className="font-medium text-ink">{kinds.join(' · ')}{more > 0 ? ` · +${more}` : ''}</span>
          {sum.last && <span className="text-muted"> — {sum.last}</span>}
        </span>
        {running && <span className="state__mark flex-none rounded-full bg-working" style={{ width: '0.5rem', height: '0.5rem', animation: 'breathe 2.4s ease-in-out infinite' }} aria-hidden />}
        <span className="flex-none text-xs text-muted">{t('chat-steps-count', { count: sum.total })}{sum.spanMs ? ` · ${duration(sum.spanMs)}` : ''}</span>
        <Icon name="chevron" className="size-3 flex-none text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <div className="grid divide-y divide-line border-t border-line">
        {lines.map((l) => <ToolStep key={l.id} line={l} />)}
      </div>
    </details>
  );
});

// A file the agent changed: a card with its name and how much changed, the lines coloured; a long change starts folded.
const DiffEntry = memo(function DiffEntry({ line }: { line: ChatLine }) {
  const d = useMemo(() => parseHunks(line.text), [line.text]);
  const cut = line.path.replace(/\\/g, '/').lastIndexOf('/');
  const name = line.path.slice(cut + 1);
  const dir = cut > 0 ? line.path.slice(0, cut + 1) : '';
  const body = d.lines.length ? (
    <pre dir="ltr" className="m-0 max-h-80 overflow-auto font-mono text-xs leading-5">
      {d.lines.map((l, i) => (
        <div key={i} className={l.kind === 'add' ? 'bg-finished/15 diff-bar-add' : l.kind === 'del' ? 'bg-removed/15 diff-bar-del' : l.kind === 'hunk' ? 'bg-surface-2 text-muted' : ''}>
          <span className="inline-block w-5 select-none text-center text-muted">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>{l.text || ' '}
        </div>
      ))}
    </pre>
  ) : <pre dir="ltr" className="m-0 max-h-60 overflow-auto p-2 font-mono text-xs">{line.text}</pre>;
  return (
    <details open={d.lines.length <= 24} className="group max-w-3xl overflow-hidden rounded-s border border-line" data-diff-path={line.path}>
      <summary className="flex min-w-0 cursor-pointer list-none items-center gap-2 bg-surface-1 px-3 py-1.5 text-[0.8125rem] hover:bg-surface-2 [&::-webkit-details-marker]:hidden" title={fullStamp(line.atMs)}>
        <Icon name="pencil" className="size-3.5 flex-none text-muted" />
        <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs"><span className="text-muted">{dir}</span><span className="font-semibold text-ink">{name}</span></span>
        {(d.added > 0 || d.removed > 0) && <span className="flex-none font-mono text-xs"><span className="text-finished">+{d.added}</span> <span className="text-removed">−{d.removed}</span></span>}
        <Icon name="chevron" className="size-3 flex-none text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <div className="border-t border-line">{body}</div>
    </details>
  );
});

// The files this conversation changed, in one place: each file once with its totals; a click goes to its change in the conversation.
export function FilesChanged({ lines, onGo }: { lines: readonly ChatLine[]; onGo?: (path: string) => void }) {
  const files = useMemo(() => changedFiles(lines), [lines]);
  if (files.length === 0) return null;
  const added = files.reduce((n, f) => n + f.added, 0);
  const removed = files.reduce((n, f) => n + f.removed, 0);
  const SHOWN = 8;
  return (
    <details className="group flex-none rounded-s border border-line bg-surface-1">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-1.5 text-[0.8125rem] hover:bg-surface-2 [&::-webkit-details-marker]:hidden">
        <Icon name="diff" className="size-3.5 flex-none text-muted" />
        <span className="min-w-0 flex-1 truncate font-medium text-ink">{t('chat-files-title', { count: files.length })}</span>
        <span className="flex-none font-mono text-xs"><span className="text-finished">+{added}</span> <span className="text-removed">−{removed}</span></span>
        <Icon name="chevron" className="size-3 flex-none text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <ul className="m-0 grid max-h-40 list-none overflow-y-auto border-t border-line p-0">
        {files.slice(0, SHOWN * 4).map((f) => <li key={f.path}>
          <button type="button" className="ide-row justify-between gap-3 rounded-none px-3 py-1" onClick={() => onGo?.(f.path)}>
            <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs">{f.path}</span>
            <span className="flex-none font-mono text-xs"><span className="text-finished">+{f.added}</span> <span className="text-removed">−{f.removed}</span></span>
          </button>
        </li>)}
        {files.length > SHOWN * 4 && <li className="muted px-3 py-1 text-xs">{t('chat-files-more', { count: files.length - SHOWN * 4 })}</li>}
      </ul>
    </details>
  );
}

// The agent's plan, as a numbered checklist. Agents send the whole plan again each time it changes: the latest is shown, earlier copies
// stay folded. The node keeps the steps' text only (no done or not done), so no step is ever drawn ticked.
const PlanEntry = memo(function PlanEntry({ line, latest }: { line: ChatLine; latest: boolean }) {
  const steps = line.text.split('\n').filter((s) => s.trim());
  const list = <ol className="m-0 grid list-none gap-1.5 p-0">{steps.map((p, i) => <li key={i} className="flex items-start gap-2.5">
    <span aria-hidden className="mt-px grid size-5 flex-none place-items-center rounded-full border border-line bg-surface-0 text-xs text-muted">{i + 1}</span>
    <span className="min-w-0 break-words"><Inlines parts={inlineTitle(p)} /></span>
  </li>)}</ol>;
  if (!latest) {
    return (
      <details className="max-w-3xl text-xs text-muted">
        <summary className="cursor-pointer">{t('chat-plan-earlier', { count: steps.length })}</summary>
        <div className="mt-1 ps-1 text-[0.8125rem] text-ink">{list}</div>
      </details>
    );
  }
  return (
    <div className="max-w-3xl rounded-s border border-line bg-surface-1 px-3 py-2 text-[0.8125rem]" title={fullStamp(line.atMs)}>
      <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted"><Icon name="board" className="size-3.5" />{t('chat-plan-steps', { count: steps.length })}</div>
      {list}
    </div>
  );
});

// Where the day changes: a line with the date, in the reader's own format.
function DateRule({ atMs }: { atMs: number }) {
  const label = new Date(atMs).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' });
  return <div role="separator" aria-label={label} className="flex items-center gap-3 py-1 text-xs text-muted"><span className="h-px flex-1 bg-line" /><span>{label}</span><span className="h-px flex-1 bg-line" /></div>;
}

export function Item({ item, running, notice }: { item: TranscriptItem; running?: boolean; notice: (l: ChatLine) => ReactNode }) {
  switch (item.kind) {
    case 'date': return <DateRule atMs={item.atMs} />;
    case 'user': return <UserText line={item.line} />;
    case 'reply': return <Reply text={item.text} atMs={item.atMs} />;
    case 'steps': return <ToolSteps lines={item.lines} running={running} />;
    case 'plan': return <PlanEntry line={item.line} latest={item.latest} />;
    case 'diff': return <DiffEntry line={item.line} />;
    case 'notice': return <>{notice(item.line)}</>;
  }
}
