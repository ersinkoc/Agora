import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { mergeLines, noticeText, type ChatLine, type ChatSource } from '../net/chat';
import { atBottom, grew, jumped, keepPosition, scrolled, start, type Follow } from '../net/follow';
import { groupFlag, transcript, windowed } from '../net/transcript';
import { FilesChanged, Item } from './ChatEntry';
import { sendsOnEnter } from './composer';
import { Icon } from './Icon';
import { SessionBar } from './SessionBar';
import { StateMark } from './StateBadge';
import type { OptionKind } from './sessionChoice';

// A notice (ADR-0058): something went wrong or changed in the conversation, said by the window from the code the adapter sent.
const notice = (line: ChatLine) => (
  <div role="note" className="flex max-w-3xl items-start gap-2 rounded-s border border-needs-you/60 bg-surface-1 px-3 py-2 text-[0.8125rem] break-words whitespace-pre-wrap text-ink">
    <Icon name="warn" className="mt-0.5 size-3.5 flex-none text-needs-you" /><span className="min-w-0"><span className="sr-only">{t('chat-error-label')}: </span>{noticeText(line, t)}</span>
  </div>
);

// How many items of a long history are drawn at once, and how many more "Show earlier" adds. The node keeps at most 500 entries, which
// a window draws in full only when asked; the rest of the time a conversation is as light as its newest page.
const PAGE = 120;

// A chat agent has no terminal: you talk to it with requests and read its answers, as a conversation the node keeps (ADR-0038).
export function PromptBox({ agent, send, source, choose }: { agent: AgentTileModel; send?: (a: AgentTileModel, text: string) => Promise<boolean>; source?: ChatSource; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<boolean> }) {
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
    if (target instanceof HTMLDetailsElement) target.open = true;
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
  const rows = Math.min(8, Math.max(2, text.split('\n').length));
  const working = agent.state === 'working' && !agent.closed;
  return (
    <div className="flex h-full min-h-0 flex-col gap-2 rounded-m bg-surface-0 p-3">
      {/* Keyed by the agent: another agent on screen gets a fresh bar, so nothing left open carries over; usage updates keep it. */}
      {agent.session && <div className="flex-none border-b border-line pb-2"><SessionBar key={agent.id} part="status" agentKey={agent.id} session={agent.session} startedMs={agent.startedMs} disabled={agent.closed} /></div>}
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div ref={log} onScroll={onScroll} className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto pe-1 [&>*]:shrink-0" aria-live="polite" aria-label={t('chat-log')} role="log" tabIndex={0}>
          {lines.length === 0 && <p className="muted m-0">{t(source ? 'chat-empty' : 'prompt-intro')}</p>}
          {view.hidden > 0 && <button type="button" className="button justify-self-center self-center px-3 py-1 text-xs" onClick={earlier}>{t('chat-earlier-items', { count: view.hidden })}</button>}
          {view.items.map((item) => <Item key={item.id} item={item} running={working && item === last} flag={item === newestSteps ? groupFlag(agent, true) : undefined} notice={notice} />)}
          {working && lines.length > 0 && <div role="status" className="flex items-center gap-2 text-xs text-muted"><StateMark state="working" />{t('chat-working')}</div>}
        </div>
        {!follow.following && lines.length > 0 && (
          <button type="button" onClick={toLatest} className="terminal-jump absolute bottom-2 start-1/2 -translate-x-1/2 transition-opacity motion-reduce:transition-none">
            <Icon name="chevronDown" className="size-3.5" />{follow.unseen > 0 ? t('chat-jump-new', { count: follow.unseen }) : t('chat-jump')}
          </button>
        )}
      </div>
      <FilesChanged lines={lines} onGo={goTo} />
      <form className="flex flex-none flex-col gap-2 rounded-m border border-line bg-surface-1 p-2 transition-colors focus-within:border-accent" onSubmit={(e) => { e.preventDefault(); void go(); }}>
        <textarea value={text} onChange={(e) => setText(e.target.value)} aria-label={t('prompt-label')} placeholder={t('prompt-label')} rows={rows}
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
          <span className="flex-1" />
          {sent && <span role="status" className={`text-xs ${sent === 'ok' ? 'text-finished' : 'text-needs-you'}`}>{t(sent === 'ok' ? 'prompt-sent' : 'prompt-failed')}</span>}
          <span className="hidden text-xs text-muted md:inline">{t(agent.closed ? 'prompt-closed' : 'prompt-hint')}</span>
          <button type="submit" className="button button--primary" disabled={busy || !text.trim() || !send || agent.closed}>{t('prompt-send')}</button>
        </div>
      </form>
    </div>
  );
}
