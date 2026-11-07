import { type TerminalEvent } from './frames';

// Where a terminal's bytes come from. The hub connection implements this; until it exists the demo source stands in.
export interface TerminalSession {
  sendInput(bytes: Uint8Array): void;
  resize(cols: number, rows: number): void;
  close(): void;
}

export interface TerminalSource {
  connect(onEvent: (e: TerminalEvent) => void): TerminalSession;
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
        resize() {},
        close() {},
      };
    },
  };
}
