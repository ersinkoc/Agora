import { Terminal } from '@xterm/xterm';
import { afterEach, describe, expect, it } from 'vitest';
import { binaryBytes, inputBytes } from './frames';
import { encodeMouse, followMouseEncoding, UrxvtMode, type MouseReportEvent } from './mouse';

// Column and row from 1, as xterm.js hands them to an encoding.
const ev = (o: Partial<MouseReportEvent> = {}): MouseReportEvent => ({ col: 6, row: 4, x: 45, y: 63, button: 0, action: 1, ctrl: false, alt: false, shift: false, ...o });
const up = (o: Partial<MouseReportEvent> = {}) => ev({ action: 0, ...o });

describe('a mouse report in the classic and urxvt forms', () => {
  it('writes a click as xterm and urxvt do', () => {
    expect(encodeMouse(ev(), 'classic') + encodeMouse(up(), 'classic')).toBe('\x1b[M &$\x1b[M#&$');
    expect(encodeMouse(ev(), 'urxvt') + encodeMouse(up(), 'urxvt')).toBe('\x1b[32;6;4M\x1b[35;6;4M');
  });
  it('keeps the wheel, motion and modifiers; a release is button 3', () => {
    expect(encodeMouse(ev({ button: 4, action: 0 }), 'urxvt')).toBe('\x1b[96;6;4M'); // wheel up
    expect(encodeMouse(ev({ button: 4, action: 1 }), 'urxvt')).toBe('\x1b[97;6;4M'); // wheel down
    expect(encodeMouse(ev({ action: 32 }), 'urxvt')).toBe('\x1b[64;6;4M'); // drag with button 1
    expect(encodeMouse(ev({ button: 3, action: 32 }), 'urxvt')).toBe('\x1b[67;6;4M'); // motion, no button
    expect(encodeMouse(up({ button: 2, ctrl: true, shift: true }), 'urxvt')).toBe('\x1b[55;6;4M');
  });
  it('has no coordinate limit in the urxvt form; the classic one sends nothing past 223', () => {
    expect(encodeMouse(ev({ col: 300, row: 80 }), 'urxvt')).toBe('\x1b[32;300;80M');
    expect(encodeMouse(ev({ col: 223, row: 223 }), 'classic')).toBe(`\x1b[M ${String.fromCharCode(255, 255)}`);
    expect(encodeMouse(ev({ col: 224 }), 'classic')).toBe('');
  });
});

describe('mode 1015', () => {
  it('is set and reset by itself only, and ends on a full reset; sub-parameters are not it', () => {
    const m = new UrxvtMode();
    m.apply([1000, 1015], true);
    expect(m.on).toBe(true);
    m.apply([1006], true); m.apply([1006, 1000], false);
    expect(m.on).toBe(true);
    m.apply([1015], false);
    expect(m.on).toBe(false);
    m.apply([1015], true); m.reset();
    expect(m.on).toBe(false);
    m.apply([[1015, 1]], true);
    expect(m.on).toBe(false);
  });
});

// The real xterm.js, its parser and its mouse service: what a program receives after each sequence of modes.
describe('reports from xterm.js after the program changes encodings', () => {
  const terms: Terminal[] = [];
  afterEach(() => { for (const t of terms.splice(0)) t.dispose(); });
  type Core = { coreMouseService: { triggerMouseEvent(e: object): boolean; activeEncoding: string } };
  const setup = async (modes: string) => {
    const term = new Terminal({ allowProposedApi: true, cols: 300, rows: 100 });
    terms.push(term);
    expect(followMouseEncoding(term)).toBeDefined();
    const sent: string[] = [];
    // The bytes the program receives, as the renderer and TerminalView pass them on (latin1 here, to compare).
    const bytes = (b: Uint8Array) => sent.push(String.fromCharCode(...b));
    term.onData((d) => bytes(inputBytes(d)));
    term.onBinary((d) => bytes(binaryBytes(d)));
    await new Promise<void>((r) => term.write(`\x1b[?1000h${modes}`, r));
    const core = (term as unknown as { _core: Core })._core;
    // Cell (5,3), 0-based, at 45,63 px: press and release of the left button.
    const click = (col = 5, row = 3) => {
      sent.length = 0;
      for (const action of [1, 0]) core.coreMouseService.triggerMouseEvent({ col, row, x: 45, y: 63, button: 0, action, ctrl: false, alt: false, shift: false });
      return sent.join('');
    };
    return { click, xterm: () => core.coreMouseService.activeEncoding };
  };
  const urxvt = '\x1b[32;6;4M\x1b[35;6;4M';
  const classic = '\x1b[M &$\x1b[M#&$';
  const sgr = '\x1b[<0;6;4M\x1b[<0;6;4m';

  it('1015 alone: urxvt, also past column 223', async () => {
    const t = await setup('\x1b[?1015h');
    expect(t.click()).toBe(urxvt);
    expect(t.click(250, 3)).toBe('\x1b[32;251;4M\x1b[35;251;4M');
  });
  it('1006 and 1015 both on, in either order: SGR, which wins whenever it is on', async () => {
    expect((await setup('\x1b[?1006h\x1b[?1015h')).click()).toBe(sgr);
    expect((await setup('\x1b[?1015h\x1b[?1006h')).click()).toBe(sgr);
  });
  it('1006, 1015, then 1015 reset: SGR as xterm.js has it; then 1006 reset with 1015 on: urxvt', async () => {
    const t = await setup('\x1b[?1006h\x1b[?1015h\x1b[?1015l');
    expect(t.xterm()).toBe('SGR');
    expect(t.click()).toBe(sgr);
    const r = await setup('\x1b[?1006h\x1b[?1015h\x1b[?1006l');
    expect(r.xterm()).toBe('DEFAULT');
    expect(r.click()).toBe(urxvt);
  });
  it('1016 then 1015: SGR pixels as xterm.js writes them, never pixels passed off as cells', async () => {
    const t = await setup('\x1b[?1016h\x1b[?1015h');
    expect(t.xterm()).toBe('SGR_PIXELS');
    expect(t.click()).toBe('\x1b[<0;45;63M\x1b[<0;45;63m');
  });
  it('no encoding asked for, 1015 reset, or a full reset: the classic form, bytes as they are', async () => {
    const t = await setup('');
    expect(t.click()).toBe(classic);
    expect(t.click(150, 3)).toBe(`\x1b[M ${String.fromCharCode(183)}$\x1b[M#${String.fromCharCode(183)}$`);
    expect((await setup('\x1b[?1015h\x1b[?1015l')).click()).toBe(classic);
    expect((await setup('\x1b[?1015h\x1bc\x1b[?1000h')).click()).toBe(classic);
  });
});
