// The mouse encodings a program can ask for (DEC private modes 1005, 1006, 1015, 1016) and the urxvt form (1015), which xterm.js
// does not write (it logs "not supported" and keeps writing the classic form, which a program that asked for 1015 misreads). The
// renderer follows the modes in the program's output and rewrites the report xterm.js made into the urxvt form. Pure.

export type MouseEncoding = 'classic' | 'utf8' | 'sgr' | 'urxvt' | 'sgr-pixels';

const MODES: Record<number, MouseEncoding> = { 1005: 'utf8', 1006: 'sgr', 1015: 'urxvt', 1016: 'sgr-pixels' };

export class MouseEncodingModes {
  encoding: MouseEncoding = 'classic';

  /** A DECSET (`set`) or DECRST with these parameters. As in xterm, the encodings exclude each other: the last one set is the one,
   * and a reset ends only the encoding it names. */
  apply(params: readonly (number | readonly number[])[], set: boolean): void {
    for (const p of params) {
      const e = typeof p === 'number' ? MODES[p] : undefined;
      if (!e) continue;
      if (set) this.encoding = e;
      else if (this.encoding === e) this.encoding = 'classic';
    }
  }

  /** A full reset of the terminal (RIS, `ESC c`), which xterm.js also answers by going back to the classic form. */
  reset(): void { this.encoding = 'classic'; }
}

const SGR = /^\x1b\[<(\d+);(\d+);(\d+)([Mm])$/;

/** The urxvt form, `ESC [ Cb ; Cx ; Cy M` (Cb is the classic button code plus 32, a release is button 3, coordinates count from 1),
 * of a report xterm.js wrote in the classic form (`ESC [ M` and three bytes) or in SGR; undefined for anything else. */
export function urxvtReport(data: string): string | undefined {
  if (data.length === 6 && data.startsWith('\x1b[M')) {
    return `\x1b[${data.charCodeAt(3)};${data.charCodeAt(4) - 32};${data.charCodeAt(5) - 32}M`;
  }
  const m = SGR.exec(data);
  if (!m) return undefined;
  // Only SGR can say which button was let go; every other form reports a release as button 3, keeping the modifiers.
  const code = Number(m[1]) | (m[4] === 'm' ? 3 : 0);
  return `\x1b[${code + 32};${m[2]};${m[3]}M`;
}
