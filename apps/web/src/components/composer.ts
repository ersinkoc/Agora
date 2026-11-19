// The composer's Enter: sends, except Shift+Enter (a new line) and the Enter that only confirms an input method's candidate. Safari fires
// that keydown after `compositionend` with `isComposing` already false, so a key code of 229 and the moment the composition ended count
// too. Pure, so it is tested without a page.
export const IME_GRACE_MS = 80;

export interface EnterKey { key: string; shiftKey: boolean; isComposing?: boolean; keyCode?: number }

export function sendsOnEnter(e: EnterKey, sinceCompositionEndMs: number): boolean {
  return e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229 && sinceCompositionEndMs > IME_GRACE_MS;
}
