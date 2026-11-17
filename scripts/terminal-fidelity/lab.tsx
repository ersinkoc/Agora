// The real TerminalView on a fake source, for the real-browser checks in run.mjs. The page gets: window.__sent (what the terminal sent to
// the node, one string per call, each character one byte), window.__emit(text|bytes, kind) to write output or a snapshot, window.__resize
// (cols, rows) for the node's size frame, and the hooks __onInput / __onReport that run.mjs uses to wire a real program on a PTY.
import { createRoot } from 'react-dom/client';
import '../../apps/web/src/design/app.css';
import '../../apps/web/src/phone/phone.css';
import { initTheme } from '../../apps/web/src/themes/store';
import { TerminalView } from '../../apps/web/src/terminal/TerminalView';
import type { TerminalSource } from '../../apps/web/src/terminal/source';

initTheme();
const w = window as unknown as Record<string, any>;
w.__sent = [] as string[];
const source: TerminalSource = {
  connect(on) {
    w.__emit = (b: Uint8Array | string, type = 'data') => on({ type, bytes: typeof b === 'string' ? new TextEncoder().encode(b) : b } as never);
    w.__resize = (cols: number, rows: number) => on({ type: 'resize', cols, rows });
    return {
      sendInput(b) { const s = String.fromCharCode(...b); w.__sent.push(s); w.__onInput?.(s); },
      // Without a node the window's own size is the terminal's size (one window); __noecho keeps the node's size frame under the test's control.
      resize(cols, rows) { w.__reported = { cols, rows }; if (!w.__noecho) on({ type: 'resize', cols, rows }); w.__onReport?.(cols, rows); },
      close() {},
    };
  },
};
const slot = document.getElementById('slot');
if (slot) createRoot(slot).render(<TerminalView source={source} label="Terminal" />);
