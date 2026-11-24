import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { Details } from './Inspector';
import { pathParts } from '../net/paneTools';

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
        const text = html.replace(/<[^>]*>/g, '');
        expect(text.split('/home/me/code/login-fix').length - 1).toBe(1);
      });
      it('lets a long row shrink to the panel, so the count and the pane button stay in view', () => {
        const html = renderToStaticMarkup(<Details tab="changes" {...none} changed={[shared]} onReview={() => {}} onPane={() => {}} />);
        // A grid column is as wide as its widest row unless it may shrink: `grid-cols-1` is `minmax(0, 1fr)`.
        expect(html).toMatch(/<ul class="[^"]*\bgrid-cols-1\b/);
      });
      it('cuts a long path in the middle so the folder name stays, and gives the full path and the machine in its tooltip', () => {
        const path = '/home/someone/Code/clients/acme/services/login-fix';
        const html = renderToStaticMarkup(<Details tab="changes" {...none} changed={[{ ...shared, workspace: path }]} onReview={() => {}} onPane={() => {}} />);
        expect(html).toContain(`title="${path} · blackview"`);
        // What leads to the folder gives way (it ends in an ellipsis); the folder's name never shrinks. The text is the whole path.
        expect(html).toContain('<span dir="ltr" class="flex min-w-0"><span class="min-w-0 truncate">/home/someone/Code/clients/acme/services</span><span class="max-w-full flex-none truncate">/login-fix</span>');
        // The machine leads the second line, so it is in view however narrow the panel.
        expect(html).toMatch(/<span class="block truncate text-xs text-muted">blackview · /);
      });
    });
  }
});

describe('a path in two parts', () => {
  it('splits off the folder name with its separator, on either kind of machine', () => {
    expect(pathParts('/home/me/code/login-fix')).toEqual({ dir: '/home/me/code', name: '/login-fix' });
    expect(pathParts('C:\\Users\\me\\agora')).toEqual({ dir: 'C:\\Users\\me', name: '\\agora' });
    expect(pathParts('/srv')).toEqual({ dir: '', name: '/srv' });
    expect(pathParts('proj')).toEqual({ dir: '', name: 'proj' });
  });
});
