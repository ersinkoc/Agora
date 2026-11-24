// Whether a "Needs you" list is on screen (the side panel on that subject, or a layout panel of it, ADR-0068). The attention toast is
// hidden meanwhile: it would be a third copy of what the list already shows (redesign PR 4). Memory only, counted per list drawn, so two
// lists open at once and one closing still leaves the other counted.
let shown = 0;
const listeners = new Set<() => void>();
const emit = () => { for (const l of listeners) l(); };

export const needsPanel = {
  /** A list was drawn; the returned function says it is gone. Each call counts once, however often its release runs. */
  show(): () => void {
    shown += 1;
    emit();
    let done = false;
    return () => { if (done) return; done = true; shown -= 1; emit(); };
  },
  isShown: (): boolean => shown > 0,
  subscribe(l: () => void): () => void { listeners.add(l); return () => { listeners.delete(l); }; },
};
