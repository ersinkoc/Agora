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
  it('treats a code block still being written as code to the end', () => {
    expect(blocks('look:\n```\nline 1\nline 2')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: 'look:' }] }, { kind: 'code', lang: '', text: 'line 1\nline 2' }]);
  });
  it('keeps a table as it is, aligned', () => {
    expect(blocks('| a | b |\n|---|---|\n| 1 | 2 |')).toEqual([{ kind: 'table', text: '| a | b |\n|---|---|\n| 1 | 2 |' }]);
  });
  it('keeps the lines of a paragraph, and a dash inside one is not a list', () => {
    expect(blocks('first\n- not a list\nsecond')).toEqual([{ kind: 'p', inlines: [{ kind: 'text', text: 'first\n- not a list\nsecond' }] }]);
    expect(blocks('')).toEqual([]);
  });
});
