import { describe, expect, it } from 'vitest';
import { languageOf, markRanges, splitHighlighted } from './highlight';

describe('the language of a file', () => {
  it('comes from a known name or the extension', () => {
    expect(languageOf('src/App.tsx')).toBe('typescript');
    expect(languageOf('crates/x/src/lib.rs')).toBe('rust');
    expect(languageOf('Dockerfile')).toBe('dockerfile');
    expect(languageOf('locales/en/web.ftl')).toBe('ini');
    expect(languageOf('README')).toBeUndefined();
    expect(languageOf('.env')).toBeUndefined();
  });
});

describe('highlighted lines', () => {
  it('closes and reopens spans across line breaks', () => {
    const html = 'a <span class="hljs-comment">/* one\ntwo */</span> b\n<span class="hljs-string">&quot;x&quot;</span>\n';
    expect(splitHighlighted(html)).toEqual([
      'a <span class="hljs-comment">/* one</span>',
      '<span class="hljs-comment">two */</span> b',
      '<span class="hljs-string">&quot;x&quot;</span>',
    ]);
  });

  it('keeps nested spans and drops carriage returns at line ends', () => {
    const html = '<span class="a">x<span class="b">y\r\nz</span></span>';
    expect(splitHighlighted(html)).toEqual(['<span class="a">x<span class="b">y</span></span>', '<span class="a"><span class="b">z</span></span>']);
  });

  it('gives one line for a file without a line break', () => expect(splitHighlighted('plain')).toEqual(['plain']));
});

describe('a mark over highlighted text', () => {
  it('counts an escaped character as one and splits at tags', () => {
    const html = 'if <span class="k">a</span> &amp;&amp; b';
    expect(markRanges(html, [{ start: 3, end: 6, cls: 'm' }])).toBe('if <span class="k"><mark class="m">a</mark></span><mark class="m"> &amp;</mark>&amp; b');
  });

  it('marks several ranges and leaves the text unchanged', () => {
    const html = 'abc<span class="x">def</span>';
    const marked = markRanges(html, [{ start: 0, end: 1, cls: 'm' }, { start: 4, end: 6, cls: 'n' }]);
    expect(marked).toBe('<mark class="m">a</mark>bc<span class="x">d<mark class="n">ef</mark></span>');
    expect(marked.replace(/<[^>]+>/g, '')).toBe('abcdef');
    expect(markRanges(html, [])).toBe(html);
  });
});
