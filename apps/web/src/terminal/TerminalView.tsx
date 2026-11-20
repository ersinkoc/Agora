import { useEffect, useRef, useState } from 'react';
import { binaryBytes, inputBytes, snapshotAfterSize } from './frames';
import { TerminalFloat } from './TerminalFloat';
import { FindBar } from './FindBar';
import { OfferGate } from './clipboard';
import { type TerminalRenderer, XtermRenderer } from './renderer';
import type { TerminalSession, TerminalSource } from './source';
import { terminalPrefs, terminalSize } from './prefs';
import { KeyBar, useTouch } from './KeyBar';
import { keyBytes, withCtrl, type BarKey } from './keys';
import { TerminalSizing } from './sizing';

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
  // Scrolled up: new output is below the view, so a button takes you back. A program offered clipboard text (it needs your say-so).
  const [atBottom, setAtBottom] = useState(true);
  const [offer, setOffer] = useState<string | undefined>(undefined);
  const [finding, setFinding] = useState(false);
  const bell = useRef<number | undefined>(undefined);
  const gate = useRef(new OfferGate());
  const pendingOffer = useRef(false);
  const [link, setLink] = useState<string | undefined>(undefined);
  const offerRef = (v: string | undefined) => { pendingOffer.current = v !== undefined; setOffer(v); };

  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const r: TerminalRenderer = new XtermRenderer();
    renderer.current = r;
    r.setPrefs(terminalPrefs());
    r.mount(el);
    r.onScrollState(setAtBottom);
    r.onClipboardOffer((text) => { if (gate.current.accept(Date.now(), pendingOffer.current)) offerRef(text); });
    r.onLinkHover(setLink);
    r.onFindRequest(() => setFinding(true));
    // The bell is a brief ring round the terminal (and nothing else: no sound, no title change).
    r.onBell(() => {
      el.dataset.bell = '';
      window.clearTimeout(bell.current);
      bell.current = window.setTimeout(() => delete el.dataset.bell, 300);
    });
    // The window reports what fits it; the terminal is shown at the size the node reports, which is the smallest of the windows
    // watching it (sizing.ts). The element says which, for a bigger window's edge and for tests.
    let s: TerminalSession | undefined;
    const sizing = new TerminalSizing({
      measure: () => r.measure(),
      show: (z) => {
        r.resize(z.cols, z.rows);
        el.dataset.cols = String(z.cols);
        el.dataset.rows = String(z.rows);
        el.toggleAttribute('data-smaller', sizing.smaller());
      },
      report: (z) => {
        s?.resize(z.cols, z.rows);
        el.toggleAttribute('data-smaller', sizing.smaller());
      },
    });
    const ordered = snapshotAfterSize((e) => {
      if (e.type === 'data') r.write(e.bytes);
      else if (e.type === 'snapshot') r.reset(e.bytes);
      else if (e.type === 'resize') sizing.resized({ cols: e.cols, rows: e.rows });
    });
    s = source.connect(ordered.push);
    const live = s;
    session.current = live;
    r.onInput((d, binary) => {
      if (binary) { live.sendInput(binaryBytes(d)); return; }
      if (ctrlRef.current) { ctrlRef.current = false; setCtrl(false); d = withCtrl(d); }
      live.sendInput(inputBytes(d));
    });
    sizing.viewportChanged();
    const observer = new ResizeObserver(() => sizing.viewportChanged());
    observer.observe(el);
    const onSize = () => { r.setFontSize(terminalSize()); sizing.viewportChanged(); };
    window.addEventListener('agora-terminal-size', onSize);
    // A theme can change the terminal's font: the renderer has repainted by now (it listens first), so measure again.
    window.addEventListener('agora-theme', onSize);
    const onPrefs = () => { r.setPrefs(terminalPrefs()); requestAnimationFrame(() => sizing.viewportChanged()); };
    window.addEventListener('agora-terminal-prefs', onPrefs);
    return () => {
      window.removeEventListener('agora-terminal-prefs', onPrefs);
      window.clearTimeout(bell.current);
      window.removeEventListener('agora-terminal-size', onSize);
      window.removeEventListener('agora-theme', onSize);
      observer.disconnect();
      live.close();
      ordered.dispose();
      r.dispose();
      renderer.current = undefined;
      session.current = undefined;
      setAtBottom(true); offerRef(undefined); setFinding(false); setLink(undefined);
    };
  // The translated ARIA label can change without disposing the renderer or reconnecting the terminal. A change of touch screen (a
  // tablet's keyboard attached) moves the terminal into or out of the key bar's frame, so it is mounted again.
  }, [source, touch]);

  const closeFind = () => { setFinding(false); renderer.current?.clearFind(); renderer.current?.focus(); };
  const view = (
    <div className="terminal-frame">
      <div ref={host} className="terminal" role="region" aria-label={label} dir="ltr" />
      {finding && <FindBar find={(q, back) => renderer.current?.find(q, back) ?? false} onClose={closeFind} />}
      <TerminalFloat atBottom={atBottom} onJump={() => renderer.current?.scrollToBottom()} link={link} offer={offer}
        onCopy={() => { if (offer !== undefined) void navigator.clipboard?.writeText(offer).catch(() => undefined); offerRef(undefined); renderer.current?.focus(); }}
        onIgnore={() => { offerRef(undefined); renderer.current?.focus(); }} />
    </div>
  );
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
