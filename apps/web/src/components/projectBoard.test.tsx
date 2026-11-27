import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { BoardModel } from '../net/view';
import { ProjectBoard } from './ProjectBoard';

describe('project board summary text', () => {
  it('makes every machine-supplied label visible', () => {
    const board: BoardModel = {
      projectId: 'p\u034f', project: 'pro\u2800ject',
      folders: [{ nodeId: 'n\u3164', machine: 'ma\uffa0chine', path: '/w\u202e/repo' }],
      tasks: [{ id: 't', title: 'ta\u034fsk', status: 'claimed', owner: 'ow\u2800ner' }],
      leases: [{ path: '/lo\u3164ck', holder: 'ho\uffa0lder', expiresAtMs: 1 }],
    };
    const html = renderToStaticMarkup(<ProjectBoard onOpen={() => {}} board={board} />);
    const text = html.replace(/<[^>]+>/g, '');
    expect([...text].some((c) => ['\u034f', '\u2800', '\u3164', '\uffa0', '\u202e'].includes(c))).toBe(false);
    for (const shown of ['pro\\u{2800}ject', 'ma\\u{ffa0}chine', '/w\\u{202e}/repo', 'ta\\u{34f}sk', 'ow\\u{2800}ner', '/lo\\u{3164}ck', 'ho\\u{ffa0}lder']) expect(text).toContain(shown);
  });
  it('verifies multilingual display while preserving the raw project identity', () => {
    const board: BoardModel = { projectId: 'kim\u202elik', project: 'Grü\u3164ße 😀', folders: [], tasks: [], leases: [] };
    const html = renderToStaticMarkup(<ProjectBoard onOpen={() => {}} board={board} />);
    expect(html).toContain('data-project-id="kim‮lik"');
    expect(html).toContain('Grü\\u{3164}ße 😀');
    expect(html).toContain('kim\\u{202e}lik');
  });
});
