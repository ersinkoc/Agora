import { memo, useMemo, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { ChatLine } from '../net/chat';
import { parseHunks } from '../net/diff';
import { blocks, inlines, type Block, type Inline } from '../net/markdown';
import { titleSays, toolView, type ToolAction } from '../net/tool';
import { Icon, type IconName } from './Icon';

// How one conversation entry looks. Everything here was written by an agent or a person: it is drawn as React text, never as HTML.

const ICONS: Record<ToolAction, IconName> = {
  read: 'files', edit: 'pencil', run: 'terminal', search: 'search', fetch: 'globe', delete: 'trash', move: 'move', think: 'think', other: 'step',
};

function Inlines({ parts }: { parts: Inline[] }) {
  return <>{parts.map((p, i) => p.kind === 'code'
    ? <code key={i} className="rounded-xs bg-surface-2 px-1 py-px font-mono text-[0.92em]">{p.text}</code>
    : p.kind === 'strong' ? <strong key={i} className="font-semibold text-ink">{p.text}</strong> : <span key={i}>{p.text}</span>)}</>;
}

function CopyButton({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  if (typeof navigator === 'undefined' || !navigator.clipboard) return null;
  return (
    <button type="button" className="ide-iconbtn size-6" aria-label={t(done ? 'chat-copied' : 'chat-copy')} title={t(done ? 'chat-copied' : 'chat-copy')}
      onClick={() => { void navigator.clipboard.writeText(text).then(() => { setDone(true); setTimeout(() => setDone(false), 1500); }, () => {}); }}>
      <Icon name="copy" className="size-3.5" />
    </button>
  );
}

function BlockView({ block }: { block: Block }) {
  switch (block.kind) {
    case 'heading': return <p className="m-0 mt-1 font-semibold text-ink"><Inlines parts={block.inlines} /></p>;
    case 'list': {
      const items = block.items.map((it, i) => <li key={i} className="whitespace-pre-wrap"><Inlines parts={it} /></li>);
      return block.ordered ? <ol start={block.start} className="m-0 grid list-decimal gap-0.5 ps-5">{items}</ol> : <ul className="m-0 grid list-disc gap-0.5 ps-5">{items}</ul>;
    }
    case 'quote': return <blockquote className="m-0 border-s-2 border-line ps-3 whitespace-pre-wrap text-muted"><Inlines parts={block.inlines} /></blockquote>;
    case 'code': return (
      <div className="overflow-hidden rounded-s border border-line bg-surface-1">
        <div className="flex h-7 items-center justify-between gap-2 border-b border-line ps-3 pe-1 text-[11px] text-muted"><span className="font-mono">{block.lang}</span><CopyButton text={block.text} /></div>
        <pre dir="ltr" className="m-0 max-h-80 overflow-auto p-3 font-mono text-xs leading-5">{block.text}</pre>
      </div>
    );
    case 'table': return <pre dir="ltr" className="m-0 overflow-x-auto font-mono text-xs leading-5">{block.text}</pre>;
    default: return <p className="m-0 break-words whitespace-pre-wrap"><Inlines parts={block.inlines} /></p>;
  }
}

// What the agent said, with the Markdown it writes (code blocks, inline code, lists, headings) drawn as such.
const AgentText = memo(function AgentText({ text }: { text: string }) {
  const parsed = useMemo(() => blocks(text), [text]);
  return <div className="grid max-w-[90%] gap-2 leading-relaxed">{parsed.map((b, i) => <BlockView key={i} block={b} />)}</div>;
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
  const row = 'flex min-w-0 items-center gap-2 px-3 py-1.5 text-[13px]';
  const icon = <Icon name={ICONS[v.action]} className="size-3.5 text-muted" />;
  if (v.details.length === 0 && !v.raw) return <div className={row} title={stamp(line.atMs)}>{icon}{heading}</div>;
  return (
    <details className="group">
      <summary className={`${row} cursor-pointer list-none hover:bg-surface-2 [&::-webkit-details-marker]:hidden`} title={stamp(line.atMs)}>
        {icon}{heading}
        <span className="flex-1" />
        <Icon name="chevron" className="size-3 text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
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

// Steps in a row read as one list. A long run shows its last few; the earlier ones open on request.
const SHOWN_STEPS = 3;
export function ToolSteps({ lines }: { lines: ChatLine[] }) {
  const [all, setAll] = useState(false);
  const hidden = all || lines.length <= SHOWN_STEPS + 1 ? 0 : lines.length - SHOWN_STEPS;
  return (
    <div className="grid max-w-[90%] divide-y divide-line overflow-hidden rounded-s border border-line bg-surface-1">
      {hidden > 0 && <button type="button" className="flex items-center gap-2 px-3 py-1.5 text-start text-xs text-muted hover:bg-surface-2 hover:text-ink" onClick={() => setAll(true)}>
        <Icon name="more" className="size-3.5" />{t('chat-steps-earlier', { count: hidden })}
      </button>}
      {lines.slice(hidden).map((l) => <ToolStep key={l.id} line={l} />)}
    </div>
  );
}

// A file the agent changed: its name and how much changed, the lines coloured; a long change starts folded.
const DiffEntry = memo(function DiffEntry({ line }: { line: ChatLine }) {
  const d = useMemo(() => parseHunks(line.text), [line.text]);
  const cut = line.path.replace(/\\/g, '/').lastIndexOf('/');
  const name = line.path.slice(cut + 1);
  const dir = cut > 0 ? line.path.slice(0, cut + 1) : '';
  const body = d.lines.length ? (
    <pre dir="ltr" className="m-0 max-h-80 overflow-auto font-mono text-xs leading-5">
      {d.lines.map((l, i) => (
        <div key={i} className={l.kind === 'add' ? 'bg-finished/15' : l.kind === 'del' ? 'bg-needs-you/15' : l.kind === 'hunk' ? 'bg-surface-2 text-muted' : ''}>
          <span className="inline-block w-5 select-none text-center text-muted">{l.kind === 'add' ? '+' : l.kind === 'del' ? '−' : ''}</span>{l.text || ' '}
        </div>
      ))}
    </pre>
  ) : <pre dir="ltr" className="m-0 max-h-60 overflow-auto p-2 font-mono text-xs">{line.text}</pre>;
  return (
    <details open={d.lines.length <= 24} className="group max-w-full overflow-hidden rounded-s border border-line">
      <summary className="flex min-w-0 cursor-pointer list-none items-center gap-2 bg-surface-1 px-3 py-1.5 text-[13px] hover:bg-surface-2 [&::-webkit-details-marker]:hidden" title={stamp(line.atMs)}>
        <Icon name="pencil" className="size-3.5 text-muted" />
        <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-xs"><span className="text-muted">{dir}</span><span className="font-semibold text-ink">{name}</span></span>
        {(d.added > 0 || d.removed > 0) && <span className="flex-none font-mono text-xs"><span className="text-finished">+{d.added}</span> <span className="text-needs-you">−{d.removed}</span></span>}
        <Icon name="chevron" className="size-3 text-muted transition-transform motion-reduce:transition-none rtl:rotate-180 group-open:rotate-90!" />
      </summary>
      <div className="border-t border-line">{body}</div>
    </details>
  );
});

// The agent's plan. Agents send the whole plan again each time it changes: the latest is shown, earlier copies stay folded.
const PlanEntry = memo(function PlanEntry({ line, latest }: { line: ChatLine; latest: boolean }) {
  const steps = line.text.split('\n').filter((s) => s.trim());
  const list = <ol className="m-0 grid list-decimal gap-1 ps-5 marker:text-muted">{steps.map((p, i) => <li key={i}><Inlines parts={inlineTitle(p)} /></li>)}</ol>;
  if (!latest) {
    return (
      <details className="max-w-[90%] text-xs text-muted">
        <summary className="cursor-pointer">{t('chat-plan-earlier', { count: steps.length })}</summary>
        <div className="mt-1 ps-1">{list}</div>
      </details>
    );
  }
  return (
    <div className="max-w-[90%] rounded-s border border-line bg-surface-1 px-3 py-2 text-[13px]" title={stamp(line.atMs)}>
      <div className="mb-1.5 text-xs font-medium text-muted">{t('chat-plan-steps', { count: steps.length })}</div>
      {list}
    </div>
  );
});

function stamp(atMs: number): string | undefined {
  return atMs > 0 ? new Date(atMs).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : undefined;
}

export const UserText = memo(function UserText({ line }: { line: ChatLine }) {
  return <div className="ms-auto max-w-[85%] rounded-m bg-accent/15 px-3 py-2 break-words whitespace-pre-wrap" title={stamp(line.atMs)}>{line.text}</div>;
});

// The transcript as a person reads it: steps in a row are one list, and only the latest plan is open.
export type ChatGroup = { kind: 'steps'; lines: ChatLine[] } | { kind: 'line'; line: ChatLine; latestPlan: boolean };

export function groupLines(lines: readonly ChatLine[]): ChatGroup[] {
  const out: ChatGroup[] = [];
  let lastPlan = -1;
  lines.forEach((l, i) => { if (l.role === 'agent' && l.kind === 'plan') lastPlan = i; });
  lines.forEach((l, i) => {
    if (l.role === 'agent' && l.kind === 'tool') {
      const prev = out[out.length - 1];
      if (prev?.kind === 'steps') prev.lines.push(l);
      else out.push({ kind: 'steps', lines: [l] });
    } else out.push({ kind: 'line', line: l, latestPlan: i === lastPlan });
  });
  return out;
}

export function Entry({ line, latestPlan, notice }: { line: ChatLine; latestPlan: boolean; notice: (l: ChatLine) => ReactNode }) {
  if (line.role === 'user') return <UserText line={line} />;
  if (line.kind === 'notice') return <>{notice(line)}</>;
  if (line.kind === 'plan') return <PlanEntry line={line} latest={latestPlan} />;
  if (line.kind === 'diff') return <DiffEntry line={line} />;
  return <AgentText text={line.text} />;
}
