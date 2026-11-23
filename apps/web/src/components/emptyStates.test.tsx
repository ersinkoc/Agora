import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import { FleetEmpty } from './FleetConsole';
import { Details } from './Inspector';

const none = { fleet: [], attention: [], approvals: [], changed: [], boards: [], messages: [], onOpen: () => {}, onResolve: () => {} };

describe('empty and loading panels', () => {
  for (const lang of ['en', 'tr'] as const) {
    describe(lang, () => {
      beforeEach(() => setLanguage(lang));
      it('Team chat, Changes, Shared tasks and Usage say something friendly in the one kit empty state, not a bare line', () => {
        for (const tab of ['changes', 'messages', 'board', 'usage'] as const) {
          const html = renderToStaticMarkup(<Details tab={tab} {...none} />);
          expect(html).toContain('class="kit-empty kit-tone-accent"');
          expect(html).toContain('kit-empty__title');
          expect(html).toContain('kit-empty__hint');
          expect(html).toContain('role="status"');
          expect(html).not.toMatch(/empty-(changes|messages|board|usage)-(title|hint)/); // a missing message would show its id
          expect(html).not.toMatch(/class="muted/); // no plain grey line instead
        }
      });
      it('the fleet draws skeleton tiles while it connects, hidden from assistive tech, with the status said once', () => {
        const html = renderToStaticMarkup(<FleetEmpty kind="connecting" machines={[]} />);
        expect(html).toContain('aria-busy="true"');
        expect((html.match(/class="skeleton /g) ?? []).length).toBeGreaterThanOrEqual(6);
        expect(html).toMatch(/aria-hidden="true"[^>]*class="fleet-grid|class="fleet-grid[^>]*aria-hidden="true"/);
      });
    });
  }
});
