import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import { LiveSummary, lastScreenLine } from './LiveSummary';

const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({ id: 'n/a', name: 'claude · amber', accent: '#fff', machine: 'laptop', workspace: '/w', state: 'working', preview: '', ...over });

describe('live summary of a terminal agent', () => {
  beforeEach(() => setLanguage('en'));
  it('takes the last line that says something, cut to fit', () => {
    expect(lastScreenLine('one\n  two  \n\n   \n')).toBe('two');
    expect(lastScreenLine('')).toBe('');
    expect(lastScreenLine('x'.repeat(300), 10)).toBe('xxxxxxxxx…');
  });
  it('shows state, last line, time, tokens and a one-click way to the changes, and no money', () => {
    const html = renderToStaticMarkup(<LiveSummary now={130_000} onChanges={() => {}} agent={agent({ preview: 'Running tests…', startedMs: 10_000, tokens: 12_000, filesChanged: 3 })} />);
    expect(html).toContain('Running tests…');
    expect(html).toContain('2 min 0 s');
    expect(html).toContain('3 files');
    expect(html).toContain('<button');
    expect(html).not.toMatch(/\$|USD/);
  });
  it('says nothing when the window knows nothing', () => {
    expect(renderToStaticMarkup(<LiveSummary now={1} agent={agent()} />)).toBe('');
  });
  it('makes hidden characters in the live terminal summary visible', () => {
    const html = renderToStaticMarkup(<LiveSummary now={1} agent={agent({ preview: 'safe\nrun\u3164ning\u202e\ue000' })} />);
    expect(html).toContain('run\\u{3164}ning\\u{202e}\\u{e000}');
    expect(html).not.toContain('run\u3164ning');
  });
  it('verifies live summary text across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u200b', '\\u{200b}'], ['\u0001', '\\u{1}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const html = renderToStaticMarkup(<LiveSummary now={1} agent={agent({ preview: `wor${char}king` })} />);
      const shown = `wor${escaped}king`;
      expect(html).toContain(shown);
      expect(html.split(shown)).toHaveLength(3);
    }
  });
});
