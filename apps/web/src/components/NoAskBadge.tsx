import { useEffect, useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { createPortal } from 'react-dom';
import { t } from '../i18n';
import { plain } from './boardColumns';
import { Icon } from './Icon';

// ADR-0059: the agent was started with an option that lets it act without asking. Shown wherever the agent is (the tree, its tile, its tab,
// its pane header, the board, the give-task picker) for its whole life (threat model T15).
// It is one sign and no words: a lightning bolt in the "risk" colour (the fixed red of `--diff-removed`), so it can never be read as the
// quota chip (an amber warning sign with its percentage) or as "needs you" (amber). The whole meaning ("Runs without asking: Started so
// that it acts without asking you first") is its accessible name and its tooltip. The tooltip opens on hover, on keyboard focus and on a
// tap; it is drawn over the page (a portal), so a row that clips its content or the window's edge never cuts it, it stays open while the
// pointer is on it, and Esc closes it.
// `nested`: it sits inside a button or a tab, which must stay the one stop, so it is not focusable there; the tooltip then opens when that
// control has keyboard focus, a tap on the sign shows it without pressing the control, and the meaning is part of the control's name
// (a control with its own aria-label must say it there: AgentTile).
// `bare`: in a dense row or a tab, the bolt alone in red, with no chip around it (redesign PR 7); the same name, tooltip and behaviour.
// `cause`: the agent's own name for the mode or setting that makes it act so, when the window knows it (a session switch); agent text, so
// it is shown as one plain short line.
// `look="tag"`: the chat composer's sign beside Mode, a red-tinted square as tall as the menus next to it (app.css `.no-ask--tag`); the
// same sign, name, tooltip and behaviour, only drawn on the danger tint.
export function NoAskBadge({ agent, nested, bare, session, approved, cause, look = 'chip' }: { agent: { skipsPermissions?: boolean }; nested?: boolean; bare?: boolean; session?: boolean; approved?: boolean; cause?: string; look?: 'chip' | 'tag' }) {
  if (!agent.skipsPermissions) return null;
  // `session` (ADR-0090 Amendment A): it runs so after a switch and holds until it ends or is switched back. "With your approval" is said only when
  // this window saw that exact value approved (`approved`); after a restart, a resume or the agent's own switch it says only what is so.
  const why = t(session ? (approved ? 'agent-does-not-ask-session-explain' : 'agent-does-not-ask-session-neutral-explain') : 'agent-does-not-ask-explain');
  const shown = cause ? plain(cause, 64) : '';
  const full = `${t('agent-does-not-ask')}: ${why}${shown ? `. ${t('agent-does-not-ask-cause', { value: shown })}` : ''}`;
  return <Badge full={full} nested={!!nested} bare={!!bare} look={look} kind={session ? (approved ? 'session-approved' : 'session') : 'started'} />;
}

const CONTROL = 'button, a[href], [tabindex], [role="tab"], input, select, textarea';

function Badge({ full, nested, bare, kind, look }: { full: string; nested: boolean; bare: boolean; kind: string; look: 'chip' | 'tag' }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const pointer = useRef('');
  const show = () => { clearTimeout(timer.current); setAnchor(ref.current); };
  const hide = () => { clearTimeout(timer.current); setAnchor(null); };
  // The control that holds a nested sign (its row, tab or tile).
  const host = () => (nested ? ref.current?.parentElement?.closest<HTMLElement>(CONTROL) ?? null : null);
  // The pointer may move from the sign onto its tooltip and stay there (WCAG 1.4.13, hoverable): leaving waits a moment, and entering the
  // tooltip keeps it open. While keyboard focus still opens it (the sign's own, or its control's), the pointer leaving does not close it
  // (WCAG 1.4.13, persistent; Codex review of #1004).
  const focused = () => {
    const at = document.activeElement;
    if (at && at === ref.current) return true;
    const ctrl = host();
    return !!ctrl && at === ctrl && focusVisible(ctrl);
  };
  const leave = () => { clearTimeout(timer.current); if (focused()) return; timer.current = setTimeout(() => { if (!focused()) setAnchor(null); }, 300); };
  useEffect(() => () => clearTimeout(timer.current), []);
  // Inside a control: the tooltip follows that control's keyboard focus (not a mouse click, which would only flash it).
  useEffect(() => {
    const ctrl = host();
    if (!ctrl) return;
    // Only the control's own focus: a focus inside it that bubbles up (a dialog that holds the sign, say) is not the control's.
    const on = (e: FocusEvent) => { if (e.target === ctrl) setAnchor(focusVisible(ctrl) ? ref.current : null); };
    const off = (e: FocusEvent) => { if (e.target === ctrl) setAnchor(null); };
    ctrl.addEventListener('focusin', on);
    ctrl.addEventListener('focusout', off);
    return () => { ctrl.removeEventListener('focusin', on); ctrl.removeEventListener('focusout', off); };
  }, [nested]);
  // Esc closes it (WCAG 1.4.13); a scroll would leave it behind, so a scroll closes it too; so does a tap or click anywhere else.
  useEffect(() => {
    if (!anchor) return;
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') hide(); };
    const away = (e: PointerEvent) => { const at = e.target as Node | null; if (at && !ref.current?.contains(at) && !(at instanceof Element && at.closest('[data-no-ask-tip]'))) hide(); };
    document.addEventListener('keydown', key, true);
    document.addEventListener('pointerdown', away, true);
    window.addEventListener('scroll', hide, true);
    return () => { document.removeEventListener('keydown', key, true); document.removeEventListener('pointerdown', away, true); window.removeEventListener('scroll', hide, true); };
  }, [anchor]);
  // A tap on a sign inside a row, tab or tile shows what it means and does not press that control (a tap elsewhere on it still does). A
  // mouse click goes to the control as before: hovering already shows the tooltip.
  const tap = nested ? {
    onPointerDown: (e: ReactPointerEvent) => { pointer.current = e.pointerType; },
    onClick: (e: ReactMouseEvent) => { if (pointer.current === 'touch' || pointer.current === 'pen') { e.preventDefault(); e.stopPropagation(); show(); } pointer.current = ''; },
  } : {};
  const own = nested ? tap : { tabIndex: 0, onFocus: show, onBlur: hide };
  return (
    <span ref={ref} role="img" aria-label={full} data-no-ask={kind} data-look={look} className={look === 'tag' ? 'no-ask--tag' : `${bare ? 'inline-flex items-center' : 'ide-chip border-risk/60 px-1'} w-fit flex-none self-center justify-self-start text-risk`}
      onMouseEnter={show} onMouseLeave={leave} {...own}>
      <Icon name="bolt" className="size-3.5" />
      {anchor && <Bubble anchor={anchor} text={full} onEnter={show} onLeave={leave} />}
    </span>
  );
}

function focusVisible(el: Element): boolean {
  try { return el.matches(':focus-visible'); } catch { return true; } // a browser that does not know :focus-visible: show it on any focus
}

// The tooltip itself (app.css `.tip-bubble`), placed under the sign (above it at the bottom of the window) and kept
// inside the window on both sides. Hidden from screen readers: the sign's name already says the same.
function Bubble({ anchor, text, onEnter, onLeave }: { anchor: HTMLElement; text: string; onEnter: () => void; onLeave: () => void }) {
  const ref = useRef<HTMLSpanElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number }>();
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const a = anchor.getBoundingClientRect();
    const b = el.getBoundingClientRect();
    const margin = 8;
    const width = document.documentElement.clientWidth || window.innerWidth;
    const rtl = getComputedStyle(anchor).direction === 'rtl';
    const left = Math.max(margin, Math.min(rtl ? a.right - b.width : a.left, width - b.width - margin));
    const below = a.bottom + 4;
    const top = below + b.height > window.innerHeight - margin ? Math.max(margin, a.top - 4 - b.height) : below;
    setPos({ left, top });
  }, [anchor, text]);
  return createPortal(
    // It is a React child of the sign: a press on it must not reach the row or tab that holds the sign.
    <span ref={ref} aria-hidden data-no-ask-tip className="tip-bubble" onMouseEnter={onEnter} onMouseLeave={onLeave} onClick={(e) => e.stopPropagation()} onPointerDown={(e) => e.stopPropagation()} style={pos ? { left: pos.left, top: pos.top } : { left: 0, top: 0, visibility: 'hidden' }}>{text}</span>,
    document.body,
  );
}
