// The command registry and the one key listener (docs/ux/keybindings.md). The parts of the window that can run a command register its
// handler while they are on screen (the app: the palette, Home, Settings; the window: the agent list, tabs, the layout; the side panel:
// its rail); a key press is looked up in the keys in force and runs the first command bound to it that can act now. The command palette
// lists the same handlers, so every command is in it with its key.
import { useEffect, useRef, useSyncExternalStore } from 'react';
import { type Bound } from './file';
import { fixedReason } from './commands';
import { type KeyEventLike, type Stroke, reachesPastTerminal, sameStroke, startsWith, strokeOf, typesText } from './strokes';

export interface Handler {
  run: () => void;
  /** It can act now (a tab that exists, a hub that is connected); when false the key goes on to the next command or the page. */
  available?: boolean;
  /** Its name now, when it says more than the command's name (the side panel's place 3 is "Team chat" here). */
  label?: string;
}

// The handler registered last wins, so a part drawn over another takes its commands while it is there.
const handlers = new Map<string, Handler[]>();
const listeners = new Set<() => void>();
let version = 0;
const changed = () => { version++; listeners.forEach((l) => l()); };

/** Registers a command's handler; the function returned takes it away again. */
export function registerCommand(id: string, h: Handler): () => void {
  const list = handlers.get(id) ?? [];
  handlers.set(id, [...list, h]);
  changed();
  return () => {
    const now = (handlers.get(id) ?? []).filter((x) => x !== h);
    if (now.length) handlers.set(id, now); else handlers.delete(id);
    changed();
  };
}
export const handlerOf = (id: string): Handler | undefined => handlers.get(id)?.at(-1);
export const canRun = (id: string): boolean => { const h = handlerOf(id); return !!h && h.available !== false; };
/** Runs a command now; false when it has no handler on screen or cannot act. */
export function runCommand(id: string): boolean {
  const h = handlerOf(id);
  if (!h || h.available === false) return false;
  h.run();
  return true;
}
export const registeredCommands = (): string[] => [...handlers.keys()];
export const subscribeCommands = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
/** Changes whenever a handler comes or goes. */
export const useCommandsVersion = () => useSyncExternalStore(subscribeCommands, () => version, () => version);

/**
 * Registers a table of handlers for as long as the component is on screen. The table is read again on every render, so a handler
 * always runs with what the component has now; a `false` entry is a command it does not offer at the moment.
 */
export function useCommands(table: Record<string, Handler | false | undefined>): void {
  const ref = useRef(table);
  ref.current = table;
  const ids = Object.keys(table).filter((k) => table[k]).join('\n');
  useEffect(() => {
    if (!ids) return undefined;
    const offs = ids.split('\n').map((id) => registerCommand(id, {
      run: () => { const h = ref.current[id]; if (h) h.run(); },
      get available() { const h = ref.current[id]; return !!h && h.available !== false; },
      get label() { const h = ref.current[id]; return h ? h.label : undefined; },
    }));
    return () => offs.forEach((off) => off());
  }, [ids]);
}

// ---- A key press -----------------------------------------------------------------------------------------------------------------------

/** How long a chord waits for its second key. */
export const CHORD_MS = 1500;

/** What has the keyboard: a terminal (its program gets the keys), a text field, or anything else. */
export type Where = 'terminal' | 'text' | 'other';

export function whereOf(target: EventTarget | null | undefined): Where {
  const el = target as Element | null | undefined;
  if (!el || typeof el.closest !== 'function') return 'other';
  if (el.closest('.xterm')) return 'terminal';
  if (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || (el as HTMLElement).isContentEditable) return 'text';
  const role = el.getAttribute('role');
  return role === 'textbox' || role === 'searchbox' || role === 'combobox' ? 'text' : 'other';
}

export type Outcome = { kind: 'none' } | { kind: 'pending'; first: Stroke } | { kind: 'ran'; command: string };

export interface DispatchEnv {
  map: () => readonly Bound[];
  mac: boolean;
  now?: () => number;
  available?: (id: string) => boolean;
  run?: (id: string) => boolean;
}

/**
 * The key matcher, pure apart from the chord it waits on. Rules, in order:
 * 1. A second key while a chord waits (within CHORD_MS) completes it, or the chord is dropped and the key is taken as a new one.
 * 2. A fixed key (commands.ts) is never a command here: the safe-mode key is heard by the window before this, and Esc, Tab, the arrows,
 *    a terminal's copy, find and paste and a field's editing keys stay what they are.
 * 3. A terminal keeps its keys: only a key it can never receive (any ⌘ key, Ctrl+Shift+letter) acts from it, and no chord starts there.
 * 4. A text field keeps what types text (a letter, Shift+letter, Option+letter on a Mac); Ctrl, Alt or ⌘ keys still act.
 * 5. A chord bound to this key waits for its second key; otherwise the first command bound to it that can act now runs.
 */
export function createDispatcher(env: DispatchEnv) {
  const now = env.now ?? (() => Date.now());
  const available = env.available ?? canRun;
  const run = env.run ?? runCommand;
  let pending: { first: Stroke; at: number } | undefined;
  return {
    pending: () => pending?.first,
    cancel: () => { pending = undefined; },
    handle(e: KeyEventLike & { defaultPrevented?: boolean }, where: Where): Outcome {
      if (e.defaultPrevented) return { kind: 'none' };
      const s = strokeOf(e);
      if (!s) return { kind: 'none' }; // a modifier on its own: a chord keeps waiting
      const map = env.map();
      const waiting = pending && now() - pending.at <= CHORD_MS ? pending.first : undefined;
      pending = undefined;
      if (waiting) {
        const seq = [waiting, s];
        const done = map.find((b) => b.binding.length === 2 && startsWith(b.binding, seq) && available(b.command));
        if (done && run(done.command)) return { kind: 'ran', command: done.command };
      }
      if (fixedReason(s)) return { kind: 'none' };
      if (where === 'terminal' && !reachesPastTerminal(s)) return { kind: 'none' };
      if (where === 'text' && typesText(s, env.mac)) return { kind: 'none' };
      const first = map.filter((b) => sameStroke(b.binding[0] as Stroke, s) && available(b.command));
      if (where !== 'terminal' && first.some((b) => b.binding.length > 1)) {
        pending = { first: s, at: now() };
        return { kind: 'pending', first: s };
      }
      for (const b of first) if (b.binding.length === 1 && run(b.command)) return { kind: 'ran', command: b.command };
      return { kind: 'none' };
    },
  };
}

/**
 * Listens on the window, after the page's own handlers (the bubble phase): a dialog that keeps a key to itself (a letter typed in it)
 * keeps it from the window, as before this rung, and a terminal's keys never get here (the terminal takes them first).
 */
export function listenForKeys(env: DispatchEnv, w: Pick<Window, 'addEventListener' | 'removeEventListener'> = window): () => void {
  const d = createDispatcher(env);
  const on = (e: KeyboardEvent) => {
    const r = d.handle(e, whereOf(e.target));
    if (r.kind !== 'none') e.preventDefault();
  };
  w.addEventListener('keydown', on);
  return () => w.removeEventListener('keydown', on);
}
