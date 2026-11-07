// The renderer behind an interface (ADR-0017): xterm.js today, swappable for a WebAssembly emulator once the
// choice is measured against the budgets in ADR-0022. Nothing outside this folder imports xterm.
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';

export interface TerminalRenderer {
  mount(el: HTMLElement): void;
  write(bytes: Uint8Array): void;
  /** Clears and repaints from an ANSI snapshot (after a reconnect or when the client fell behind). */
  reset(snapshot: Uint8Array): void;
  resize(cols: number, rows: number): void;
  /** Size that fits the mounted element, to report to the node. */
  fit(): { cols: number; rows: number } | undefined;
  onInput(cb: (data: string) => void): void;
  focus(): void;
  dispose(): void;
}

function token(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export class XtermRenderer implements TerminalRenderer {
  private term: Terminal;
  private fitAddon = new FitAddon();

  constructor() {
    // Colors come from design tokens, so the terminal follows the theme.
    this.term = new Terminal({
      fontFamily: token('--font-mono') || 'monospace',
      fontSize: 13,
      cursorBlink: false,
      scrollback: 2000,
      screenReaderMode: false,
      theme: { background: token('--surface-0'), foreground: token('--text'), cursor: token('--accent') },
    });
    this.term.loadAddon(this.fitAddon);
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

  focus(): void {
    this.term.focus();
  }

  dispose(): void {
    this.term.dispose();
  }
}
