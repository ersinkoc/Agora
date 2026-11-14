import { useEffect, useState } from 'react';
import { languageOf, splitHighlighted } from '../net/highlight';

type Highlighter = typeof import('./hl');

// The highlighter is a chunk of its own, loaded the first time a file is shown and kept for the session.
let loading: Promise<Highlighter | undefined> | undefined;
const loaded = (): Promise<Highlighter | undefined> => (loading ??= import('./hl').catch(() => undefined));

/** Larger files stay plain: highlighting them would hold the page for longer than reading them takes. */
export const MAX_HIGHLIGHT_CHARS = 256 * 1024;

/** A whole file, highlighted, as one HTML string per line; undefined while loading, for an unknown language or a large file (the
 *  caller shows plain text then). Done after the plain text is on screen, never in the way of showing it. */
export function useHighlightedFile(path: string | undefined, lines: string[] | undefined): string[] | undefined {
  const [html, setHtml] = useState<string[] | undefined>();
  useEffect(() => {
    setHtml(undefined);
    const language = path ? languageOf(path) : undefined;
    if (!language || !lines || lines.reduce((n, l) => n + l.length + 1, 0) > MAX_HIGHLIGHT_CHARS) return;
    let live = true;
    const timer = setTimeout(() => {
      loaded().then((hl) => {
        if (!live || !hl) return;
        const out = hl.highlight(lines.join('\n'), language);
        const split = out === undefined ? undefined : splitHighlighted(out);
        // Only if it lines up: the same number of lines as the text (otherwise plain is the safe answer).
        if (live && split && split.length === lines.length) setHtml(split);
      });
    }, 0);
    return () => { live = false; clearTimeout(timer); };
  }, [path, lines]);
  return html;
}

/** Each of `texts` highlighted on its own (lines of a diff, whose neighbours may be far apart), keyed by position; empty until the
 *  highlighter is loaded, or for an unknown language. */
export function useHighlightedLines(path: string | undefined, texts: string[]): (string | undefined)[] {
  const [html, setHtml] = useState<(string | undefined)[]>([]);
  const key = texts.join('\n');
  useEffect(() => {
    setHtml([]);
    const language = path ? languageOf(path) : undefined;
    if (!language || texts.length === 0 || key.length > MAX_HIGHLIGHT_CHARS) return;
    let live = true;
    loaded().then((hl) => {
      if (!live || !hl) return;
      const out = texts.map((t) => hl.highlight(t, language));
      if (live) setHtml(out);
    });
    return () => { live = false; };
  }, [path, key]); // eslint-disable-line react-hooks/exhaustive-deps
  return html;
}
