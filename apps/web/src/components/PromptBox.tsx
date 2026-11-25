import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { t } from '../i18n';
import type { AgentTileModel, SessionModel } from '../model';
import { isRich, mergeLines, noticeText, queuedNote, queueState, trimLines, type ChatLine, type ChatSource } from '../net/chat';
import { groupTurns, pairDiffs, type TurnItem } from '../net/chatView';
import { atBottom, grew, jumped, keepPosition, laidOut, resized, scrolled, sizeOf, start, type Follow, type Size } from '../net/follow';
import { refusalKey, type ActionResult } from '../net/refusal';
import { applyCommand, filterCommands, slashQuery, stepIndex, uniqueCommands } from '../net/slash';
import { endedGroups, flagOf, transcript, windowed } from '../net/transcript';
import { FilesChanged, Item, Turn } from './ChatEntry';
import { CloseAgentContext } from './CloseAgent';
import { closeAsk, isCloseCommand } from '../net/closeAgent';
import { ChatActionsContext, promptFailure, type PromptFailure, type PromptResult, type PromptSend } from './chatActions';
import { IME_GRACE_MS, sendsOnEnter } from './composer';
import { Icon } from './Icon';
import { PinnedShort, SHORT_PANE_PX, useBelow } from './ApprovalRequest';
import { NoAskBadge } from './NoAskBadge';
import { AsksLessChip } from './AllowAlways';
import { SessionBar } from './SessionBar';
import { StateMark } from './StateBadge';
import type { OptionKind } from './sessionChoice';

// What the header shows for an agent that reported no session (no modes, models or context): state, time and tokens only.
const NO_SESSION: SessionModel = { modes: [], models: [], currentMode: '', currentModel: '', contextTokens: 0, contextWindow: 0 };

// A notice (ADR-0058): something went wrong or changed in the conversation, said by the window from the code the adapter sent.
const notice = (line: ChatLine) => (
  <div role="note" className="flex max-w-3xl items-start gap-2 rounded-m border border-danger/60 bg-surface-1 px-3 py-2 text-[0.8125rem] break-words whitespace-pre-wrap text-ink">
    <Icon name="warn" className="mt-0.5 size-3.5 flex-none text-danger" /><span className="min-w-0"><span className="sr-only">{t('chat-error-label')}: </span>{noticeText(line, t)}</span>
  </div>
);

// How many items of a long history are drawn at once, and how many more "Show earlier" adds. The node keeps at most 500 entries, which
// a window draws in full only when asked; the rest of the time a conversation is as light as its newest page.
const PAGE = 120;
/** #908: how long the agent must be out of its turn before a call that never said it finished is drawn stopped: longer than the node's
 *  150 ms coalescing of a call's updates and the skew between the state and the conversation streams. */
export const STOP_GRACE_MS = 1000;

// A chat agent has no terminal: you talk to it with requests and read its answers, as a conversation the node keeps (ADR-0038).
// `approvals`: what this agent waits on, pinned above the composer (the caller draws them). `onReview`: the way into the review screen.
// Without `send` (a viewer's window, ADR-0106: the hub would refuse a request) the composer is read-only: the conversation and the agent's
// settings (their menus off without `choose`), and a line that says why there is nothing to type in.
export function PromptBox({ agent, send, source, choose, approvals, onReview }: { agent: AgentTileModel; send?: PromptSend; source?: ChatSource; choose?: (agentKey: string, kind: OptionKind, id: string) => Promise<ActionResult>; approvals?: ReactNode; onReview?: () => void }) {
  const actions = useContext(ChatActionsContext);
  const canClose = !!useContext(CloseAgentContext);
  const [text, setText] = useState('');
  const [sent, setSent] = useState<'ok' | 'queued' | PromptFailure | undefined>();
  // #908: the newest entry id held when the person last sent: a user entry newer than it is theirs (it has arrived).
  const [sentMark, setSentMark] = useState(0);
  const [busy, setBusy] = useState(false);
  // At most MAX_LINES entries are held; older ones are dropped and counted, so a stream of new ids cannot grow memory or the per-chunk work.
  const [held, setHeld] = useState<{ lines: ChatLine[]; trimmed: number }>({ lines: [], trimmed: 0 });
  const lines = held.lines;
  const [shown, setShown] = useState(PAGE);
  const [follow, setFollow] = useState<Follow>(start);
  // The "/" menu: where the caret is, which command is highlighted, and whether Escape closed it for what is typed now.
  const [caret, setCaret] = useState(0);
  const [slashAt, setSlashAt] = useState(0);
  const [slashOff, setSlashOff] = useState(false);
  // Stopping the turn, and dropping the queue (which asks first, because it also stops the turn).
  const [stopping, setStopping] = useState<'busy' | undefined>();
  const [stopFailed, setStopFailed] = useState<string | undefined>(); // the locale key of why the turn did not stop
  const [dropAsk, setDropAsk] = useState(false);
  const log = useRef<HTMLDivElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const pinned = useRef<HTMLDivElement>(null);
  // #1082: a pane too short for a request card and the conversation draws what it waits on as one row (`PinnedShort`).
  const pane = useRef<HTMLDivElement>(null);
  const short = useBelow(pane, SHORT_PANE_PX, 'height');
  const caretTo = useRef<number | undefined>(undefined);
  const seen = useRef(0);
  // Older entries are being drawn above the reader: the height before, and the diff to scroll to once they are in.
  const before = useRef<number | undefined>(undefined);
  const goAfter = useRef<string | undefined>(undefined);
  // #1060: what the scroll handler and the size observer need without waiting for a render: whether the log follows, its sizes when last
  // seen, and whether the log itself was just resized (the scroll event of an offset the resize clamped can come a frame later, with the
  // sizes already back).
  const followingNow = useRef(true);
  const seenSize = useRef<Size | undefined>(undefined);
  const resizing = useRef(false);
  // The reader just clicked in the log (opened a fold, asked for more output): the size change that follows is theirs, not a resize.
  const readerActed = useRef(false);
  const composedAt = useRef(0);
  const all = useMemo(() => transcript(lines), [lines]);
  const view = useMemo(() => windowed(all, shown), [all, shown]);
  // The newest item the agent is on: prompts still waiting are drawn after everything (#908) and are not it.
  const last = [...view.items].reverse().find((i) => !(i.kind === 'user' && i.line.queued));
  const newestSteps = [...view.items].reverse().find((i) => i.kind === 'steps');
  const rows = useMemo(() => groupTurns(view.items).map((row) => ({ row, paired: row.kind === 'turn' ? pairDiffs(row.items) : undefined })), [view.items]);
  const queue = useMemo(() => queueState(lines), [lines]);
  const waitingCount = queue.waiting.length;
  const arrived = useMemo(() => lines.some((l) => l.role === 'user' && l.id > sentMark), [lines, sentMark]);
  const modelId = agent.session?.currentModel;
  const model = agent.session?.models.find((m) => m.id === modelId)?.name || undefined;
  const offered = agent.session?.commands;
  const commands = useMemo(() => uniqueCommands(offered ?? []), [offered]);

  useEffect(() => {
    if (!source) return;
    setHeld({ lines: [], trimmed: 0 }); setShown(PAGE); setFollow(start); seen.current = 0;
    const sub = source.connect((incoming, replace) => setHeld((h) => {
      const t = trimLines(replace ? [...incoming] : mergeLines(h.lines, incoming));
      if (t.lines === h.lines) return h; // nothing new: nothing redraws
      return { lines: t.lines, trimmed: (replace ? 0 : h.trimmed) + t.dropped };
    }));
    return () => sub.close();
  }, [source]);

  // To the newest entry, remembering the sizes it was pinned at: the scroll event this causes is then not taken for a resize.
  const pin = useCallback(() => { const el = log.current; if (!el) return; el.scrollTop = el.scrollHeight; seenSize.current = sizeOf(el); }, []);

  // Older entries were drawn above the reader (it runs when `shown` changes, which is when they appear): keep the same line in view, never
  // leave the position for a later streamed chunk to consume. Then a pending "go to this file" finishes.
  useLayoutEffect(() => {
    const el = log.current;
    if (!el) return;
    if (before.current !== undefined) { el.scrollTop = keepPosition(el.scrollTop, before.current, el.scrollHeight); before.current = undefined; seenSize.current = sizeOf(el); }
    const path = goAfter.current;
    goAfter.current = undefined;
    if (path) reveal(path);
  }, [shown]); // eslint-disable-line react-hooks/exhaustive-deps

  // The reader is at the bottom: new entries keep them there (one jump, never smooth: a smooth scroll fights the next chunk). Otherwise
  // nothing moves and the pill counts what arrived. An entry that only changed in place (a call that finished) adds none to the count and
  // moves nothing: `lines.length` is the same.
  useLayoutEffect(() => {
    const el = log.current;
    const total = held.trimmed + lines.length;
    const added = total - seen.current;
    seen.current = total;
    if (!el) return;
    if (follow.following) pin();
    else if (added > 0) setFollow((f) => grew(f, added));
  }, [lines, agent.state]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => { followingNow.current = follow.following; }, [follow.following]);
  // A scroll while following that came with a size change of the log or its content (a window or pane resize, the composer growing) is the
  // browser moving the offset, not the reader: back to the bottom, still following (#1060). Any other scroll is the reader's.
  const onScroll = useCallback(() => {
    const el = log.current;
    if (!el) return;
    if (followingNow.current && !readerActed.current && (resizing.current || resized(seenSize.current, el))) { pin(); return; }
    seenSize.current = sizeOf(el);
    const bottom = atBottom(el);
    followingNow.current = bottom;
    setFollow((f) => scrolled(f, bottom));
  }, [pin]);
  // Leaving the tail on purpose (older entries, a file's change, a fold opened) is the reader moving: the resize rule must not pull them back.
  const leave = useCallback(() => { followingNow.current = false; setFollow((f) => scrolled(f, false)); }, []);
  // The log or its content changed size: while following it stays on the newest entry, even when the resize sends no scroll event. A change
  // of the log's own height also marks the next frames' scroll events as the resize's (one more frame for a late clamp). A change right
  // after the reader clicked in the log is theirs: the fold they opened stays under the pointer, and the log leaves the tail if it is now
  // off its bottom.
  useEffect(() => {
    const el = log.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const frames = { resizing: 0, acted: 0 };
    const forTwoFrames = (flag: { current: boolean }, key: keyof typeof frames) => {
      flag.current = true;
      cancelAnimationFrame(frames[key]);
      frames[key] = requestAnimationFrame(() => { frames[key] = requestAnimationFrame(() => { flag.current = false; }); });
    };
    const observer = new ResizeObserver(() => {
      if (laidOut(seenSize.current) && el.clientHeight !== seenSize.current.clientHeight) forTwoFrames(resizing, 'resizing');
      if (followingNow.current && readerActed.current) { seenSize.current = sizeOf(el); if (!atBottom(el)) leave(); return; }
      if (followingNow.current) pin(); else seenSize.current = sizeOf(el);
    });
    observer.observe(el);
    if (el.firstElementChild) observer.observe(el.firstElementChild);
    const acted = () => forTwoFrames(readerActed, 'acted');
    el.addEventListener('click', acted, true);
    return () => { observer.disconnect(); el.removeEventListener('click', acted, true); cancelAnimationFrame(frames.resizing); cancelAnimationFrame(frames.acted); };
  }, [pin, leave]);
  const toLatest = () => { pin(); followingNow.current = true; setFollow(jumped()); };
  const earlier = () => { before.current = log.current?.scrollHeight; leave(); setShown((n) => n + PAGE); };
  const reveal = (path: string): boolean => {
    const target = [...(log.current?.querySelectorAll<HTMLElement>('[data-diff-path]') ?? [])].reverse().find((d) => d.dataset.diffPath === path);
    if (!target) return false;
    // The change may sit in a row of a folded group: open every fold around it.
    for (let el: HTMLElement | null = target; el; el = el.parentElement) if (el instanceof HTMLDetailsElement) el.open = true;
    target.scrollIntoView?.({ block: 'center' });
    const el = log.current;
    if (el && !atBottom(el)) leave();
    return true;
  };
  // A file whose change is older than what is drawn: draw earlier entries (keeping the position), then go to it.
  const goTo = (path: string) => { if (!reveal(path) && view.hidden > 0) { goAfter.current = path; earlier(); } };
  // A permission record points at the pinned request below the conversation: take the reader there (the request stays where it is answered).
  const showRequest = approvals ? () => { const el = pinned.current; if (el) { el.scrollIntoView?.({ block: 'nearest' }); el.focus(); } } : undefined;

  const go = async () => {
    const body = text.trim();
    // `/exit` and `/quit` end this agent (after a confirm in the page); they are never sent as text, unless the agent offers a command of that name.
    if (canClose && !agent.closed && isCloseCommand(body, commands)) { closeAsk.ask(agent.id, 'pane'); setText(''); return; }
    if (!body || !send || busy || agent.closed) return;
    setBusy(true);
    setSentMark(lines.length ? (lines[lines.length - 1] as ChatLine).id : 0);
    const r = await send(agent, body);
    setBusy(false);
    const res: PromptResult = typeof r === 'boolean' ? { ok: r } : r;
    setSent(res.ok ? (res.queued ? 'queued' : 'ok') : promptFailure(res.code));
    // Only what was sent is cleared: text typed while the send was in flight stays.
    if (res.ok) { setText((now) => (now.trim() === body ? '' : now)); toLatest(); }
  };
  // The composer grows with what is typed, up to a limit, then scrolls.
  const rowCount = Math.min(8, Math.max(2, text.split('\n').length));
  const working = agent.state === 'working' && !agent.closed;
  // #908: the groups of calls whose turn is over: a later prompt went, or the agent is in no turn (idle or finished, not `unknown`)
  // and has been for STOP_GRACE_MS (its state and its entries come on separate streams, and a call's last update can be held 150 ms).
  // What still says it runs there is drawn stopped.
  const inTurn = !agent.closed && agent.state !== 'idle' && agent.state !== 'finished';
  const [outOfTurn, setOutOfTurn] = useState(!inTurn);
  useEffect(() => {
    if (inTurn) { setOutOfTurn(false); return; }
    const timer = setTimeout(() => setOutOfTurn(true), STOP_GRACE_MS);
    return () => clearTimeout(timer);
  }, [inTurn]);
  const turnOver = !inTurn && outOfTurn;
  const ended = useMemo(() => endedGroups(view.items, !turnOver), [view.items, turnOver]);

  // ADR-0090. The window cannot read a node's capabilities, so "this node speaks the rich form" is what it has seen: rich entries, the
  // agent's settings or commands, or a prompt answered in the rich form. Stop turn is offered only then, while the agent works.
  const rich = isRich(lines) || !!actions.rich?.has(agent.id) || !!agent.session?.config?.length || commands.length > 0;
  const canStop = !!actions.cancelTurn && rich && working;
  const stopTurn = async (dropQueue: boolean) => {
    if (!actions.cancelTurn) return;
    setStopping('busy');
    setStopFailed(undefined);
    const r = await actions.cancelTurn(agent.id, dropQueue);
    setStopping(undefined);
    if (r !== true) setStopFailed(refusalKey(typeof r === 'string' ? r : '') ?? 'turn-stop-failed');
    if (r === true) setDropAsk(false);
  };

  // The "/" menu: open while the first word starts with "/" and the agent offers commands that fit; never in a closed agent.
  const query = slashQuery(text, caret);
  const matches = useMemo(() => (query === undefined || commands.length === 0 ? [] : filterCommands(commands, query)), [query, commands]);
  const menuOpen = matches.length > 0 && !slashOff && !agent.closed;
  const at = Math.min(slashAt, Math.max(0, matches.length - 1));
  const take = (i: number) => {
    const c = matches[i];
    if (!c) return;
    const next = applyCommand(text, caret, c);
    caretTo.current = next.caret;
    setText(next.text); setCaret(next.caret); setSlashAt(0);
    field.current?.focus();
  };
  useEffect(() => {
    if (caretTo.current === undefined || !field.current) return;
    field.current.setSelectionRange(caretTo.current, caretTo.current);
    caretTo.current = undefined;
  }, [text]);
  const sentText = sent && (sent !== 'queued' || queuedNote(sent, waitingCount, arrived, working)) && t({ ok: 'prompt-sent', queued: 'prompt-queued', full: 'prompt-queue-full', no_answer: 'prompt-no-answer', exited: 'prompt-agent-exited', failed: 'prompt-failed' }[sent]);

  return (
    <div ref={pane} className="flex h-full min-h-0 flex-col bg-surface-0">
      {/* Keyed by the agent: another agent on screen gets a fresh bar, so nothing left open carries over; usage updates keep it. */}
      <div className="flex-none border-b border-line bg-surface-1 px-4 py-2" data-chat-header>
        <SessionBar key={agent.id} part="status" agentKey={agent.id} session={agent.session ?? NO_SESSION} startedMs={agent.startedMs} disabled={agent.closed} state={agent.closed ? undefined : agent.state} tokens={agent.tokens} />
      </div>
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div ref={log} onScroll={onScroll} className="min-h-0 flex-1 overflow-y-auto px-4 pt-6 pb-4 max-md:pt-4" aria-live="polite" aria-label={t('chat-log')} role="log" tabIndex={0}>
          <div className="mx-auto flex max-w-195 flex-col gap-5 [&>*]:shrink-0">
            {lines.length === 0 && <p className="muted m-0">{t(source ? (send ? 'chat-empty' : 'chat-empty-watching') : (send ? 'prompt-intro' : 'prompt-intro-watching'))}</p>}
            {held.trimmed > 0 && <p role="note" className="muted m-0 self-center text-xs" data-trimmed>{t('chat-trimmed', { count: held.trimmed })}</p>}
            {view.hidden > 0 && <button type="button" className="button justify-self-center self-center px-3 py-1 text-xs" onClick={earlier}>{t('chat-earlier-items', { count: view.hidden })}</button>}
            {rows.map(({ row, paired }) => {
              if (row.kind !== 'turn' || !paired) return row.kind === 'turn' ? null : <Item key={row.id} item={row} notice={notice} queued={row.kind === 'user' ? (queue.notSent.has(row.line.id) ? 'not-sent' : row.line.queued ? 'waiting' : undefined) : undefined} />;
              // An edit and the diff it made are one row; every other item is drawn as it is. A group with a failed call, or right before a
              // request that waits for you, opens and says so, wherever it is.
              const draw = (item: TurnItem, i: number) => <Item key={item.id} item={item} running={working && item === last} ended={ended.has(item.id)} open={item === newestSteps} diffs={paired.attached}
                flag={item.kind === 'steps' ? flagOf(agent, item === newestSteps, item.lines, paired.items[i + 1]) : undefined} onShowRequest={showRequest} notice={notice} />;
              return <Turn key={row.id} name={agent.name} accent={agent.accent} kind={agent.kind || agent.name} model={model} atMs={row.atMs}>{paired.items.map(draw)}</Turn>;
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
      {approvals && <div ref={pinned} tabIndex={-1} className="mx-auto max-h-[40vh] w-full max-w-195 overflow-y-auto outline-none" data-pinned-approvals><PinnedShort.Provider value={short}>{approvals}</PinnedShort.Provider></div>}
      {queue.waiting.length > 0 && !agent.closed && (
        <div className="mx-auto flex w-full max-w-195 flex-col gap-1.5" data-queue>
          <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs text-muted">
            <span className="ide-chip h-5.5 px-2 font-sans text-ink" role="status">{t('queue-count', { count: queue.waiting.length })}</span>
            {/* The waiting prompt is the person's own text; shown as text, cut to one line. */}
            <span className="min-w-0 flex-1 truncate" title={(queue.waiting[0] as ChatLine).text} dir="auto">“{(queue.waiting[0] as ChatLine).text}”</span>
            <span className="max-md:hidden">{t('queue-note')}</span>
            {actions.cancelTurn && rich && !dropAsk && <button type="button" className="text-xs underline underline-offset-2 hover:text-ink" onClick={() => setDropAsk(true)}>{t('queue-drop')}</button>}
          </div>
          {dropAsk && (
            <div role="alertdialog" aria-label={t('queue-drop-title')} className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-s border border-danger/60 bg-surface-2 px-3 py-2">
              <div className="min-w-0 flex-1 basis-64 text-xs"><strong className="block">{t('queue-drop-title')}</strong><span className="text-muted">{t('queue-drop-body', { count: queue.waiting.length })}</span></div>
              <button type="button" className="button px-3 py-1 text-xs" disabled={stopping === 'busy'} onClick={() => void stopTurn(true)}>{t('queue-drop-yes')}</button>
              <button type="button" className="button button--primary px-3 py-1 text-xs" onClick={() => setDropAsk(false)}>{t('queue-drop-no')}</button>
            </div>
          )}
        </div>
      )}
      <form className="relative mx-auto flex w-full max-w-195 flex-none flex-col gap-2 rounded-l border border-line bg-surface-1 p-2.5 shadow-card transition-colors focus-within:border-accent" onSubmit={(e) => { e.preventDefault(); void go(); }}>
        {menuOpen && (
          <ul id="slash-menu" role="listbox" aria-label={t('slash-menu')} className="absolute start-0 end-0 bottom-full z-20 m-0 mb-1.5 grid max-h-64 list-none gap-px overflow-y-auto rounded-m border border-line bg-surface-1 p-1 shadow-card">
            {matches.map((c, i) => (
              <li key={`${i}:${c.name}`} id={`slash-opt-${i}`} role="option" aria-selected={i === at} data-slash-option
                className={`flex min-h-9 cursor-pointer items-baseline gap-2.5 rounded-xs px-2.5 py-1.5 text-[0.8125rem] ${i === at ? 'bg-accent/15 text-ink' : 'text-muted hover:bg-surface-2'}`}
                // Pressing here must not move the caret out of the box before the choice is taken.
                onPointerDown={(e) => e.preventDefault()} onClick={() => take(i)}>
                <code dir="ltr" className="flex-none font-mono text-xs font-semibold text-ink">/{c.name.replace(/^\/+/, '')}</code>
                {c.inputHint && <span dir="auto" className="flex-none truncate text-xs text-muted italic max-md:hidden">{c.inputHint}</span>}
                <span dir="auto" className="min-w-0 flex-1 truncate text-xs">{c.description}</span>
              </li>
            ))}
          </ul>
        )}
        {!send && !agent.closed && <p className="m-0 p-1 text-sm text-muted" data-watching>{t('prompt-watching')}</p>}
        {send && <textarea ref={field} value={text} onChange={(e) => { setText(e.target.value); setCaret(e.target.selectionStart); setSlashOff(false); setSlashAt(0); }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
          aria-label={t('prompt-label')} placeholder={t(commands.length > 0 ? 'prompt-placeholder-commands' : 'prompt-label')} rows={rowCount}
          {...(commands.length > 0 ? { role: 'combobox' as const, 'aria-autocomplete': 'list' as const, 'aria-haspopup': 'listbox' as const, 'aria-expanded': menuOpen, 'aria-controls': menuOpen ? 'slash-menu' : undefined, 'aria-activedescendant': menuOpen ? `slash-opt-${at}` : undefined } : {})}
          className="w-full resize-none border-0 bg-transparent p-1 text-ink outline-none focus-visible:outline-none"
          disabled={agent.closed}
          onCompositionEnd={() => { composedAt.current = Date.now(); }}
          onKeyDown={(e) => {
            // The menu takes the arrows, Enter or Tab (to choose) and Escape; not while an input method is composing.
            const composing = e.nativeEvent.isComposing || e.keyCode === 229 || Date.now() - composedAt.current <= IME_GRACE_MS;
            if (menuOpen && !composing && !(canClose && e.key === 'Enter' && isCloseCommand(text, commands))) {
              if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); setSlashAt(stepIndex(at, e.key === 'ArrowDown' ? 1 : -1, matches.length)); return; }
              if ((e.key === 'Enter' && !e.shiftKey) || (e.key === 'Tab' && !e.shiftKey)) { e.preventDefault(); take(at); return; }
              if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setSlashOff(true); return; }
            }
            // Enter sends, Shift+Enter starts a new line; a key that only confirms an input method's candidate is not a send (Safari too).
            if (e.key === 'Enter' && !e.shiftKey) e.preventDefault();
            if (sendsOnEnter({ key: e.key, shiftKey: e.shiftKey, isComposing: e.nativeEvent.isComposing, keyCode: e.keyCode }, Date.now() - composedAt.current)) void go();
          }} />}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
          {agent.session && <SessionBar key={agent.id} part="controls" agentKey={agent.id} session={agent.session} disabled={agent.closed} choose={choose} setConfig={actions.setConfig} approvals={actions.approvals} launchedFree={agent.skipsPermissions} />}
          {/* With a session the bar draws the sign beside Mode (`launchedFree`); without one it leads the row. */}
          {!agent.session && <NoAskBadge agent={agent} look="tag" />}
          <AsksLessChip agent={agent} />
          <span className="flex-1" />
          {sentText && <span role="status" className={`text-xs ${sent === 'ok' || sent === 'queued' ? 'text-finished' : 'text-danger'}`}>{sentText}</span>}
          {stopFailed && <span role="status" className="text-xs text-danger">{t(stopFailed)}</span>}
          {(send || agent.closed) && <span className="hidden text-xs text-muted md:inline">{t(agent.closed ? 'prompt-closed' : 'prompt-hint')}</span>}
          {/* Ends this turn only: the agent stays and can be asked again. Not the same as stopping the agent. */}
          {canStop && <button type="button" className="button gap-1.5 px-3 text-removed" disabled={stopping === 'busy'} title={t('turn-stop-hint')} aria-label={`${t('turn-stop')}. ${t('turn-stop-hint')}`} onClick={() => void stopTurn(false)}>
            <svg aria-hidden viewBox="0 0 16 16" className="size-3.5 fill-current"><rect x="3.5" y="3.5" width="9" height="9" rx="1.5" /></svg>{t('turn-stop')}
          </button>}
          {send && <button type="submit" className="button button--primary" disabled={busy || !text.trim() || agent.closed}>{t('prompt-send')}</button>}
        </div>
      </form>
      </div>
    </div>
  );
}
