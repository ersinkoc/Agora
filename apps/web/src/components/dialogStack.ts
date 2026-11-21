// Escape belongs to the topmost open dialog only. A dialog opened over another one (the "Give to an agent" dialog over the board) starts
// listening for Escape a render or two after it appears, and until then the dialog under it still does: an Escape pressed in that moment
// closed the board instead of the dialog on top. A lower dialog passes its `onEscapeKeyDown` through this to ignore such a key.

/** Whether `el` (a dialog's content) has another open dialog above it: a later open dialog in the document that is not inside it. */
export function belowAnotherDialog(el: Element | null, doc: Document = document): boolean {
  if (!el) return false;
  const open = [...doc.querySelectorAll('[role="dialog"][data-state="open"]')];
  const top = open[open.length - 1];
  return !!top && top !== el && !el.contains(top);
}
