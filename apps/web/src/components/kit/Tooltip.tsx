import { cloneElement, useEffect, useId, useRef, useState, type ReactElement } from 'react';

// A short name or explanation for a control, shown on hover and on keyboard focus, under it (an overlay: the raised surface and the one
// shadow). It is the control's description (aria-describedby), so the control keeps its own name. WCAG 1.4.13: the pointer can move onto
// it (leaving waits a moment), it stays while focus or the pointer is there, and Esc closes it.
export function Tooltip({ text, children }: { text: string; children: ReactElement<{ 'aria-describedby'?: string }> }) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const show = () => { clearTimeout(timer.current); setOpen(true); };
  const leave = () => { clearTimeout(timer.current); timer.current = setTimeout(() => setOpen(false), 200); };
  const hide = () => { clearTimeout(timer.current); setOpen(false); };
  useEffect(() => () => clearTimeout(timer.current), []);
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') hide(); };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, [open]);
  return (
    <span className="kit-tip-host" onMouseEnter={show} onMouseLeave={leave} onFocus={show} onBlur={hide}>
      {cloneElement(children, { 'aria-describedby': id })}
      <span id={id} role="tooltip" className="kit-tip" hidden={!open}>{text}</span>
    </span>
  );
}
