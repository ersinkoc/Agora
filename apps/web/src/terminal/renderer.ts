// The renderer behind an interface (ADR-0017): xterm.js today, swappable for a WebAssembly emulator once the
// choice is measured against the budgets in ADR-0022. Nothing outside this folder imports xterm.
import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { terminalTheme } from './palette';
import { terminalFontFamily } from './fonts';
import { type TerminalPrefs, terminalPrefs, terminalSize } from './prefs';
import { clipboardOffer, sanitizePaste } from './clipboard';
import { enhancedKey, KeyboardProtocols, registerKeyboardProtocols, shortcut } from './keys';
import { openLink } from './links';
import { searchDecorations, toHex } from './searchColours';
import { followMouseEncoding } from './mouse';
import { TouchScroll, Wheel, type WheelAction } from './wheel';

export interface TerminalRenderer {
  mount(el: HTMLElement): void;
  write(bytes: Uint8Array): void;
  /** Repaints from an ANSI snapshot, in one paint (after a reconnect or when the client fell behind), after the history the node sent
   * before it (kind 4, ADR-0100), when there is one. */
  reset(snapshot: Uint8Array, history?: Uint8Array): void;
  /** Shows the terminal at the size the node reports (which may be smaller than the element: it is shared, see sizing.ts). */
  resize(cols: number, rows: number): void;
  /** Size that fits the mounted element, to report to the node. Measures only: the terminal keeps its size. */
  measure(): { cols: number; rows: number } | undefined;
  /** What the user typed or the mouse reported. `binary`: each character is one byte as it is (the older mouse reports), not UTF-8 text. */
  onInput(cb: (data: string, binary?: boolean) => void): void;
  /** Pastes text as the program expects it (bracketed when it asked for that); it arrives through `onInput`. */
  paste(text: string): void;
  /** Whether the program asked for application cursor keys (DECCKM), which changes the bytes an arrow sends. */
  applicationCursor(): boolean;
  focus(): void;
  setFontSize(px: number): void;
  /** Line height, history size, colour lifting and copy-on-select (prefs.ts). */
  setPrefs(p: TerminalPrefs): void;
  /** Tells whether the view sits at the live bottom (false: the user scrolled up and new output is below). */
  onScrollState(cb: (atBottom: boolean) => void): void;
  scrollToBottom(): void;
  /** A program offered text for the clipboard (OSC 52); nothing is copied until the user agrees. */
  onClipboardOffer(cb: (text: string) => void): void;
  /** Copies the selection; false when nothing is selected. */
  copySelection(): boolean;
  /** The pointer is over a link (its address) or left it (undefined). */
  onLinkHover(cb: (url: string | undefined) => void): void;
  /** The program rang the bell. */
  onBell(cb: () => void): void;
  /** The find shortcut was pressed in the terminal. */
  onFindRequest(cb: () => void): void;
  /** Finds the next (or previous) match of the text in the history and screen; false when there is none. */
  find(text: string, backwards: boolean): boolean;
  clearFind(): void;
  dispose(): void;
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

// The 16 colours programs draw with must stay readable on the pane's background: one palette per scheme, unless the theme sets its
// own (terminal/palette.ts).
function theme() {
  const root = document.documentElement.dataset.theme;
  const light = root ? root === 'light' : !!window.matchMedia?.('(prefers-color-scheme: light)').matches;
  return terminalTheme(token, light);
}
// A token as the browser computed it (a theme may write any CSS colour, and the page's variables are what is drawn): a hidden probe takes
// the colour and its computed `rgb(…)` is read back as `#rrggbb`.
let probe: HTMLSpanElement | undefined;
// `under` is the token the variable falls back to when the theme does not set it (the terminal's background is the pane's).
function computedHex(name: string, fallback: string, under?: string): string {
  if (!probe?.isConnected) {
    probe = document.createElement('span');
    probe.hidden = true;
    document.body.append(probe);
  }
  probe.style.color = under ? `var(${name}, var(${under}, ${fallback}))` : `var(${name}, ${fallback})`;
  return toHex(getComputedStyle(probe).color) ?? toHex(token(name)) ?? (under ? toHex(token(under)) : undefined) ?? fallback;
}
// Search matches in the theme's accent mixed toward the terminal's background (searchColours.ts), read again for every search and on
// every theme change, so they follow the theme.
function searchColors() {
  return searchDecorations(computedHex('--accent', '#8b93ff'), computedHex('--terminal-background', '#09090b', '--surface-0'), computedHex('--terminal-foreground', '#f4f4f5', '--text'));
}
const MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

export class XtermRenderer implements TerminalRenderer {
  private term: Terminal;
  private fitAddon = new FitAddon();
  private searchAddon = new SearchAddon();
  private wheel = new Wheel();
  private touch = new TouchScroll();
  private keys = new KeyboardProtocols();
  // Ends the listeners put on the element this renderer was mounted in: the element outlives it (a new source mounts a new renderer
  // there), and an old wheel listener would take every event before the new terminal saw it.
  private listeners = new AbortController();
  private copyOnSelect = false;
  private clipboardCb: ((text: string) => void) | undefined;
  private findCb: (() => void) | undefined;
  private findText: string | undefined;
  private scrollCb: ((atBottom: boolean) => void) | undefined;
  private lastAtBottom = true;
  private hoverCb: ((url: string | undefined) => void) | undefined;

  constructor() {
    const prefs = terminalPrefs();
    this.copyOnSelect = prefs.copyOnSelect;
    this.gpu = prefs.gpu;
    // Colors come from design tokens, so the terminal follows the theme.
    this.term = new Terminal({
      fontFamily: terminalFontFamily(token('--font-mono')),
      fontSize: terminalSize(),
      // A real terminal's rows are as tall as the font says: block and box-drawing characters then join.
      lineHeight: prefs.lineHeight,
      // 1 draws a program's colours as it chose them; 4 lifts the ones that are hard to read on this pane (a setting).
      minimumContrastRatio: prefs.readable ? 4 : 1,
      drawBoldTextInBrightColors: true,
      cursorStyle: 'block',
      cursorInactiveStyle: 'outline',
      rescaleOverlappingGlyphs: true,
      cursorBlink: false,
      scrollback: prefs.scrollback,
      // The wheel is decided in wheel.ts (a notch is three lines, a trackpad one line per row height).
      scrollSensitivity: 1,
      macOptionClickForcesSelection: true,
      screenReaderMode: false,
      allowProposedApi: true,
      linkHandler: { activate: (e, uri) => { if (e.ctrlKey || e.metaKey) openLink(uri); }, hover: (_e, uri) => this.hoverCb?.(uri), leave: () => this.hoverCb?.(undefined), allowNonHttpProtocols: false },
      theme: theme(),
    });
    this.term.loadAddon(this.fitAddon);
    this.term.loadAddon(this.searchAddon);
    // Full-width and emoji characters take two cells, as they do on the node's screen (xterm's default width table is older).
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.loadAddon(new WebLinksAddon((e, uri) => { if (e.ctrlKey || e.metaKey) openLink(uri); }, { hover: (_e, uri) => this.hoverCb?.(uri), leave: () => this.hoverCb?.(undefined) }));
    this.term.attachCustomKeyEventHandler((e) => this.onKey(e));
    this.term.onSelectionChange(() => { if (this.copyOnSelect && this.term.hasSelection()) void this.copy(); });
    this.term.onScroll(() => this.scrollState());
    this.term.onResize(() => { if (this.gl) this.checkGpu(); });
    this.term.onWriteParsed(() => this.scrollState());
    // The keyboard protocols a program may switch on for keys a plain terminal cannot tell apart (keys.ts).
    const p = this.term.parser;
    registerKeyboardProtocols(p, this.keys, (data) => this.term.input(data, false));
    // Mouse reports in the urxvt encoding (1015) when the program asked for it, which xterm.js does not write (mouse.ts).
    followMouseEncoding(this.term);
    // OSC 52: offered to the user, never written by itself, never answered (clipboard.ts).
    p.registerOscHandler(52, (data) => { const text = clipboardOffer(data); if (text !== undefined) this.clipboardCb?.(text); return true; });
    // A theme change repaints the terminal; the tokens have changed by the time the event fires.
    window.addEventListener('agora-theme', this.repaint);
  }

  private async copy(): Promise<boolean> {
    const text = this.term.getSelection();
    if (!text) return false;
    try { await navigator.clipboard.writeText(text); return true; } catch { return false; }
  }

  private scrollState() {
    const b = this.term.buffer.active;
    const atBottom = b.viewportY >= b.baseY;
    if (atBottom !== this.lastAtBottom) { this.lastAtBottom = atBottom; this.scrollCb?.(atBottom); }
  }

  private wheelState() {
    const screen = this.term.element?.querySelector('.xterm-screen');
    const cellHeight = screen && this.term.rows > 0 ? screen.getBoundingClientRect().height / this.term.rows : 16;
    return {
      altScreen: this.term.buffer.active.type === 'alternate',
      mouse: this.term.modes.mouseTrackingMode !== 'none',
      rows: this.term.rows,
      cellHeight,
    };
  }

  private apply(a: WheelAction) {
    if (a.kind === 'scroll') this.term.scrollLines(a.lines);
    else if (a.kind === 'arrows') {
      const arrow = `${this.term.modes.applicationCursorKeysMode ? '\x1bO' : '\x1b['}${a.up ? 'A' : 'B'}`;
      this.term.input(arrow.repeat(a.count), false);
    }
  }

  // Every wheel event over the terminal is decided here (wheel.ts) before xterm.js or its scroll bar see it: the scroll bar counts a notch
  // by the legacy `wheelDeltaY / 120`, which is 2.5 in Firefox and WebKit (7 history lines a notch there, 3 in Chromium), and xterm.js
  // asks a custom wheel handler only when its scroll bar did not take the event. Left to xterm.js: a report for a program that has the
  // mouse, and a trackpad's pixels over the main screen (its scroll bar scrolls them smoothly). Ctrl+wheel and a pinch are the browser's
  // (zoom): xterm.js would cancel them.
  private onWheel(e: WheelEvent): void {
    const a = this.wheel.decide(e, this.wheelState());
    if (a.kind === 'pass') return;
    e.stopPropagation();
    if (a.kind === 'browser') return;
    this.apply(a);
    // Nothing else may act on it: the terminal's own scroll bar would scroll by its own measure as well.
    e.preventDefault();
  }

  private onKey(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown') return true;
    const sc = shortcut(e, MAC);
    if (sc === 'copy') {
      if (this.term.hasSelection()) { void this.copy(); e.preventDefault(); return false; }
      // Nothing selected: Ctrl+Shift+C does nothing (the browser would open its inspector); Cmd+C is left to the browser.
      if (MAC) return true;
      e.preventDefault();
      return false;
    }
    if (sc === 'find') { e.preventDefault(); this.findCb?.(); return false; }
    const bytes = enhancedKey(e, this.keys.enhanced);
    if (bytes !== undefined) { e.preventDefault(); this.term.input(bytes, true); return false; }
    return true;
  }

  private repaint = () => {
    this.term.options.theme = theme();
    // Matches on show take the new theme's colours: drawn again and found again from the current match, as the addon itself does when
    // the screen changes (an incremental findPrevious keeps the current match in place).
    if (this.findText) {
      this.searchAddon.clearDecorations();
      this.searchAddon.findPrevious(this.findText, { decorations: searchColors(), incremental: true });
    }
    const font = terminalFontFamily(token('--font-mono'));
    if (font !== this.term.options.fontFamily) this.term.options.fontFamily = font;
  };

  setFontSize(px: number): void {
    this.term.options.fontSize = px;
  }

  setPrefs(p: TerminalPrefs): void {
    this.copyOnSelect = p.copyOnSelect;
    this.useGpu(p.gpu);
    const o = this.term.options;
    if (o.lineHeight !== p.lineHeight) o.lineHeight = p.lineHeight;
    if (o.scrollback !== p.scrollback) o.scrollback = p.scrollback;
    const contrast = p.readable ? 4 : 1;
    if (o.minimumContrastRatio !== contrast) o.minimumContrastRatio = contrast;
  }

  private gl: WebglAddon | undefined;
  private gpu = false;

  mount(el: HTMLElement): void {
    this.term.open(el);
    this.useGpu(this.gpu);
    // A paste from the keyboard or the menu is cleaned like the paste button's (clipboard.ts).
    this.term.textarea?.addEventListener('paste', (e) => {
      const text = e.clipboardData?.getData('text/plain');
      if (text === undefined) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (text) this.term.paste(sanitizePaste(text));
    }, true);
    const signal = this.listeners.signal;
    el.addEventListener('wheel', (e) => this.onWheel(e), { capture: true, passive: false, signal });
    // A finger drags the history (the terminal's own handling does not on a touch screen).
    el.addEventListener('touchstart', (e) => { if (e.touches.length === 1) this.touch.start(e.touches[0]?.clientY ?? 0); else this.touch.end(); }, { passive: true, signal });
    el.addEventListener('touchmove', (e) => {
      if (e.touches.length !== 1) return;
      this.apply(this.touch.move(e.touches[0]?.clientY ?? 0, this.wheelState()));
      if (e.cancelable) e.preventDefault();
    }, { passive: false, signal });
    el.addEventListener('touchend', () => this.touch.end(), { passive: true, signal });
    el.addEventListener('touchcancel', () => this.touch.end(), { passive: true, signal });
  }

  // The graphics-card renderer draws box-drawing, block and Powerline characters itself, exactly to the cell, so they join; the page's
  // own text drawing cannot (a font's glyphs are never the whole cell). Where it cannot start, or the browser takes its context away
  // (browsers allow about sixteen per page), the terminal keeps the page's drawing.
  private useGpu(on: boolean) {
    this.gpu = on;
    if (!this.term.element) return;
    if (!on) { this.gl?.dispose(); this.gl = undefined; return; }
    if (this.gl) return;
    try {
      const gl = new WebglAddon();
      gl.onContextLoss(() => { gl.dispose(); if (this.gl === gl) this.gl = undefined; });
      this.term.loadAddon(gl);
      this.gl = gl;
      this.checkGpu();
    } catch { this.gl = undefined; }
  }

  // A graphics-card canvas that is not as many device pixels wide as the page shows it (a browser or a driver that does not scale
  // it for this screen) would draw nothing or a blur: the terminal then goes back to the page's drawing. Checked 250 ms after the
  // renderer starts and after each resize; a browser that settles later is not caught, and the pane then shows the page's drawing only
  // after the next resize. This is why the renderer is opt-in (prefs.ts).
  private checkGpu() {
    window.setTimeout(() => {
      const canvas = this.gl && this.term.element?.querySelector<HTMLCanvasElement>('.xterm-screen canvas:not(.xterm-link-layer)');
      if (!canvas) return;
      const want = canvas.getBoundingClientRect().width * window.devicePixelRatio;
      if (want > 0 && canvas.width < want * 0.9) this.useGpu(false);
    }, 250);
  }

  write(bytes: Uint8Array): void {
    this.term.write(bytes);
  }

  reset(snapshot: Uint8Array, history?: Uint8Array): void {
    // The snapshot clears the screen and draws every row itself, so nothing is reset first: a reset (a clear, then the repaint a moment
    // later) is painted as an empty screen between the two, on every reconnect. Synchronized output (mode 2026) makes the whole repaint
    // one paint. History (kind 4, ADR-0100), when the window asked for it, goes into the same write, just before the snapshot: it fills
    // the scrollback and leaves the screen clear for the snapshot, so a reload paints once, at the size both were made for (the window
    // took it before calling this, frames.ts). Without it (a reconnect, falling behind) the history this window gathered stays. A
    // current node's repaint sets every mode it tracks (ADR-0100 decision 3); an older node's sets none, and then the modes learned from
    // the stream stay.
    const head = new TextEncoder().encode('\x1b[?2026h');
    const tail = new TextEncoder().encode('\x1b[?2026l');
    const past = history ?? new Uint8Array();
    const all = new Uint8Array(head.length + past.length + snapshot.length + tail.length);
    let at = 0;
    for (const part of [head, past, snapshot, tail]) { all.set(part, at); at += part.length; }
    this.term.write(all);
  }

  resize(cols: number, rows: number): void {
    if (cols !== this.term.cols || rows !== this.term.rows) this.term.resize(cols, rows);
  }

  measure(): { cols: number; rows: number } | undefined {
    const d = this.fitAddon.proposeDimensions();
    if (!d || !Number.isFinite(d.cols) || !Number.isFinite(d.rows) || d.cols < 1 || d.rows < 1) return undefined;
    return { cols: d.cols, rows: d.rows };
  }

  onInput(cb: (data: string, binary?: boolean) => void): void {
    this.term.onData((d) => cb(d));
    // The older mouse encodings (X10, and urxvt as mouse.ts writes it) arrive as bytes, not text: each char is one byte as it is.
    this.term.onBinary((s) => cb(s, true));
  }

  paste(text: string): void {
    this.term.paste(sanitizePaste(text));
  }

  applicationCursor(): boolean {
    return this.term.modes.applicationCursorKeysMode;
  }

  focus(): void {
    this.term.focus();
  }

  onScrollState(cb: (atBottom: boolean) => void): void { this.scrollCb = cb; }
  scrollToBottom(): void { this.term.scrollToBottom(); }
  onClipboardOffer(cb: (text: string) => void): void { this.clipboardCb = cb; }
  copySelection(): boolean { if (!this.term.hasSelection()) return false; void this.copy(); return true; }
  onBell(cb: () => void): void { this.term.onBell(cb); }
  onLinkHover(cb: (url: string | undefined) => void): void { this.hoverCb = cb; }
  onFindRequest(cb: () => void): void { this.findCb = cb; }

  find(text: string, backwards: boolean): boolean {
    this.findText = text || undefined;
    if (!text) { this.searchAddon.clearDecorations(); return false; }
    const opts = { decorations: searchColors(), incremental: false };
    return backwards ? this.searchAddon.findPrevious(text, opts) : this.searchAddon.findNext(text, opts);
  }

  clearFind(): void {
    this.findText = undefined;
    this.searchAddon.clearDecorations();
  }

  dispose(): void {
    window.removeEventListener('agora-theme', this.repaint);
    this.listeners.abort();
    this.term.dispose();
  }
}
