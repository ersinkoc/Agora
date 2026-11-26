import { useEffect, useMemo } from 'react';

// Which agents are on screen: the agent of the focused pane, and every agent whose own view (its terminal or chat) is drawn in a visible
// pane. The window (Console), the full-size view and the phone's agent screen each say what they show while they are drawn; nothing else
// is "on screen" (a tile on Home is not looking at the agent). Read by the sounds (never for the focused agent) and the finished mark
// (cleared once its agent is seen), together with whether the window itself is in front.

export interface Screen { focused?: string; visible: ReadonlySet<string> }
const EMPTY: Screen = Object.freeze({ visible: new Set<string>() });

const shown = new Map<symbol, Screen>();
let combined: Screen = EMPTY;
const listeners = new Set<() => void>();
const sameSet = (a: ReadonlySet<string>, b: ReadonlySet<string>) => a.size === b.size && [...a].every((x) => b.has(x));
function recompute() {
  let focused: string | undefined;
  const visible = new Set<string>();
  for (const s of shown.values()) { if (s.focused) focused = s.focused; for (const id of s.visible) visible.add(id); }
  if (focused) visible.add(focused);
  if (focused === combined.focused && sameSet(visible, combined.visible)) return;
  combined = visible.size === 0 && !focused ? EMPTY : { focused, visible };
  listeners.forEach((l) => l());
}

export const onScreen = {
  get: (): Screen => combined,
  // Says what one part of the window shows; called again when it changes, and `withdraw` when it is gone.
  publish(owner: symbol, s: Screen) { shown.set(owner, s); recompute(); },
  withdraw(owner: symbol) { if (shown.delete(owner)) recompute(); },
  subscribe(l: () => void) { listeners.add(l); return () => { listeners.delete(l); }; },
};

// Publishes what a component shows for as long as it is drawn.
export function useOnScreen(focused: string | undefined, visible: readonly string[]): void {
  const owner = useMemo(() => Symbol('screen'), []);
  const key = `${focused ?? ''}\n${visible.join('\n')}`;
  useEffect(() => { onScreen.publish(owner, { focused, visible: new Set(visible) }); }, [owner, key]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => onScreen.withdraw(owner), [owner]);
}
