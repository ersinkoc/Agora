import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { AgentTileModel, SessionModel } from '../model';
import { mergeLines, noticeText, type ChatLine, type ChatSource } from '../net/chat';
import { groupTurns, pairDiffs, type TurnItem } from '../net/chatView';
import { atBottom, grew, jumped, keepPosition, scrolled, start, type Follow } from '../net/follow';
import { groupFlag, transcript, windowed } from '../net/transcript';
import { FilesChanged, Item, Turn } from './ChatEntry';
import { sendsOnEnter } from './composer';
import { Icon } from './Icon';
import { NoAskBadge } from './NoAskBadge';
import { SessionBar } from './SessionBar';
import { StateMark } from './StateBadge';
import type { OptionKind } from './sessionChoice';

// What the header shows for an agent that reported no session (no modes, models or context): state, time and tokens only.
const NO_SESSION: SessionModel = { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 0, contextWindow: 0 };

// A notice (ADR-0058): something went wrong or changed in the conversation, said by the window from the code the adapter sent.
const notice = (line: ChatLine) => (
  <div role="note" className="flex max-w-3xl items-start gap-2 rounded-m border border-needs-you/60 bg-surface-1 px-3 py-2 text-[0.8125rem] break-words whitespace-pre-wrap text-ink">
    <Icon name="warn" className="mt-0.5 size-3.5 flex-none text-needs-you" /><span className="min-w-0"><span className="sr-only">{t('chat-error-label')}: </span>{noticeText(line, t)}</span>
  </div>
);

// How many items of a long history are drawn at once, and how many more "Show earlier" adds. The node keeps at most 500 entries, which
// a window draws in full only when asked; the rest of the time a conversation is as light as its newest page.
const PAGE = 120;

// A chat agent has no terminal: you talk to it with requests and read its answers, as a conversation the node keeps (ADR-0038).
// `approvals`: what this agent waits on, pinned above the composer (the caller draws them). `onReview`: the way into the review screen.
export function PromptBox({ agent, send, source, choose, approvals, onReview }: { agent: AgentTileModel; send?: (a: AgentTileModel, text: string) => Promise<boolean>; source?: ChatSource; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<boolean>; approvals?: ReactNode; onReview?: () => void }) {
  const [text, setText] = useState('');
  const [sent, setSent] = useState<'ok' | 'failed' | undefined>();
  const [busy, setBusy] = useState(false);
  const [lines, setLines] = useState<ChatLine[]>([]);
  const [shown, setShown] = useState(PAGE);
  const [follow, setFollow] = useState<Follow>(start);
  const log = useRef<HTMLDivElement>(null);
  const seen = useRef(0);
  // Older entries are being drawn above the reader: the height before, and the diff to scroll to once they are in.
  const before = useRef<number | undefined>(undefined);
  const goAfter = useRef<string | undefined>(undefined);
  const composedAt = useRef(0);
  const all = useMemo(() => transcript(lines), [lines]);
  const view = useMemo(() => windowed(all, shown), [all, shown]);
  const last = view.items.length ? view.items[view.items.length - 1] : undefined;
  const newestSteps = [...view.items].reverse().find((i) => i.kind === 'steps');
  const rows = useMemo(() => groupTurns(view.items).map((row) => ({ row, paired: row.kind === 'turn' ? pairDiffs(row.items) : undefined })), [view.items]);
  const modelId = agent.session?.currentModel;
  const model = agent.session?.models.find((m) => m.id === modelId)?.name || undefined;

  useEffect(() => {
    if (!source) return;
    setLines([]); setShown(PAGE); setFollow(start); seen.current = 0;
    const sub = source.connect((incoming, replace) => setLines((held) => (replace ? incoming : mergeLines(held, incoming))));
    return () => sub.close();
  }, [source]);

  // Older entries were drawn above the reader (it runs when `shown` changes, which is when they appear): keep the same line in view, never
  // leave the position for a later streamed chunk to consume. Then a pending "go to this file" finishes.
  useLayoutEffect(() => {
    const el = log.current;
    if (!el) return;
    if (before.current !== undefined) { el.scrollTop = keepPosition(el.scrollTop, before.current, el.scrollHeight); before.current = undefined; }
    const path = goAfter.current;
    goAfter.current = undefined;
    if (path) reveal(path);
  }, [shown]); // eslint-disable-line react-hooks/exhaustive-deps

  // The reader is at the bottom: new entries keep them there (one jump, never smooth: a smooth scroll fights the next chunk). Otherwise
  // nothing moves and the pill counts what arrived.
  useLayoutEffect(() => {
    const el = log.current;
    const added = lines.length - seen.current;
    seen.current = lines.length;
    if (!el) return;
    if (follow.following) el.scrollTop = el.scrollHeight;
    else if (added > 0) setFollow((f) => grew(f, added));
  }, [lines, agent.state]); // eslint-disable-line react-hooks/exhaustive-deps
  const onScroll = useCallback(() => { const el = log.current; if (el) setFollow((f) => scrolled(f, atBottom(el))); }, []);
  const toLatest = () => { const el = log.current; if (el) el.scrollTop = el.scrollHeight; setFollow(jumped()); };
  const earlier = () => { before.current = log.current?.scrollHeight; setShown((n) => n + PAGE); };
  const reveal = (path: string): boolean => {
    const target = [...(log.current?.querySelectorAll<HTMLElement>('[data-diff-path]') ?? [])].reverse().find((d) => d.dataset.diffPath === path);
    if (!target) return false;
    // The change may sit in a row of a folded group: open every fold around it.
    for (let el: HTMLElement | null = target; el; el = el.parentElement) if (el instanceof HTMLDetailsElement) el.open = true;
    target.scrollIntoView?.({ block: 'center' });
    return true;
  };
  // A file whose change is older than what is drawn: draw earlier entries (keeping the position), then go to it.
  const goTo = (path: string) => { if (!reveal(path) && view.hidden > 0) { goAfter.current = path; earlier(); } };

  const go = async () => {
    const body = text.trim();
    if (!body || !send || busy || agent.closed) return;
    setBusy(true);
    const ok = await send(agent, body);
    setBusy(false);
    setSent(ok ? 'ok' : 'failed');
    // Only what was sent is cleared: text typed while the send was in flight stays.
    if (ok) { setText((now) => (now.trim() === body ? '' : now)); toLatest(); }
  };
  // The composer grows with what is typed, up to a limit, then scrolls.
  const rowCount = Math.min(8, Math.max(2, text.split('\n').length));
  const working = agent.state === 'working' && !agent.closed;
  return (
    <div className="flex h-full min-h-0 flex-col bg-surface-0">
      {/* Keyed by the agent: another agent on screen gets a fresh bar, so nothing left open carries over; usage updates keep it. */}
      <div className="flex-none border-b border-line bg-surface-1 px-4 py-2" data-chat-header>
        <SessionBar key={agent.id} part="status" agentKey={agent.id} session={agent.session ?? NO_SESSION} startedMs={agent.startedMs} disabled={agent.closed} state={agent.closed ? undefined : agent.state} tokens={agent.tokens} />
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div ref={log} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto px-4 pt-6 pb-4 max-md:pt-4" aria-live="polite" aria-label={t('chat-log')} role="log" tabIndex={0}>
          <div className="mx-auto flex max-w-195 flex-col gap-5 [&>*]:shrink-0">
            {lines.length === 0 && <p className="muted m-0">{t(source ? 'chat-empty' : 'prompt-intro')}</p>}
            {view.hidden > 0 && <button type="button" className="button justify-self-center self-center px-3 py-1 text-xs" onClick={earlier}>{t('chat-earlier-items', { count: view.hidden })}</button>}
            {rows.map(({ row, paired }) => {
              if (row.kind !== 'turn' || !paired) return row.kind === 'turn' ? null : <Item key={row.id} item={row} notice={notice} />;
              // An edit and the diff it made are one row; every other item is drawn as it is.
              const draw = (item: TurnItem) => <Item key={item.id} item={item} running={working && item === last} open={item === newestSteps} diffs={paired.attached} flag={item === newestSteps ? groupFlag(agent, true) : undefined} notice={notice} />;
              return <Turn key={row.id} name={agent.name} model={model} atMs={row.atMs}>{paired.items.map(draw)}</Turn>;
            })}
            {working && lines.length > 0 && <div role="status" className="flex items-center gap-2 text-xs text-muted"><StateMark state="working" />{t('chat-working')}</div>}
            <FilesChanged lines={lines} onGo={goTo} onReview={onReview} />
          </div>
        </div>
        {!follow.following && lines.length > 0 && (
          <button type="button" onClick={toLatest} className="terminal-jump absolute bottom-2 start-1/2 -translate-x-1/2 transition-opacity motion-reduce:transition-none">
            <Icon name="chevronDown" className="size-3.5" />{follow.unseen > 0 ? t('chat-jump-new', { count: follow.unseen }) : t('chat-jump')}
          </button>
        )}
      </div>
      <div className="flex flex-none flex-col gap-2 px-4 pt-2 pb-3.5">
      {approvals && <div className="mx-auto max-h-[40vh] w-full max-w-195 overflow-y-auto" data-pinned-approvals>{approvals}</div>}
      <form className="mx-auto flex w-full max-w-195 flex-none flex-col gap-2 rounded-l border border-line bg-surface-1 p-2.5 shadow-card transition-colors focus-within:border-accent" onSubmit={(e) => { e.preventDefault(); void go(); }}>
        <textarea value={text} onChange={(e) => setText(e.target.value)} aria-label={t('prompt-label')} placeholder={t('prompt-label')} rows={rowCount}
          className="w-full resize-none border-0 bg-transparent p-1 text-ink outline-none focus-visible:outline-none"
          disabled={agent.closed}
          onCompositionEnd={() => { composedAt.current = Date.now(); }}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter starts a new line; a key that only confirms an input method's candidate is not a send (Safari too).
            if (e.key === 'Enter' && !e.shiftKey) e.preventDefault();
            if (sendsOnEnter({ key: e.key, shiftKey: e.shiftKey, isComposing: e.nativeEvent.isComposing, keyCode: e.keyCode }, Date.now() - composedAt.current)) void go();
          }} />
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          {agent.session && <SessionBar key={agent.id} part="controls" agentKey={agent.id} session={agent.session} disabled={agent.closed} choose={choose} />}
          <NoAskBadge agent={agent} />
          <span className="flex-1" />
          {sent && <span role="status" className={`text-xs ${sent === 'ok' ? 'text-finished' : 'text-needs-you'}`}>{t(sent === 'ok' ? 'prompt-sent' : 'prompt-failed')}</span>}
          <span className="hidden text-xs text-muted md:inline">{t(agent.closed ? 'prompt-closed' : 'prompt-hint')}</span>
          <button type="submit" className="button button--primary" disabled={busy || !text.trim() || !send || agent.closed}>{t('prompt-send')}</button>
        </div>
      </form>
      </div>
    </div>
  );
}
