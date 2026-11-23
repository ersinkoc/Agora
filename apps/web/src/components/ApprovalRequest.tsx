import { createContext, useCallback, useContext, useId, useLayoutEffect, useRef, useState, type ReactElement, type ReactNode, type RefObject } from 'react';
import { t } from '../i18n';
import { approvalName } from '../net/approvalText';
import type { ApprovalModel } from '../net/view';
import { allowAlwaysWord } from './AllowAlways';
import { Icon, type IconName } from './Icon';
import { Button } from './kit';

// Redesign PR 4: one request, drawn the same way wherever it waits (visual-language.md §3, approvals.md "One request, three sizes"). The
// tint, the icon chip and the order of the buttons never change; only the density does:
// - `row`: the Home queue (one line of who and where, the ask under it, the buttons at the end);
// - `card`: the side panel, above a terminal and above a chat's composer (the buttons under the ask);
// - `full`: the attention dialog (larger, the title is the ask).
// It is a frame only: every rule about when a button answers (the dwell, read first, one answer, the link) stays with the screen that
// draws it, which passes its buttons' `disabled` and `aria-disabled` in as it decides them.
export type ApprovalDensity = 'row' | 'card' | 'full';

// What kind of thing is asked, for the title and the icon chip. The only source is the request's own action word, which the agent wrote:
// a closed set of well-known tool names, matched exactly; anything else is the neutral "asks permission". The command, file or address is
// always on the line under the title, whole, and that line is what is decided on.
export type AskKind = 'run' | 'edit' | 'read' | 'fetch' | 'delete' | 'start' | 'skip' | 'other';
const KIND_WORDS = new Map<string, AskKind>([
  ...['run_command', 'bash', 'shell', 'execute', 'exec', 'run', 'command', 'terminal'].map((w) => [w, 'run'] as const),
  ...['edit', 'write', 'write_file', 'edit_file', 'multiedit', 'apply_patch', 'patch', 'notebookedit', 'str_replace'].map((w) => [w, 'edit'] as const),
  ...['read', 'read_file', 'view'].map((w) => [w, 'read'] as const),
  ...['fetch', 'webfetch', 'web_fetch'].map((w) => [w, 'fetch'] as const),
  ...['delete', 'delete_file', 'remove', 'rm'].map((w) => [w, 'delete'] as const),
]);
export function askKind(x: Pick<ApprovalModel, 'action' | 'start' | 'skip' | 'skipUnknown'>): AskKind {
  if (x.start) return 'start';
  if (x.skip || x.skipUnknown) return 'skip';
  return KIND_WORDS.get((x.action.trim().split(/\s+/)[0] ?? '').toLowerCase()) ?? 'other';
}
export const ASK_ICON: Record<AskKind, IconName> = { run: 'terminal', edit: 'pencil', read: 'files', fetch: 'globe', delete: 'trash', start: 'plus', skip: 'bolt', other: 'bell' };
/** The title of a request: the ask itself ("codex · teal wants to run a command"). `name` defaults to the request's agent name. */
export function askTitle(x: Pick<ApprovalModel, 'action' | 'start' | 'skip' | 'skipUnknown' | 'agentName'>, name = approvalName(x)): string {
  const kind = askKind(x);
  if (kind === 'other') return t('approval-title', { name });
  if (kind === 'skip') return t('skip-approval-title', { name });
  return t(`approval-ask-${kind}`, { name });
}

export function ApprovalRequest({ density, icon, avatar, title, where, titleEnd, children, actions, foot, stacked, alert, titleWrap, className = '', attrs }: {
  density: ApprovalDensity;
  icon: IconName;
  avatar?: ReactNode;
  title: ReactNode;
  // Where it runs and how long it waited, muted, after the title (tags in the full density).
  where?: ReactNode;
  // At the end of the title line (the side panel's "open the agent").
  titleEnd?: ReactNode;
  // The ask, whole (or, where the screen only offers Review, the short form).
  children?: ReactNode;
  actions?: ReactNode;
  // Under the buttons: the ADR-0101 note, a status line.
  foot?: ReactNode;
  // A narrow pane: the buttons go under the text, each as wide as it can be (never squeezed beside it).
  stacked?: boolean;
  alert?: boolean;
  // The attention dialog makes the title its own (Radix's Dialog.Title, which names the dialog).
  titleWrap?: (title: ReactElement) => ReactNode;
  className?: string;
  // Marks for a screen or a test to find the request by (`data-…`).
  attrs?: Record<`data-${string}`, string>;
}) {
  const id = useId();
  // Under `titleWrap` the wrapper gives the title its id (an `id` key here, even undefined, would replace it).
  const head = <p {...(titleWrap ? {} : { id })} className="kit-callout__title approval-req__title">{avatar}<span className="approval-req__text"><span className="approval-req__ask">{title}</span>{where ? <span className="approval-req__where">{where}</span> : null}</span>{titleEnd}</p>;
  const cls = ['kit-callout kit-tone-needs approval-req', `approval-req--${density}`, stacked && 'approval-req--stacked', className].filter(Boolean).join(' ');
  return (
    <section {...attrs} className={cls} data-density={stacked ? 'row' : density} aria-labelledby={titleWrap ? undefined : id} role={alert ? 'alert' : undefined}>
      <span aria-hidden className="kit-chip"><Icon name={icon} /></span>
      {titleWrap ? titleWrap(head) : head}
      {children ? <div className="kit-callout__body">{children}</div> : null}
      {actions ? <div className="kit-callout__actions">{actions}</div> : null}
      {foot ? <div className="approval-req__foot">{foot}</div> : null}
    </section>
  );
}

// The answers, always in this order: Deny (secondary), then "Allow always" where the request offers it beside a single answer (ADR-0101,
// secondary), then Allow, the primary and the last. A request that offers only "always" has it in the place of Allow. An Allow that lets an
// agent act without asking is the danger button (ADR-0102). `off` is what the screen decided for every answer button (disabled while the
// link is down, aria-disabled during the dwell or while an answer goes); `onAlways` is left out where "always" is never answered.
export interface AnswerProps { disabled?: boolean; 'aria-disabled'?: boolean; 'aria-describedby'?: string }
export function AnswerButtons({ approval: x, size = 'md', off = {}, danger, alwaysNote, denyLabel, allowLabel, allowExtra, onDeny, onAllow, onAlways }: {
  approval: Pick<ApprovalModel, 'always'>;
  size?: 'sm' | 'md';
  off?: AnswerProps;
  danger?: boolean;
  alwaysNote?: string;
  denyLabel?: string;
  allowLabel?: string;
  allowExtra?: ReactNode;
  onDeny: () => void;
  onAllow: () => void;
  onAlways?: () => void;
}) {
  const always = onAlways ? x.always : undefined;
  const alwaysOff = { ...off, 'aria-describedby': off['aria-describedby'] ?? alwaysNote };
  return <>
    <Button size={size} {...off} aria-label={denyLabel} onClick={onDeny}>{t('approval-deny')}</Button>
    {always && !always.only && <Button size={size} {...alwaysOff} onClick={onAlways}>{allowAlwaysWord()}</Button>}
    {always?.only
      ? <Button size={size} variant="primary" {...alwaysOff} onClick={onAlways}>{allowAlwaysWord()}</Button>
      : <Button size={size} variant={danger ? 'danger' : 'primary'} {...off} aria-label={allowLabel} onClick={onAllow}>{t('approval-allow')}{allowExtra}</Button>}
  </>;
}

// Below this width a pane draws its request in the row density with the buttons under the text (audit screenshot 35: a card squeezed
// beside its buttons to one word per line).
export const NARROW_PX = 360;
export function useNarrow(ref: RefObject<HTMLElement | null>, below = NARROW_PX): boolean {
  return useBelow(ref, below, 'width');
}

// Whether an element is smaller than `below` px along one axis. Unknown (nothing laid out yet, a test without layout) counts as large.
export function useBelow(ref: RefObject<HTMLElement | null>, below: number, axis: 'width' | 'height'): boolean {
  const [small, setSmall] = useState(false);
  // Measured before the first paint, so a small pane never shows the large layout for a frame.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const now = el.getBoundingClientRect()[axis];
    if (now > 0) setSmall(now < below);
    if (typeof ResizeObserver === 'undefined') return;
    // A callback without entries (a test's observer that only says "something changed") reads the element itself.
    const ro = new ResizeObserver((entries?: ResizeObserverEntry[]) => {
      const size = entries?.[entries.length - 1]?.contentRect[axis] ?? el.getBoundingClientRect()[axis];
      if (size > 0) setSmall(size < below);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref, below, axis]);
  return small;
}

// #1082: a chat pane shorter than this (the whole pane, header and composer included, which the card's own size never changes) draws the
// request pinned above its composer as one row, so the conversation keeps its room. The pane says so through `PinnedShort`.
export const SHORT_PANE_PX = 400;
export const PinnedShort = createContext(false);

// A card pinned above a chat's composer: whether it is drawn as the short row now, and the press that opens it to the whole card in place.
// The row offers Allow and Deny only for what `decidableInline` shows whole and only where they fit beside it (else Review, which opens
// the card); "always" is never answered from it (ADR-0101, as Home's row). The row and the card put the buttons in different places, so
// every change between them (the pane getting short or tall, Review opening the card) restarts the surface's dwell (`restart`, T44).
export function usePinnedRow(restart: () => void): { short: boolean; row: boolean; open: () => void } {
  const short = useContext(PinnedShort);
  const [opened, setOpened] = useState(false);
  const row = short && !opened;
  const drawn = useRef(row);
  useLayoutEffect(() => { if (drawn.current !== row) { drawn.current = row; restart(); } }, [row, restart]);
  return { short, row, open: useCallback(() => setOpened(true), []) };
}

// A card in a pane (`narrowable`): whether it is drawn narrow now. The buttons move under the text when the pane gets narrow (and back): a
// ready Allow must not move under a click, so the surface's dwell restarts (`restart`, the guard's).
export function useNarrowPane(ref: RefObject<HTMLElement | null>, narrowable: boolean | undefined, restart: () => void): boolean {
  const narrow = useNarrow(ref) && !!narrowable;
  const drawn = useRef(narrow);
  useLayoutEffect(() => { if (drawn.current !== narrow) { drawn.current = narrow; restart(); } }, [narrow, restart]);
  return narrow;
}
