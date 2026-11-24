import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { ChatLine } from '../net/chat';
import { ToolSteps } from './ChatEntry';
import { CodeView } from './FilesView';
import { ProjectHint } from './ProjectMark';

// The places that were amber without meaning "needs you" (ADR-0102), drawn: a failed tool group and a deleted line are the danger red,
// a hint is neutral, and only a group whose agent waits on the person stays amber. The source scan (design/amberOnlyNeedsYou.test.ts)
// keeps new amber out; these pin what the moved places became.
beforeEach(() => setLanguage('en'));

const call: ChatLine = { id: 1, atMs: 0, role: 'agent', kind: 'tool', text: '{"tool":"Read","arguments":{"file_path":"a.rs"}}', path: '', callId: 'c1' };

describe('amber only means needs you', () => {
  it('a tool group that failed is red, one that waits on the person stays amber', () => {
    for (const flag of ['failed', 'call-failed'] as const) {
      const html = renderToStaticMarkup(<ToolSteps lines={[call]} flag={flag} />);
      expect(html, flag).toContain('border-danger/70');
      expect(html, flag).toContain('text-danger');
      expect(html, flag).not.toContain('needs-you');
    }
    const asks = renderToStaticMarkup(<ToolSteps lines={[call]} flag="needs-you" />);
    expect(asks).toContain('border-needs-you/70');
    expect(asks).toContain('text-needs-you');
    expect(asks).not.toContain('danger');
  });

  it("a deleted line's mark in the file browser is red, never amber", () => {
    const html = renderToStaticMarkup(<CodeView label="a.rs" lines={['a', 'b']} marks={new Map([[1, 'add'], [2, 'del']])} matches={[]} />);
    expect(html).toContain('bg-danger');
    expect(html).toContain('bg-finished');
    expect(html).not.toContain('needs-you');
  });

  it('the "changed on more than one machine" hint is a neutral note', () => {
    const html = renderToStaticMarkup(<ProjectHint spread={{ machines: ['a', 'b'], agents: 2, changing: 2 }} />);
    expect(html).toContain('text-muted');
    expect(html).not.toContain('needs-you');
  });
});
