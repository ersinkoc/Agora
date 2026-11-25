// The idle lock (ADR-0067): when the owner turned it on for this browser (This device), the window locks itself after that many
// minutes without a key press, a click, a touch or a scroll. Off by default. The choice is kept in this browser only; locking
// never touches agents, it only hides the window until the screen password is typed again.

export const IDLE_CHOICES = [0, 5, 10, 30, 60] as const;
export type IdleMinutes = (typeof IDLE_CHOICES)[number];

const KEY = 'agora.idleLock';

type Store = Pick<Storage, 'getItem' | 'setItem'>;

function local(): Store | undefined {
  try { return window.localStorage; } catch { return undefined; }
}

export function idleMinutes(store: Store | undefined = local()): IdleMinutes {
  let raw: string | null = null;
  try { raw = store?.getItem(KEY) ?? null; } catch { /* unavailable */ }
  const n = Number(raw);
  return (IDLE_CHOICES as readonly number[]).includes(n) ? (n as IdleMinutes) : 0;
}

export function setIdleMinutes(minutes: IdleMinutes, store: Store | undefined = local()): void {
  try { store?.setItem(KEY, String(minutes)); } catch { /* private mode: lasts until reload */ }
}

// What the person does that counts as "still here".
export const ACTIVITY = ['keydown', 'pointerdown', 'pointermove', 'wheel', 'touchstart', 'scroll'] as const;

// Calls `onIdle` once after `minutes` without activity on `target`; any activity starts the count again. Returns the stop function.
// `minutes` 0 watches nothing.
export function watchIdle(minutes: number, onIdle: () => void, target: Pick<EventTarget, 'addEventListener' | 'removeEventListener'>): () => void {
  if (minutes <= 0) return () => {};
  const ms = minutes * 60_000;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let last = performance.now();
  // Record every event without restarting the timer on every pointer move.
  const active = () => { last = performance.now(); };
  const fire = () => {
    const remaining = ms - (performance.now() - last);
    if (remaining > 0) {
      timer = setTimeout(fire, Math.ceil(remaining));
      return;
    }
    timer = undefined;
    stop();
    onIdle();
  };
  const stop = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    for (const e of ACTIVITY) target.removeEventListener(e, active, true);
  };
  for (const e of ACTIVITY) target.addEventListener(e, active, { capture: true, passive: true });
  timer = setTimeout(fire, ms);
  return stop;
}
