// Characters that are invisible or change how text around them looks: the one list in crates/agora-core/src/hidden-characters.txt
// (Default_Ignorable_Code_Point and every format character, the line and paragraph separators, the private use areas).
export const HIDDEN_RANGES: readonly (readonly [number, number])[] = [
  [0x00ad, 0x00ad], [0x034f, 0x034f], [0x0600, 0x0605], [0x061c, 0x061c], [0x06dd, 0x06dd], [0x070f, 0x070f], [0x0890, 0x0891],
  [0x08e2, 0x08e2], [0x115f, 0x1160], [0x17b4, 0x17b5], [0x180b, 0x180f], [0x200b, 0x200f], [0x2028, 0x202e], [0x2060, 0x206f],
  [0x2800, 0x2800], [0x3164, 0x3164], [0xfe00, 0xfe0f], [0xfeff, 0xfeff], [0xffa0, 0xffa0], [0xfff0, 0xfffb], [0x110bd, 0x110bd], [0x110cd, 0x110cd],
  [0x13430, 0x1343f], [0x1bca0, 0x1bca3], [0x1d173, 0x1d17a], [0xe0000, 0xe0fff], [0xe000, 0xf8ff], [0xf0000, 0xffffd],
  [0x100000, 0x10fffd],
];

const hex = (n: number) => `\\u{${n.toString(16)}}`;
const SOURCE = `[${HIDDEN_RANGES.map(([lo, hi]) => (lo === hi ? hex(lo) : `${hex(lo)}-${hex(hi)}`)).join('')}]|[\\u0000-\\u0008\\u000b-\\u001f\\u007f-\\u009f]`;
const HIDDEN = new RegExp(SOURCE, 'gu');
const HAS_HIDDEN = new RegExp(SOURCE, 'u');

export function hasHidden(text: string): boolean {
  return HAS_HIDDEN.test(text);
}

export function replaceHidden(text: string, replacement = ' '): string {
  return text.replace(HIDDEN, replacement);
}

export function visible(text: string): string {
  return text.replace(HIDDEN, (c) => `\\u{${(c.codePointAt(0) ?? 0).toString(16)}}`);
}
