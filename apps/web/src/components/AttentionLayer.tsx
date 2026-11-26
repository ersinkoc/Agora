import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { type AttentionMode, attentionPrefs, isTypingTarget, mayInterrupt, modalKind, notifyDelay, notifyText, shorten, shownKey, WHO_MAX, whoOf, type NotifyMode, type Presence } from '../net/attention';
import { appNotify } from '../net/desktopNotify';
import { titleWithCount } from '../net/alert';
import { type AttentionItem, askLine } from '../net/fleet';
import { approvalName } from '../net/approvalText';
import { pairTitle } from './pairChoice';
import { AlwaysNote, alwaysNoteId } from './AllowAlways';
import { type ApprovalModel, visible } from '../net/view';
import { needsPanel } from '../net/needsPanel';
import { kindOf } from './AgentTile';
import { AnswerButtons, ApprovalRequest, ASK_ICON, askKind, askTitle } from './ApprovalRequest';
import { AgentAvatar, Button, Tag } from './kit';
import { ReviewDialog, type ReviewResolve } from './ReviewRequest';
import type { StartChoice } from './StartApproval';
import { ShareHolds, useAnswerGuard, useAnswerHolds } from './useAnswerGuard';

// "Needs you" must never be invisible (docs/ux/main-window.md): the list, the counts and the tab stay where they are; this layer adds, by
// this device's choice, a card over the window and a notification while the window is in the background. It holds only memory: what was
// put off with Later, what is open and when it appeared are not stored anywhere.

// `choice`: a start's grant, chosen in Review (#1070); the card itself never sends one.
type Resolve = (a: ApprovalModel, approved: boolean, choice?: StartChoice, always?: boolean) => Promise<unknown> | unknown;

// #1070: a start or a switch to act without asking is read in Review, opened from this card in its place (one Review, not the agent's card
// and a second Review there). A switch the window cannot read keeps the old way, to its full card.
const reviewsHere = (it: AttentionItem): it is Extract<AttentionItem, { kind: 'approval' }> => it.kind === 'approval' && !!(it.approval.start || it.approval.skip);

// What the person is doing, read when it matters: whether the window is the one in front, and whether keys are going into a field or a
// terminal. Re-read on every focus change, every typed key and, while something waits, twice a second.
function usePresence(waiting: boolean) {
  const lastKey = useRef(-Infinity);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const bump = () => setTick((n) => n + 1);
    const typed = (e: Event) => { if (isTypingTarget(e.target as Element | null)) { lastKey.current = Date.now(); } };
    const keys = ['keydown', 'input', 'compositionupdate'] as const;
    for (const k of keys) window.addEventListener(k, typed, true);
    const bumps = ['focus', 'blur', 'focusin', 'focusout'] as const;
    for (const k of bumps) window.addEventListener(k, bump, true);
    document.addEventListener('visibilitychange', bump);
    return () => {
      for (const k of keys) window.removeEventListener(k, typed, true);
      for (const k of bumps) window.removeEventListener(k, bump, true);
      document.removeEventListener('visibilitychange', bump);
    };
  }, []);
  useEffect(() => {
    if (!waiting) return;
    const h = setInterval(() => setTick((n) => n + 1), 500);
    return () => clearInterval(h);
  }, [waiting]);
  const read = (): Presence => ({ focused: inFront(), typing: isTypingTarget(document.activeElement), lastKeyMs: lastKey.current, nowMs: Date.now() });
  return { read, tick };
}
const inFront = () => document.visibilityState === 'visible' && document.hasFocus();

export function AttentionLayer({ items, stale = false, phone = false, onResolve, onReview }: {
  items: readonly AttentionItem[];
  stale?: boolean;
  phone?: boolean;
  onResolve?: Resolve;
  // Goes to the full card of an item (its agent, or the Needs you list). A start or a switch is read in Review here instead (#1070).
  onReview: (it: AttentionItem) => void;
}) {
  const mode = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.mode(), () => 'modal' as AttentionMode);
  const notifyMode = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.notify(), () => 'off' as NotifyMode);
  const [later, setLater] = useState<ReadonlySet<string>>(new Set());
  const [openKey, setOpenKey] = useState<string | undefined>();
  // The start or switch whose Review this layer opened, by request id: drawn from the list as it is now, so a request that leaves the list
  // takes the dialog with it. While it is open no card opens over it.
  const [reviewId, setReviewId] = useState<string | undefined>();
  const reviewing = reviewId === undefined ? undefined : items.find((i): i is Extract<AttentionItem, { kind: 'approval' }> => reviewsHere(i) && i.approval.id === reviewId);
  const holds = useAnswerHolds([]);
  // Gone from the list: forgotten, so the same request asked again later does not open Review by itself.
  useEffect(() => { if (reviewId !== undefined && !reviewing) setReviewId(undefined); }, [reviewId, reviewing]);
  const candidates = useMemo(() => items.filter((i) => modalKind(i) && !later.has(i.key)), [items, later]);
  const modalOn = mode === 'modal' && !phone;
  const open = modalOn ? items.find((i) => i.key === openKey && modalKind(i)) : undefined;
  const { read, tick } = usePresence(modalOn && !open && candidates.length > 0);

  // The tab title and the app badge carry the count whatever the choice: they are the quietest way and they are never taken away.
  const baseTitle = useRef(document.title);
  useEffect(() => {
    document.title = titleWithCount(baseTitle.current, items.length);
    try { const nav = navigator as Navigator & { setAppBadge?: (n?: number) => Promise<void>; clearAppBadge?: () => Promise<void> }; void (items.length > 0 ? nav.setAppBadge?.(items.length) : nav.clearAppBadge?.())?.catch(() => {}); } catch { /* no badge here */ }
  }, [items.length]);

  // Things put off with Later come back when they are gone and asked again; a gone one is forgotten.
  useEffect(() => {
    const here = new Set(items.map((i) => i.key));
    setLater((l) => (l.size === 0 || [...l].every((k) => here.has(k)) ? l : new Set([...l].filter((k) => here.has(k)))));
  }, [items]);

  // A card opens only while the window is in front and nobody is typing; one that is open stays what it is until it is answered, sent
  // to Review or put off (a newer request waits behind it), so what is under the pointer never changes by itself.
  useEffect(() => {
    if (!modalOn) { if (openKey !== undefined) setOpenKey(undefined); return; }
    if (open || reviewing) return;
    const first = candidates[0];
    if (first && mayInterrupt(read())) setOpenKey(first.key);
    else if (openKey !== undefined) setOpenKey(undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modalOn, open, reviewing, candidates, tick]);

  // Showing what needs the person, on their own act (a click on the banner or on a notification): the card opens whatever they were doing.
  const latest = useRef({ items, candidates, modalOn, onReview, reviewing });
  latest.current = { items, candidates, modalOn, onReview, reviewing };
  const reveal = () => {
    const c = latest.current;
    if (c.reviewing) return; // Review is open over the window already

    const all = c.items.filter((i) => modalKind(i));
    if (c.modalOn && all[0]) { setLater(new Set()); setOpenKey(all[0].key); return; }
    if (c.items[0]) c.onReview(c.items[0]);
  };

  // A notification while the window is in the background, once per burst, only when the person turned it on here.
  // A request counts as told about only once a notification went out for it (or it was answered and left the list): one that arrived while the
  // window was in front, and is still unanswered when the window goes to the background, is told about then.
  const notified = useRef<Set<string>>(new Set());
  const lastSent = useRef<number | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const live = useRef({ items, notifyMode });
  live.current = { items, notifyMode };
  useEffect(() => {
    const here = new Set(items.map((i) => i.key));
    notified.current = new Set([...notified.current].filter((k) => here.has(k)));
    const fresh = items.filter((i) => !notified.current.has(i.key));
    if (fresh.length === 0 || mode === 'off' || notifyMode === 'off' || !canNotify() || timer.current !== undefined) return;
    const send = () => {
      timer.current = undefined;
      const now = live.current;
      if (now.items.length === 0 || inFront() || !canNotify()) return;
      lastSent.current = Date.now();
      notified.current = new Set(now.items.map((i) => i.key));
      show(now.items, now.notifyMode, () => { window.focus(); reveal(); });
    };
    // `send` goes out only while the window is in the background; when it is in front, the next change of focus looks again.
    const wait = notifyDelay(lastSent.current, Date.now());
    if (wait === 0) send(); else timer.current = setTimeout(send, wait);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items, mode, notifyMode, tick]);
  useEffect(() => () => { if (timer.current !== undefined) clearTimeout(timer.current); }, []);

  const putOff = (key: string) => { setLater((l) => new Set(l).add(key)); setOpenKey(undefined); };

  // The banner: not modal, in a corner, never over what is typed into. Banner mode shows it whenever something needs the person; modal
  // mode shows it for what the card is holding back (waiting for focus, or put off).
  // Hidden while a Needs you list is on screen (the side panel on that subject, or a panel of the layout): it would be a third copy.
  const listShown = useSyncExternalStore(needsPanel.subscribe, needsPanel.isShown, () => false);
  const queued = items.filter((i) => modalKind(i) && i.key !== open?.key).length;
  const showBanner = !phone && mode !== 'off' && !open && !reviewing && !listShown && (mode === 'banner' ? items.length > 0 : queued > 0);
  return (
    <>
      {showBanner && (
        <div role="status" aria-live="polite" className="attention-banner fixed bottom-4 end-4 z-30 flex max-w-[min(26rem,calc(100vw-2rem))] items-center gap-3 rounded-l border border-line bg-surface-1 px-3 py-2 shadow-card">
          <span className="min-w-0 flex-1 text-sm font-medium text-needs-you">{t('attention-banner', { count: mode === 'banner' ? items.length : queued })}</span>
          <button type="button" className="button" onClick={reveal}>{t('attention-review')}</button>
        </div>
      )}
      {open && <AttentionCard item={open} more={items.length - 1} stale={stale} onResolve={onResolve} onLater={() => putOff(open.key)}
        onReview={(it) => { putOff(open.key); if (reviewsHere(it)) setReviewId(it.approval.id); else onReview(it); }} />}
      {/* Review in the card's place: its own dwell and guards (a start's `useAnswerGuard`, a switch's `useSkipAnswer`), in the window's
          holds, with the raw answer. Closing it leaves the request put off, as Later does (the banner brings it back). */}
      {reviewing && <ShareHolds holds={holds}>
        <ReviewDialog approval={reviewing.approval} agent={reviewing.agent} stale={stale} onResolve={onResolve ? (ok, choice) => onResolve(reviewing.approval, ok, choice) as ReturnType<ReviewResolve> : undefined} onClose={() => setReviewId(undefined)} />
      </ShareHolds>}
    </>
  );
}

// Not inside a desktop app with notifications of its own (net/desktopNotify.ts): the page has no permission for the browser's there.
const canNotify = () => !appNotify.supported() && typeof Notification !== 'undefined' && Notification.permission === 'granted';

// Generic by default; the names only when asked for. Never the request: see `notifyText`.
function show(items: readonly AttentionItem[], how: NotifyMode, onClick: () => void) {
  const text = notifyText(items, how);
  try {
    const n = new Notification(t(text.title.id), { body: t(text.body.id, text.body.params), tag: text.tag });
    n.onclick = () => { try { n.close(); } catch { /* already gone */ } onClick(); };
  } catch { /* not allowed here */ }
}

function AttentionCard({ item, more, stale, onResolve, onReview, onLater }: {
  item: AttentionItem;
  more: number;
  stale: boolean;
  onResolve?: Resolve;
  onReview: (it: AttentionItem) => void;
  onLater: () => void;
}) {
  const kind = modalKind(item);
  const x = item.kind === 'approval' ? item.approval : undefined;
  // T44 through the shared guard (`useAnswerGuard`, as every other surface; #1052): what is displayed has a dwell of its own, and every change
  // of it (another request, the same one with other words, another agent or machine) starts it again, and so do the person coming back to
  // the window (focus, or the tab shown again: at once, so a press in the same batch sends nothing) and the link coming back. A click that
  // was aimed at something else must not land on Allow. The holds are the window's (#1048): a request answered, or on its way, from any
  // surface is not answerable here, one answered here is held there; while an answer goes the buttons are off, "Answer sent" (with Review)
  // only once it went, and a failed answer is answerable again after a fresh dwell.
  const key = shownKey(item);
  const guard = useAnswerGuard(x ? [x.id] : [], (status) => JSON.stringify([key, x ? status(x.id) : '']), stale);
  // The single rule for Allow and Deny: a plain request, whole on screen, past its dwell, not held by the window, the link up.
  const canAnswer = kind === 'answer' && !!x && !!onResolve && guard.canAnswer(x.id);
  const sentHere = !!x && guard.status(x.id) === 'sent';
  const answer = (ok: boolean, always = false) => {
    if (kind !== 'answer' || !x || !onResolve) return;
    guard.answer(x.id, () => (always ? onResolve(x, ok, undefined, true) : onResolve(x, ok)));
  };
  const content = useRef<HTMLDivElement>(null);
  // Said to a screen reader when the card opens (the card is an alert dialog; this is the short version of why).
  const [announce, setAnnounce] = useState('');
  useEffect(() => { setAnnounce(t('notify-body', { count: 1 })); }, []);
  const w = whoOf(item);
  const name = shorten(w.name || (x ? approvalName(x) : ''), WHO_MAX);
  const machine = shorten(w.machine, WHO_MAX);
  const agent = item.kind === 'pair' ? undefined : item.agent;
  const why = 'attention-why';
  // The title is the ask itself, in the window's words ("codex · teal wants to run a command"), with the agent's avatar and the machine
  // as a tag; the request's own words are under it, whole (`attention-what`).
  const title = item.kind === 'pair' ? pairTitle(item.card) : x ? askTitle(x, name) : name;
  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) onLater(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content ref={content} role="alertdialog" tabIndex={-1} aria-describedby="attention-what" onOpenAutoFocus={(e) => { e.preventDefault(); content.current?.focus(); }}
          className="attention-card fixed left-1/2 top-1/2 z-50 flex max-h-[calc(100vh-2rem)] w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-m shadow-pop outline-none">
          <ApprovalRequest density="full" icon={item.kind === 'pair' ? 'machine' : x ? ASK_ICON[askKind(x)] : 'bell'}
            titleWrap={(head) => <Dialog.Title asChild>{head}</Dialog.Title>}
            avatar={agent || x ? <AgentAvatar accent={agent?.accent || 'var(--agent-slate)'} kind={kindOf({ kind: agent?.kind, name })} /> : undefined}
            title={<span className="attention-who break-words">{title}</span>}
            where={machine ? <Tag>{machine}</Tag> : undefined}
            actions={<>
              <Button onClick={onLater}>{t('attention-later')}</Button>
              {/* ADR-0106: a viewer's window (no `onResolve`) shows the request with no answers: the hub would refuse them. */}
              {kind === 'answer' && x && onResolve && <AnswerButtons approval={x} off={{ 'aria-disabled': !canAnswer, 'aria-describedby': stale ? why : undefined }} alwaysNote={alwaysNoteId(x, 'attention')}
                onDeny={() => answer(false)} onAllow={() => answer(true)}
                // ADR-0101: offered as well as a single answer (a request that offers only "always" is read in its full card, `decidableInline`).
                onAlways={x.always && !x.always.only ? () => answer(true, true) : undefined} />}
              {(kind === 'review' || (kind === 'answer' && sentHere)) && <Button variant="primary" onClick={() => onReview(item)}>{t('attention-review')}</Button>}
            </>}>
            <p aria-live="assertive" className="attention-announce sr-only">{announce}</p>
            <div id="attention-what" className="min-h-0 overflow-auto">
              {kind === 'answer' && x && <p className="attention-ask m-0 whitespace-pre-wrap break-words font-mono text-sm text-ink">{visible(askLine(x.action, x.detail))}</p>}
              {kind === 'answer' && x?.always && <AlwaysNote approval={x} surface="attention" className="mt-2" />}
              {/* The title already says who wants to start another agent; this says where it is read. */}
              {kind === 'review' && x && x.start && <p className="m-0 text-sm text-ink">{t('attention-start-read')}</p>}
              {kind === 'review' && x && (x.skip || x.skipUnknown) && <p className="m-0 text-sm text-ink">{t('attention-skip', { name })}</p>}
              {kind === 'review' && x && !x.start && !x.skip && !x.skipUnknown && <>
                <p className="m-0 mb-2 text-sm text-ink">{t('attention-ask-review')}</p>
                <pre className="attention-ask m-0 whitespace-pre-wrap break-words font-mono text-sm text-ink">{visible(askLine(x.action, x.detail))}</pre>
              </>}
              {item.kind === 'pair' && <p className="m-0 text-sm text-ink">{t('attention-pair')}</p>}
            </div>
            {more > 0 && <p className="m-0 text-xs text-muted">{t('attention-more', { count: more })}</p>}
            {kind === 'answer' && sentHere && <p role="status" className="attention-sent m-0 text-sm text-muted">{t('attention-sent')}</p>}
            {stale && kind === 'answer' && onResolve && <p id={why} className="m-0 text-sm text-muted">{t('fleet-stale-no-answer')}</p>}
            {kind === 'answer' && !onResolve && <p className="m-0 text-sm text-muted" data-watching>{t('approval-watching')}</p>}
          </ApprovalRequest>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
