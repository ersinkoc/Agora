import * as Dialog from '@radix-ui/react-dialog';
import { createContext, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import { t } from '../i18n';
import type { AgentTileModel } from '../model';
import { visible } from '../net/hidden';
import { ago } from '../net/time';
import type { NoticeRowModel } from '../net/notices';
import type { MessageModel } from '../net/view';
import { OWNER_MAX, plain, TITLE_MAX } from './boardColumns';
import { Icon } from './Icon';
import { AgentAvatar, Tag } from './kit';

// The team chat (the mesh, in the UI's words): what agents wrote to each other, newest first. Each row is the sender's avatar, who wrote to
// whom, how long ago, and the text cut to two lines; the row opens the whole text in a reading view (Esc closes it). Agent text is untrusted
// data (threat model): it is drawn only as text, never as HTML or markdown, a link in it is not made clickable, and control and bidirectional
// characters are removed from it and from the names (as on the board).
// ADR-0130: `notices` are the hub's own rows in a project's channel, one per accepted task (the task's title from the board, the project,
// the time; never who gave or accepted it). They sit among the messages by time and open nothing; they wake nobody (nothing that watches
// new messages sees them).
export function TeamChat({ messages, notices = [], fleet = [], now }: { messages: readonly MessageModel[]; notices?: readonly NoticeRowModel[]; fleet?: readonly AgentTileModel[]; now: number }) {
  const [reading, setReading] = useState<MessageModel>();
  // The row the view was opened from gets focus back when it closes (a dialog with no Radix trigger would otherwise drop it on the page);
  // if newer messages replaced that row meanwhile, the newest row does.
  const opener = useRef<HTMLElement | null>(null);
  const sender = (m: MessageModel) => (m.fromId === undefined ? undefined : fleet.find((a) => a.id === m.fromId));
  return <>
    <ul className="m-0 grid list-none gap-0.5 p-0" aria-label={t('ide-tab-messages')}>
      {merged(messages, notices).map((r) => r.notice
        ? <li key={`notice-${r.notice.id}`}><NoticeRow n={r.notice} now={now} /></li>
        : <li key={`${r.m.atMs}-${r.at}`}><MessageRow m={r.m} agent={sender(r.m)} now={now} onOpen={(row) => { opener.current = row; setReading(r.m); }} /></li>)}
    </ul>
    {reading && <MessageReader m={reading} agent={sender(reading)} now={now} onClose={() => setReading(undefined)} returnTo={opener} />}
  </>;
}

/** The owner's channel notices (ADR-0130), given by the window to every Team chat it draws (the side panel, a layout panel, the phone). */
export const ChannelNotices = createContext<readonly NoticeRowModel[]>([]);

// `at` is a message's index in `messages`, so its row keeps its key when a notice is placed above it.
type Row = { notice: NoticeRowModel; m?: undefined } | { notice?: undefined; m: MessageModel; at: number };

/** Both lists newest first, as one: a notice goes before a message of the same time. */
export function merged(messages: readonly MessageModel[], notices: readonly NoticeRowModel[]): Row[] {
  const out: Row[] = [];
  let i = 0;
  let j = 0;
  while (i < messages.length || j < notices.length) {
    const m = messages[i];
    const n = notices[j];
    if (n && (!m || n.atMs >= m.atMs)) { out.push({ notice: n }); j++; } else if (m) { out.push({ m, at: i }); i++; }
  }
  return out;
}

function NoticeRow({ n, now }: { n: NoticeRowModel; now: number }) {
  return <div className="team-msg team-msg--notice" data-notice-id={n.id}>
    <span aria-hidden className="kit-chip kit-tone-finished team-msg__mark"><Icon name="check" className="size-3.5" /></span>
    <span className="team-msg__line">
      <span className="team-msg__from">{visible(n.channel)}</span>
      <span className="team-msg__to">{t('team-chat-notice-accepted')}</span>
      <When atMs={n.atMs} now={now} style="short" />
    </span>
    <span dir="auto" className="team-msg__body">{n.title !== undefined ? plain(n.title, TITLE_MAX) || t('board-task-untitled') : t('team-chat-notice-unseen')}</span>
    <span className="team-msg__foot"><span className="team-msg__tag" title={visible(n.project)}><Tag>{plain(n.project, OWNER_MAX)}</Tag></span></span>
  </div>;
}

// A sender this window does not list (another machine's agent, one already cleared) gets a neutral avatar with the first letter of its name.
function Avatar({ m, agent, size }: { m: MessageModel; agent?: AgentTileModel; size?: 'm' | 'l' }) {
  return <AgentAvatar accent={agent?.accent ?? 'var(--agent-slate)'} kind={agent?.kind ?? m.from.split('/').pop() ?? m.from} size={size} />;
}

function Meta({ m }: { m: MessageModel }) {
  return <>
    {m.crossMachine ? <span className="team-msg__tag" title={t('message-cross')}><Tag>{m.toMachine ? visible(m.toMachine) : t('message-cross')}</Tag><span className="sr-only">{t('message-cross')}</span></span> : null}
    {m.via ? <span className="team-msg__tag" title={t('message-via', { machine: visible(m.via) })}><Tag>{t('message-via', { machine: visible(m.via) })}</Tag></span> : null}
  </>;
}

function When({ atMs, now, style }: { atMs: number; now: number; style: 'long' | 'short' }) {
  if (!atMs) return null;
  return <time className="team-msg__time" dateTime={new Date(atMs).toISOString()} title={new Date(atMs).toLocaleString()}>{ago(atMs, now, style)}</time>;
}

function MessageRow({ m, agent, now, onOpen }: { m: MessageModel; agent?: AgentTileModel; now: number; onOpen: (row: HTMLElement) => void }) {
  // "Show all" only when the two lines really cut the text: measured in the page (and again when the panel is resized).
  const body = useRef<HTMLSpanElement>(null);
  const [cut, setCut] = useState(false);
  useLayoutEffect(() => {
    const el = body.current;
    if (!el) return;
    const check = () => setCut(el.scrollHeight > el.clientHeight + 1);
    check();
    if (typeof ResizeObserver === 'undefined') return;
    const watch = new ResizeObserver(check);
    watch.observe(el);
    return () => watch.disconnect();
  }, [m.body]);
  return <button type="button" className="team-msg" aria-haspopup="dialog" onClick={(e) => onOpen(e.currentTarget)}>
    <Avatar m={m} agent={agent} />
    <span className="team-msg__line">
      <span className="team-msg__from">{visible(m.from)}</span>
      <span className="team-msg__to">{t('team-chat-to', { name: visible(m.to) })}</span>
      <When atMs={m.atMs} now={now} style="short" />
    </span>
    <span ref={body} className="team-msg__body" data-team-msg-body>{visible(m.body)}</span>
    {(cut || m.crossMachine || m.via) && <span className="team-msg__foot">
      {cut && <span className="team-msg__more" aria-hidden>{t('team-chat-show-all')}</span>}
      <Meta m={m} />
    </span>}
  </button>;
}

// The whole message, as text, in a dialog: Esc or Close returns to the row it was opened from. Escape stops here, so the side panel that
// holds the list stays open under it.
function MessageReader({ m, agent, now, onClose, returnTo }: { m: MessageModel; agent?: AgentTileModel; now: number; onClose: () => void; returnTo: RefObject<HTMLElement | null> }) {
  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-surface-0/70 backdrop-blur-sm" />
        <Dialog.Content aria-describedby={undefined} className="team-reader"
          onCloseAutoFocus={(e) => { e.preventDefault(); const row = returnTo.current; (row?.isConnected ? row : document.querySelector<HTMLElement>('button.team-msg'))?.focus(); }} onKeyDown={(e) => { if (e.key === 'Escape') e.stopPropagation(); }}>
          <header className="team-reader__head">
            <Avatar m={m} agent={agent} size="l" />
            <div className="min-w-0 flex-1">
              <Dialog.Title className="team-reader__title">{visible(m.from)}</Dialog.Title>
              <p className="team-msg__line m-0"><span className="team-msg__to">{t('team-chat-to', { name: visible(m.to) })}</span><Meta m={m} /><When atMs={m.atMs} now={now} style="long" /></p>
            </div>
            <Dialog.Close className="ide-iconbtn flex-none" aria-label={t('team-chat-close')} title={t('team-chat-close')}><Icon name="close" /></Dialog.Close>
          </header>
          <div className="team-reader__body" data-team-reader-body>{visible(m.body)}</div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
