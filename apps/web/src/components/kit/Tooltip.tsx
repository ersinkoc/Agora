import { cloneElement, useEffect, useId, useRef, useState, type ReactElement } from 'react';

// A short name or explanation for a control, shown on hover and on keyboard focus, under it (an overlay: the raised surface and the one
// shadow). It is added to the control's description (aria-describedby, after any it already has), so the control keeps its own name.
// WCAG 1.4.13: hover and focus are tracked apart, so it stays while either holds (the pointer leaving a focused control, or focus leaving
// a hovered one, does not close it); the pointer can move onto it (leaving waits a moment); Esc closes it until the next hover or focus.
export function Tooltip({ text, children }: { text: string; children: ReactElement<{ 'aria-describedby'?: string }> }) {
  const id = useId();
  const [hover, setHover] = useState(false);
  const [focus, setFocus] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const open = (hover || focus) && !dismissed;
  const enter = () => { clearTimeout(timer.current); setHover(true); setDismissed(false); };
  const leave = () => { clearTimeout(timer.current); timer.current = setTimeout(() => setHover(false), 200); };
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') setDismissed(true); };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, [open]);
  const described = [children.props['aria-describedby'], id].filter(Boolean).join(' ');
  return (
    <span className="kit-tip-host" onMouseEnter={enter} onMouseLeave={leave} onFocus={() => { setFocus(true); setDismissed(false); }} onBlur={() => setFocus(false)}>
      {cloneElement(children, { 'aria-describedby': described })}
      <span id={id} role="tooltip" className="kit-tip" hidden={!open}>{text}</span>
    </span>
  );
}
