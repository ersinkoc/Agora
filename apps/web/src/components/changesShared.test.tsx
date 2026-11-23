import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { Details } from './Inspector';

const none = { fleet: [], attention: [], approvals: [], boards: [], messages: [], onOpen: () => {}, onResolve: () => {} };
const shared: AgentTileModel = { id: 'n/agent-2', name: 'shell · amber', accent: '#fff', machine: 'blackview', workspace: '/home/me/code/login-fix', project: 'login-fix', state: 'idle', preview: '', shared: true, filesChanged: 3, workspaceRef: { node: 'n', id: 'directory:/home/me/code/login-fix', directory: true } };

// #1017: a folder two agents share is one row of the right panel's Changes, about the folder (ADR-0052), with its count in view.
describe('a shared folder in the Changes panel', () => {
  for (const lang of ['en', 'tr'] as const) {
    describe(lang, () => {
      beforeEach(() => setLanguage(lang));
      it('names the folder and says the changes are the shared folder\'s, never one agent\'s, and names the path once', () => {
        const html = renderToStaticMarkup(<Details tab="changes" {...none} changed={[shared]} onReview={() => {}} onPane={() => {}} />);
        expect(html).toContain(t('workspace-shared'));
        expect(html).toContain('blackview');
        expect(html).toContain(t('changes-files', { count: 3 }));
        expect(html).not.toContain('shell · amber');
        const text = html.replace(/<[^>]*>/g, ' ');
        expect(text.split('/home/me/code/login-fix').length - 1).toBe(1);
      });
      it('lets a long row shrink to the panel, so the count and the pane button stay in view', () => {
        const html = renderToStaticMarkup(<Details tab="changes" {...none} changed={[shared]} onReview={() => {}} onPane={() => {}} />);
        // A grid column is as wide as its widest row unless it may shrink: `grid-cols-1` is `minmax(0, 1fr)`.
        expect(html).toMatch(/<ul class="[^"]*\bgrid-cols-1\b/);
      });
    });
  }
});
