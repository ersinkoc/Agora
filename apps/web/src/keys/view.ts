// What the window shows of commands and keys: a command's name, and the key it has now as the palette, the title bar and the rail show it.
import { useSyncExternalStore } from 'react';
import { t } from '../i18n';
import { type CommandDef, MESSAGE_ARGS } from './commands';
import { type Bound, keysOf } from './file';
import { keyStore } from './store';
import { IS_MAC, ariaKeys, bindingLabel, primary } from './strokes';

/** A command's name in the window's language. */
export function commandLabel(def: CommandDef): string {
  if (!def.args) return t(def.label);
  const args = Object.fromEntries(Object.entries(def.args).map(([k, v]) => [k, MESSAGE_ARGS.has(k) && typeof v === 'string' ? t(v) : v]));
  return t(def.label, args);
}

/** The keys in force, following a change made in Settings > Keyboard or in another tab. */
export const useKeymap = (): readonly Bound[] => useSyncExternalStore(keyStore.subscribe, () => keyStore.state().map, () => keyStore.state().map);

/** The key a command has now, as the window writes it (`Ctrl+K`; `sep` between a modifier and the key), or undefined when it has none. */
export function shownKey(map: readonly Bound[], command: string, sep?: string, mac = IS_MAC): string | undefined {
  const b = primary(keysOf(map, command), mac);
  return b ? bindingLabel(b, mac, sep) : undefined;
}

/** The same key for `aria-keyshortcuts`; a chord has no ARIA spelling (a space there means "or"), so it gets none. */
export function ariaKey(map: readonly Bound[], command: string, mac = IS_MAC): string | undefined {
  const b = primary(keysOf(map, command), mac);
  return b && b.length === 1 ? ariaKeys(b) : undefined;
}
