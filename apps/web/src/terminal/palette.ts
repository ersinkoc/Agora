// The colours xterm draws a terminal with (theme format 2, ADR-0113): the theme's terminal palette where it sets one, else what every
// terminal drew before: the pane's background and text, the accent as cursor and selection, and the scheme's 16 colours. Pure: the
// page's variables come in through `read` (a variable's value, '' when unset), so it is tested without a page.
import { ANSI, ANSI_NAMES, type AnsiName, type TerminalKey, terminalVar } from '../themes/look';

const HEX6 = /^#[0-9a-f]{6}$/i;
// xterm's names for the 16 colours (`brightBlack`).
type Camel<S extends string> = S extends `${infer A}-${infer B}` ? `${A}${Capitalize<B>}` : S;
export type XtermColour = Camel<AnsiName>;
const camel = (n: AnsiName) => n.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase()) as XtermColour;

export function terminalTheme(read: (name: string) => string, light: boolean) {
  const own = (k: TerminalKey) => { const v = read(terminalVar(k)).trim(); return HEX6.test(v) ? v : undefined; };
  const accent = read('--accent').trim();
  // The selection is drawn at about a third of its strength (a real terminal inverts it), fainter in a terminal not in focus, and
  // text keeps its own colour on top.
  const selection = own('selection') ?? (HEX6.test(accent) ? accent : '#8b93ff');
  const palette = ANSI[light ? 'light' : 'dark'];
  return {
    background: own('background') ?? read('--surface-0').trim(),
    foreground: own('foreground') ?? read('--text').trim(),
    cursor: own('cursor') ?? accent,
    selectionBackground: `${selection}66`,
    selectionInactiveBackground: `${selection}33`,
    ...(Object.fromEntries(ANSI_NAMES.map((n, i) => [camel(n), own(n) ?? palette[i]])) as Record<XtermColour, string>),
  };
}
