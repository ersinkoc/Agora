// The "Needs you" attention layer's rules (docs/ux/main-window.md, "Needs you, loudly"), pure: no DOM except where a function takes the
// element or the clock as an argument. The window may open a modal for something that needs the person, and a system notification when
// the window is in the background, but only by these rules: never over what the person is typing, never with text the agent wrote in a
// notification, and never an answer that a stray click or key can give.
import { decidableInline } from './fleet';
import type { AttentionItem } from './fleet';
import { visible } from './view';

// How the window gets the person's attention. `modal`: a card over the window for a request that can be answered or reviewed; `banner`:
// a small non-modal strip; `off`: neither (the Needs you list and the counts stay everywhere, always). Kept per device.
export type AttentionMode = 'modal' | 'banner' | 'off';
export const ATTENTION_MODES: readonly AttentionMode[] = ['modal', 'banner', 'off'];
// What a system notification says when the window is in the background. `generic`: no name at all; `named`: the agent and machine names.
// A notification never carries what is asked (a request's text, a command, a path), whatever the choice.
export type NotifyMode = 'off' | 'generic' | 'named';
export const NOTIFY_MODES: readonly NotifyMode[] = ['off', 'generic', 'named'];

export const parseMode = (raw: unknown): AttentionMode => (raw === 'banner' || raw === 'off' ? raw : 'modal');
export const parseNotify = (raw: unknown): NotifyMode => (raw === 'generic' || raw === 'named' ? raw : 'off');

// Both choices belong to this browser only, like the idle lock and the theme: storage that is blocked or full means they hold for this visit.
const MODE_KEY = 'agora.attention.mode';
const NOTIFY_KEY = 'agora.attention.notify';
type Kv = Pick<Storage, 'getItem' | 'setItem'>;
const local = (): Kv | undefined => { try { return typeof window !== 'undefined' ? window.localStorage : undefined; } catch { return undefined; } };
const read = (key: string, kv: Kv | undefined): string | undefined => { try { return kv?.getItem(key) ?? undefined; } catch { return undefined; } };
const write = (key: string, value: string, kv: Kv | undefined) => { try { kv?.setItem(key, value); } catch { /* holds for this visit */ } };

let mode: AttentionMode | undefined;
let notify: NotifyMode | undefined;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
export const attentionPrefs = {
  mode: (kv: Kv | undefined = local()): AttentionMode => (mode ??= parseMode(read(MODE_KEY, kv))),
  notify: (kv: Kv | undefined = local()): NotifyMode => (notify ??= parseNotify(read(NOTIFY_KEY, kv))),
  setMode(m: AttentionMode, kv: Kv | undefined = local()) { mode = m; write(MODE_KEY, m, kv); emit(); },
  setNotify(n: NotifyMode, kv: Kv | undefined = local()) { notify = n; write(NOTIFY_KEY, n, kv); emit(); },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
  // For tests: forget what this visit held.
  reset() { mode = undefined; notify = undefined; emit(); },
};

// What the modal can do for an item. `answer`: a plain yes or no that is shown whole (the fleet console's `decidableInline`): Allow and
// Deny. `review`: anything else that is a request (a start, settings, a long, multi-line or structured text, a hidden character, a machine
// asking to join): the modal says what and offers Review, which goes to the full card. An agent that waits in its terminal with no
// request open has no modal (nothing to answer or review here): the banner and the list carry it.
export type ModalKind = 'answer' | 'review';
export function modalKind(it: AttentionItem): ModalKind | undefined {
  if (it.kind === 'pair') return 'review';
  if (it.kind === 'approval') return decidableInline(it.approval) ? 'answer' : 'review';
  return undefined;
}

// A modal's buttons stay dead for this long after it appeared or after what it shows changed (the dock's rule, docs/ux/phone.md): a click
// meant for what was there before must not land on what replaced it.
export const ATTENTION_DWELL_MS = 600;
// After the last key typed into a field or a terminal, the person still counts as typing for this long.
export const TYPING_QUIET_MS = 1500;
// One notification per burst: a second one waits at least this long after the first.
export const NOTIFY_WINDOW_MS = 15_000;

// A key for what the modal displays: the request and every word shown of it. A change of it restarts the dwell.
export function shownKey(it: AttentionItem): string {
  if (it.kind === 'approval') return `${it.key}\u0001${it.approval.action}\u0001${it.approval.detail}\u0001${it.approval.start ? 's' : ''}`;
  if (it.kind === 'pair') return `${it.key}\u0001${it.card.name}\u0001${it.card.number}`;
  return it.key;
}

// Whether an element takes typed text: a field, a select, anything editable, a terminal (xterm keeps a hidden textarea).
export function isTypingTarget(el: Element | null | undefined): boolean {
  // An event's target may be the document or the window, which are not elements.
  if (!el || typeof el.getAttribute !== 'function') return false;
  if (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) return true;
  if ((el as HTMLElement).isContentEditable) return true;
  const role = el.getAttribute('role');
  return role === 'textbox' || role === 'searchbox' || role === 'combobox' || !!el.closest('.xterm');
}

export interface Presence { focused: boolean; typing: boolean; lastKeyMs: number; nowMs: number }
// A modal may open only when the window is focused and the person is not typing: not in a field or a terminal, and not within
// TYPING_QUIET_MS of a key in one. Otherwise it waits (the banner and the list show it) and nothing takes a keystroke.
export const mayInterrupt = (p: Presence): boolean => p.focused && !p.typing && p.nowMs - p.lastKeyMs >= TYPING_QUIET_MS;

// When a notification may go out: 0 now, else after this many milliseconds (one trailing notification at the end of the window).
export const notifyDelay = (lastMs: number | undefined, nowMs: number): number => (lastMs === undefined ? 0 : Math.max(0, lastMs + NOTIFY_WINDOW_MS - nowMs));

// What a notification says: message ids and parameters only, never anything the agent wrote. `named` adds the agent's and machine's display
// names (cut and made visible); it never adds a request's action, detail, folder, prompt, command or path.
export interface NotifyText { title: { id: string }; body: { id: string; params: Record<string, string | number> }; tag: string }
const NAME_MAX = 40;
const name = (s: string | undefined): string => { const v = visible((s ?? '').replace(/\s+/g, ' ').trim()); return v.length > NAME_MAX ? `${v.slice(0, NAME_MAX)}…` : v; };
export function notifyText(items: readonly AttentionItem[], how: NotifyMode): NotifyText {
  const first = items[0];
  const agent = first && first.kind !== 'pair' ? first.agent?.name || (first.kind === 'approval' ? first.approval.agentName : '') : '';
  const machine = first && first.kind !== 'pair' ? first.agent?.machine || (first.kind === 'approval' ? first.approval.machineName : '') : '';
  const tag = 'agora-needs-you'; // one at a time: a newer one replaces the older
  if (how === 'named' && items.length === 1 && agent) {
    return { title: { id: 'notify-title' }, body: { id: machine ? 'notify-body-named-on' : 'notify-body-named', params: { name: name(agent), machine: name(machine) } }, tag };
  }
  return { title: { id: 'notify-title' }, body: { id: 'notify-body', params: { count: items.length } }, tag };
}
