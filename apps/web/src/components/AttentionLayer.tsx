import * as Dialog from '@radix-ui/react-dialog';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { ATTENTION_DWELL_MS, type AttentionMode, attentionPrefs, isTypingTarget, mayInterrupt, modalKind, notifyDelay, notifyText, shorten, shownKey, WHO_MAX, whoOf, type NotifyMode, type Presence } from '../net/attention';
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

// "Needs you" must never be invisible (docs/ux/main-window.md): the list, the counts and the tab stay where they are; this layer adds, by
// this device's choice, a card over the window and a notification while the window is in the background. It holds only memory: what was
// put off with Later, what is open and when it appeared are not stored anywhere.

type Resolve = (a: ApprovalModel, approved: boolean, choice?: undefined, always?: boolean) => Promise<unknown> | unknown;

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
  // Goes to the full card of an item (its agent, or the Needs you list).
  onReview: (it: AttentionItem) => void;
}) {
  const mode = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.mode(), () => 'modal' as AttentionMode);
  const notifyMode = useSyncExternalStore(attentionPrefs.subscribe, () => attentionPrefs.notify(), () => 'off' as NotifyMode);
  const [later, setLater] = useState<ReadonlySet<string>>(new Set());
  const [openKey, setOpenKey] = useState<string | undefined>();
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
    if (open) return;
    const first = candidates[0];
    if (first && mayInterrupt(read())) setOpenKey(first.key);
    else if (openKey !== undefined) setOpenKey(undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modalOn, open, candidates, tick]);

  // Showing what needs the person, on their own act (a click on the banner or on a notification): the card opens whatever they were doing.
  const latest = useRef({ items, candidates, modalOn, onReview });
  latest.current = { items, candidates, modalOn, onReview };
  const reveal = () => {
    const c = latest.current;
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
  const showBanner = !phone && mode !== 'off' && !open && !listShown && (mode === 'banner' ? items.length > 0 : queued > 0);
  return (
    <>
      {showBanner && (
        <div role="status" aria-live="polite" className="attention-banner fixed bottom-4 end-4 z-30 flex max-w-[min(26rem,calc(100vw-2rem))] items-center gap-3 rounded-l border border-line bg-surface-1 px-3 py-2 shadow-card">
          <span className="min-w-0 flex-1 text-sm font-medium text-needs-you">{t('attention-banner', { count: mode === 'banner' ? items.length : queued })}</span>
          <button type="button" className="button" onClick={reveal}>{t('attention-review')}</button>
        </div>
      )}
      {open && <AttentionCard item={open} more={items.length - 1} stale={stale} onResolve={onResolve} onReview={(it) => { putOff(open.key); onReview(it); }} onLater={() => putOff(open.key)} />}
    </>
  );
}

const canNotify = () => typeof Notification !== 'undefined' && Notification.permission === 'granted';

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
  // What is displayed has a dwell of its own, and every change of it (another request, the same one with other words, another agent or
  // machine) starts it again, and so does the person coming back to the window (focus, or the tab shown again) and the link coming back:
  // a click that was aimed at something else must not land on Allow. Derived from what is shown, so no frame has a new request ready.
  const key = shownKey(item);
  const [epoch, setEpoch] = useState(0);
  const [readyFor, setReadyFor] = useState<string | undefined>();
  useEffect(() => {
    const again = () => setEpoch((n) => n + 1);
    window.addEventListener('focus', again);
    const seen = () => { if (document.visibilityState === 'visible') again(); };
    document.addEventListener('visibilitychange', seen);
    return () => { window.removeEventListener('focus', again); document.removeEventListener('visibilitychange', seen); };
  }, []);
  useEffect(() => {
    const h = setTimeout(() => setReadyFor(key), ATTENTION_DWELL_MS);
    return () => { clearTimeout(h); setReadyFor(undefined); };
  }, [key, epoch, stale]);
  // A request is answered once: after one answer its buttons stay dead until it leaves the list or what it shows changes.
  const [answered, setAnswered] = useState<string | undefined>();
  // The single rule for Allow and Deny: a plain request, whole on screen, past its dwell, not answered yet, the link up.
  const canAnswer = !!x && !!onResolve && readyFor === key && answered !== key && !stale;
  const answer = (ok: boolean, always = false) => {
    if (!canAnswer || !x || !onResolve) return;
    setAnswered(key);
    void Promise.resolve(always ? onResolve(x, ok, undefined, true) : onResolve(x, ok));
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
              {kind === 'answer' && x && <AnswerButtons approval={x} off={{ 'aria-disabled': !canAnswer, 'aria-describedby': stale ? why : undefined }} alwaysNote={alwaysNoteId(x, 'attention')}
                onDeny={() => answer(false)} onAllow={() => answer(true)}
                // ADR-0101: offered as well as a single answer (a request that offers only "always" is read in its full card, `decidableInline`).
                onAlways={x.always && !x.always.only ? () => answer(true, true) : undefined} />}
              {(kind === 'review' || (kind === 'answer' && answered === key)) && <Button variant="primary" onClick={() => onReview(item)}>{t('attention-review')}</Button>}
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
            {kind === 'answer' && answered === key && <p role="status" className="attention-sent m-0 text-sm text-muted">{t('attention-sent')}</p>}
            {stale && kind === 'answer' && <p id={why} className="m-0 text-sm text-muted">{t('fleet-stale-no-answer')}</p>}
          </ApprovalRequest>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
