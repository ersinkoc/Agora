import { type TerminalEvent } from './frames';

// Where a terminal's bytes come from. The hub connection implements this; until it exists the demo source stands in.
export interface TerminalSession {
  sendInput(bytes: Uint8Array): void;
  resize(cols: number, rows: number): void;
  close(): void;
}

// `historyLines`: how many lines of history the window wants before the first screen (ADR-0100 decision 2), its own history
// setting; the node sends at most what it keeps. The window that asks has no history of its own yet (a first open, a reload, a
// remount). A source asks for it only until that window has a screen: after a reconnect or falling behind it asks for none, because
// the window keeps its own (often longer) history and a replay would repeat it.
export interface ConnectOptions {
  historyLines?: number;
}

export interface TerminalSource {
  connect(onEvent: (e: TerminalEvent) => void, options?: ConnectOptions): TerminalSession;
}

// A local stand-in: paints a prompt and echoes what is typed, so the Focus view is usable without a node.
export function demoSource(banner: string): TerminalSource {
  return {
    connect(onEvent) {
      const enc = new TextEncoder();
      const dec = new TextDecoder();
      onEvent({ type: 'snapshot', bytes: enc.encode(`${banner.replace(/\n/g, '\r\n')}\r\n$ `) });
      return {
        sendInput(bytes) {
          const text = dec.decode(bytes);
          const out = text.replace(/\r/g, '\r\n$ ').replace(/\x7f/g, '\b \b');
          onEvent({ type: 'data', bytes: enc.encode(out) });
        },
        // No node: the terminal takes the size that fits, as a node with this one window would give it.
        resize(cols, rows) { onEvent({ type: 'resize', cols, rows }); },
        close() {},
      };
    },
  };
}
