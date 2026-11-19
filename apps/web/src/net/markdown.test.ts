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
});
