import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

// xterm.js gives its own element the class `terminal`. A rule of ours on `.terminal` (padding, height, clipping) lands on that element
// too, pushes the screen out of xterm's box, and the lowest pixels of the bottom row stop reaching the program (they hit the host).
const sheets = ['../design/app.css', '../phone/phone.css'];
const bareTerminal = /\.terminal(?![\w-])/;

describe('the element the terminal is mounted in', () => {
  it('is styled through a class xterm.js does not use', () => {
    for (const sheet of sheets) {
      const css = readFileSync(new URL(sheet, import.meta.url), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
      const hits = css.split('\n').filter((l) => bareTerminal.test(l));
      expect(hits, sheet).toEqual([]);
    }
    const view = readFileSync(new URL('./TerminalView.tsx', import.meta.url), 'utf8');
    expect(view).toContain('className="terminal-host" role="region"');
  });
});
