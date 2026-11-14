import { useEffect, useRef, useState } from 'react';
import { inputBytes } from './frames';
import { type TerminalRenderer, XtermRenderer } from './renderer';
import type { TerminalSession, TerminalSource } from './source';
import { terminalSize } from './prefs';
import { KeyBar, useTouch } from './KeyBar';
import { keyBytes, withCtrl, type BarKey } from './keys';

// Binds a renderer to a source. Input goes straight to the source (it outranks output, ADR-0022); output is
// written as it arrives; the renderer is torn down with the component. On a touch screen the key bar sits under it (ADR-0067).
export function TerminalView({ source, label }: { source: TerminalSource; label: string }) {
  const host = useRef<HTMLDivElement>(null);
  const touch = useTouch();
  const renderer = useRef<TerminalRenderer | undefined>(undefined);
  const session = useRef<TerminalSession | undefined>(undefined);
  // Ctrl from the key bar applies to the next key typed, then lets go (like a phone's shift key).
  const [ctrl, setCtrl] = useState(false);
  const ctrlRef = useRef(false);
  const setCtrlBoth = (v: boolean) => { ctrlRef.current = v; setCtrl(v); };

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const r: TerminalRenderer = new XtermRenderer();
    renderer.current = r;
    r.mount(el);
    const s = source.connect((e) => {
      if (e.type === 'data') r.write(e.bytes);
      else if (e.type === 'snapshot') r.reset(e.bytes);
      else if (e.type === 'resize') r.resize(e.cols, e.rows);
    });
    session.current = s;
    r.onInput((d) => {
      if (ctrlRef.current) { ctrlRef.current = false; setCtrl(false); d = withCtrl(d); }
      s.sendInput(inputBytes(d));
    });
    const size = r.fit();
    if (size) s.resize(size.cols, size.rows);
    const observer = new ResizeObserver(() => {
      const z = r.fit();
      if (z) s.resize(z.cols, z.rows);
    });
    observer.observe(el);
    const onSize = () => { r.setFontSize(terminalSize()); const z = r.fit(); if (z) s.resize(z.cols, z.rows); };
    window.addEventListener('agora-terminal-size', onSize);
    return () => {
      window.removeEventListener('agora-terminal-size', onSize);
      observer.disconnect();
      s.close();
      r.dispose();
      renderer.current = undefined;
      session.current = undefined;
    };
  // The translated ARIA label can change without disposing the renderer or reconnecting the terminal. A change of touch screen (a
  // tablet's keyboard attached) moves the terminal into or out of the key bar's frame, so it is mounted again.
  }, [source, touch]);

  const view = <div ref={host} className="terminal" role="region" aria-label={label} dir="ltr" />;
  if (!touch) return view;
  const press = (k: BarKey) => {
    const bytes = keyBytes(k, { ctrl: ctrlRef.current, appCursor: renderer.current?.applicationCursor() });
    setCtrlBoth(false);
    session.current?.sendInput(inputBytes(bytes));
  };
  // Paste reads the clipboard only when the user presses it (the browser may ask first); where it cannot, the button is not shown and
  // the touch keyboard's own paste still works in the terminal.
  const canPaste = typeof navigator !== 'undefined' && !!navigator.clipboard?.readText;
  const paste = () => {
    setCtrlBoth(false);
    void navigator.clipboard.readText().then((text) => { if (text) renderer.current?.paste(text); }, () => undefined);
  };
  return (
    <div className="terminal-touch">
      {view}
      <KeyBar ctrl={ctrl} onKey={press} onCtrl={() => setCtrlBoth(!ctrlRef.current)} onPaste={canPaste ? paste : undefined} />
    </div>
  );
}
