// The renderer behind an interface (ADR-0017): xterm.js today, swappable for a WebAssembly emulator once the
// choice is measured against the budgets in ADR-0022. Nothing outside this folder imports xterm.
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { terminalSize } from './prefs';

export interface TerminalRenderer {
  mount(el: HTMLElement): void;
  write(bytes: Uint8Array): void;
  /** Clears and repaints from an ANSI snapshot (after a reconnect or when the client fell behind). */
  reset(snapshot: Uint8Array): void;
  resize(cols: number, rows: number): void;
  /** Size that fits the mounted element, to report to the node. */
  fit(): { cols: number; rows: number } | undefined;
  onInput(cb: (data: string) => void): void;
  /** Pastes text as the program expects it (bracketed when it asked for that); it arrives through `onInput`. */
  paste(text: string): void;
  /** Whether the program asked for application cursor keys (DECCKM), which changes the bytes an arrow sends. */
  applicationCursor(): boolean;
  focus(): void;
  setFontSize(px: number): void;
  dispose(): void;
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

// The 16 colours programs draw with must stay readable on the pane's background: one palette per scheme.
const ANSI_DARK = ['#3b3b44', '#f87171', '#4ade80', '#fbbf24', '#60a5fa', '#c084fc', '#22d3ee', '#d4d4d8', '#71717a', '#fca5a5', '#86efac', '#fde68a', '#93c5fd', '#d8b4fe', '#67e8f9', '#fafafa'];
const ANSI_LIGHT = ['#27272a', '#b91c1c', '#15803d', '#a16207', '#1d4ed8', '#7e22ce', '#0e7490', '#52525b', '#52525b', '#dc2626', '#16a34a', '#ca8a04', '#2563eb', '#9333ea', '#0891b2', '#18181b'];
function palette() {
  const root = document.documentElement.dataset.theme;
  const light = root ? root === 'light' : !!window.matchMedia?.('(prefers-color-scheme: light)').matches;
  const a = light ? ANSI_LIGHT : ANSI_DARK;
  const [black, red, green, yellow, blue, magenta, cyan, white, brightBlack, brightRed, brightGreen, brightYellow, brightBlue, brightMagenta, brightCyan, brightWhite] = a;
  return { black, red, green, yellow, blue, magenta, cyan, white, brightBlack, brightRed, brightGreen, brightYellow, brightBlue, brightMagenta, brightCyan, brightWhite };
}
function theme() {
  return { background: token('--surface-0'), foreground: token('--text'), cursor: token('--accent'), selectionBackground: token('--surface-2'), ...palette() };
}

export class XtermRenderer implements TerminalRenderer {
  private term: Terminal;
  private fitAddon = new FitAddon();

  constructor() {
    // Colors come from design tokens, so the terminal follows the theme.
    this.term = new Terminal({
      fontFamily: token('--font-mono') || 'monospace',
      fontSize: terminalSize(),
      lineHeight: 1.1,
      // A program's own colour can be unreadable on this pane (dark blue on black): lift it just enough.
      minimumContrastRatio: 4,
      drawBoldTextInBrightColors: true,
      cursorStyle: 'block',
      cursorInactiveStyle: 'outline',
      rescaleOverlappingGlyphs: true,
      cursorBlink: false,
      scrollback: 2000,
      screenReaderMode: false,
      theme: theme(),
    });
    this.term.loadAddon(this.fitAddon);
    // A theme change repaints the terminal; the tokens have changed by the time the event fires.
    window.addEventListener('agora-theme', this.repaint);
  }

  private repaint = () => { this.term.options.theme = theme(); };

  setFontSize(px: number): void {
    this.term.options.fontSize = px;
  }

  mount(el: HTMLElement): void {
    this.term.open(el);
  }

  write(bytes: Uint8Array): void {
    this.term.write(bytes);
  }

  reset(snapshot: Uint8Array): void {
    this.term.reset();
    this.term.write(snapshot);
  }

  resize(cols: number, rows: number): void {
    this.term.resize(cols, rows);
  }

  fit(): { cols: number; rows: number } | undefined {
    this.fitAddon.fit();
    return { cols: this.term.cols, rows: this.term.rows };
  }

  onInput(cb: (data: string) => void): void {
    this.term.onData(cb);
  }

  paste(text: string): void {
    this.term.paste(text);
  }

  applicationCursor(): boolean {
    return this.term.modes.applicationCursorKeysMode;
  }

  focus(): void {
    this.term.focus();
  }

  dispose(): void {
    window.removeEventListener('agora-theme', this.repaint);
    this.term.dispose();
  }
}
