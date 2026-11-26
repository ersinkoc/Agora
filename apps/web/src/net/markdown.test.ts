import { describe, expect, it } from 'vitest';
import { blocks, inlines } from './markdown';

describe('agent markdown', () => {
  it('reads inline code and bold, and nothing inside code', () => {
    expect(inlines('run `cargo **test**` then **check**')).toEqual([
      { kind: 'text', text: 'run ' }, { kind: 'code', text: 'cargo **test**' }, { kind: 'text', text: ' then ' }, { kind: 'strong', text: 'check' },
    ]);
    expect(inlines('a_b * c `` d')).toEqual([{ kind: 'text', text: 'a_b * c `` d' }]);
  });
  it('keeps HTML as text', () => {
    expect(blocks('<img src=x onerror=alert(1)>')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: '<img src=x onerror=alert(1)>' }] }]);
  });
  it('splits paragraphs, headings, lists, quotes and code', () => {
    const b = blocks('## Done\n\nI changed two things:\n\n1. the test\n2. the `redirect`\n   and its docs\n\n- a\n- b\n\n> note\n\n```rust\nfn main() {}\n```\nafter');
    expect(b.map((x) => x.kind)).toEqual(['heading', 'p', 'list', 'list', 'quote', 'code', 'p']);
    expect(b[2]).toMatchObject({ ordered: true, start: 1, items: [[{ text: 'the test' }], [{ text: 'the ' }, { kind: 'code', text: 'redirect' }, { text: '\nand its docs' }]] });
    expect(b[3]).toMatchObject({ ordered: false });
    expect(b[5]).toEqual({ kind: 'code', lang: 'rust', text: 'fn main() {}' });
  });
  it('closes a code block only on a whole fence of the same kind and length', () => {
    expect(blocks('````md\n```not-a-close\n```\n~~~~\n````\nafter')).toEqual([
      { kind: 'code', lang: 'md', text: '```not-a-close\n```\n~~~~' }, { kind: 'p', inlines: [{ kind: 'text', text: 'after' }] },
    ]);
  });
  it('treats a code block still being written as code to the end', () => {
    expect(blocks('look:\n```\nline 1\nline 2')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: 'look:' }] }, { kind: 'code', lang: '', text: 'line 1\nline 2' }]);
  });
  it('reads a table with a header into cells, and keeps one without as the text it was', () => {
    const t = blocks('| a | b |\n|---|---|\n| 1 | `2` |')[0];
    expect(t).toMatchObject({ kind: 'table', text: '| a | b |\n|---|---|\n| 1 | `2` |', head: [[{ text: 'a' }], [{ text: 'b' }]], rows: [[[{ text: '1' }], [{ kind: 'code', text: '2' }]]] });
    expect(blocks('| a | b |\n| 1 | 2 |')).toEqual([{ kind: 'table', text: '| a | b |\n| 1 | 2 |' }]);
  });
  it('draws web links as links and nothing else (no javascript:, data:, file: or credentials)', () => {
    expect(inlines('see [the docs](https://example.com/a?b=1) now')).toEqual([
      { kind: 'text', text: 'see ' }, { kind: 'link', text: 'the docs', href: 'https://example.com/a?b=1' }, { kind: 'text', text: ' now' },
    ]);
    expect(inlines('go to https://example.com/x.')).toEqual([{ kind: 'text', text: 'go to ' }, { kind: 'link', text: 'https://example.com/x', href: 'https://example.com/x' }, { kind: 'text', text: '.' }]);
    for (const bad of ['[x](javascript:alert(1))', '[x](data:text/html,hi)', '[x](file:///etc/passwd)', '[x](https://user:pw@example.com/)', '[x](JaVaScRiPt:alert`1`)']) {
      expect(inlines(bad).some((p) => p.kind === 'link'), bad).toBe(false);
    }
    // Inside code nothing is read as a link.
    expect(inlines('`[x](https://a.b)`')).toEqual([{ kind: 'code', text: '[x](https://a.b)' }]);
  });
  it('never turns what an agent writes into HTML', () => {
    const all = blocks('<script>alert(1)</script>\n\n[<img src=x onerror=alert(1)>](https://a.b)\n\n| <b>x</b> |\n|---|\n| <i>y</i> |');
    expect(JSON.stringify(all)).not.toContain('"kind":"html"');
    expect(all[0]).toEqual({ kind: 'p', inlines: [{ kind: 'text', text: '<script>alert(1)</script>' }] });
  });
  it('keeps the lines of a paragraph, and a dash inside one is not a list', () => {
    expect(blocks('first\n- not a list\nsecond')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: 'first\n- not a list\nsecond' }] }]);
    expect(blocks('')).toEqual([]);
  });
  // Audit F2080: what an agent writes reaches `blocks` whole (a reply joins its chunks, up to the node's 256 KiB). A heading, a fence and a
  // table's separator row with a long run of spaces before one stray character each took seconds (the heading's time grew with the cube of
  // its length) and froze the window.
  it('reads a long line of spaces in linear time, and still reads headings, fences and tables as before', () => {
    const spaces = (n: number) => ' '.repeat(n);
    for (const line of [`# a${spaces(2000)}b`, `\`\`\`${spaces(48_000)}!`, `| a |\n|-${spaces(48_000)}x|`]) {
      const start = performance.now();
      blocks(line);
      expect(performance.now() - start, line.slice(0, 12)).toBeLessThan(200);
    }
    expect(blocks(`# a${spaces(2000)}b`)).toEqual([{ kind: 'heading', inlines: [{ kind: 'text', text: `a${spaces(2000)}b` }] }]);
    expect(blocks('  ## Done ##  ')).toEqual([{ kind: 'heading', inlines: [{ kind: 'text', text: 'Done' }] }]);
    // A closing run of `#` needs a space before it (CommonMark): `# C#` is the heading "C#".
    expect(blocks('# C#')).toEqual([{ kind: 'heading', inlines: [{ kind: 'text', text: 'C#' }] }]);
    expect(blocks('# C# ##')).toEqual([{ kind: 'heading', inlines: [{ kind: 'text', text: 'C#' }] }]);
    expect(blocks(`# a${' '.repeat(48_000)}#x`)).toEqual([{ kind: 'heading', inlines: [{ kind: 'text', text: `a${' '.repeat(48_000)}#x` }] }]);
    expect(blocks('# a # b')).toEqual([{ kind: 'heading', inlines: [{ kind: 'text', text: 'a # b' }] }]);
    expect(blocks('#   ')).toEqual([{ kind: 'heading', inlines: [] }]);
    expect(blocks('####### seven')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: '####### seven' }] }]);
    expect(blocks('  ~~~  py  \nx\n ~~~ ')).toEqual([{ kind: 'code', lang: 'py', text: 'x' }]);
    expect(blocks('| a |\n| :-: | --- |')).toMatchObject([{ kind: 'table', head: [[{ text: 'a' }]], rows: [] }]);
    expect(blocks('| a |\n|---||')).toEqual([{ kind: 'table', text: '| a |\n|---||' }]);
  });
  // Audit F2082: three more took time with the square of the line: links looked for in a run of `[`, a list item holding a line separator
  // (U+2028), and the closing punctuation cut from a web address made of a run of dots.
  it('reads a run of brackets, an item with a line separator and an address of dots in linear time, as before', () => {
    for (const line of ['['.repeat(48_000), `- ${' '.repeat(48_000)}a\u2028b`, `https://x${'.'.repeat(48_000)}a`]) {
      const start = performance.now();
      blocks(line);
      expect(performance.now() - start, line.slice(0, 12)).toBeLessThan(200);
    }
    expect(inlines('[a[b](https://x.y) [c](https://u.v/w) [d](ftp://e)')).toEqual([
      { kind: 'link', text: 'a[b', href: 'https://x.y/' }, { kind: 'text', text: ' ' }, { kind: 'link', text: 'c', href: 'https://u.v/w' },
      { kind: 'text', text: ' [d](ftp://e)' },
    ]);
    expect(inlines('see https://a.b/c... and [x](https://d.e)!')).toMatchObject([
      { kind: 'text', text: 'see ' }, { kind: 'link', text: 'https://a.b/c' }, { kind: 'text', text: '... and ' }, { kind: 'link', text: 'x' }, { kind: 'text', text: '!' },
    ]);
    expect(blocks('- a\u2028b')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: '- a\u2028b' }] }]);
    expect(blocks('  12) x\n  13) y\n   - z')).toMatchObject([{ kind: 'list', ordered: true, start: 12, items: [[{ text: 'x' }], [{ text: 'y' }], [{ text: 'z' }]] }]);
  });
});
