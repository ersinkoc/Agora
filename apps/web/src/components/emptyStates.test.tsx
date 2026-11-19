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
      it('Changes and Messages say something friendly, with a hint, not a bare line', () => {
        for (const tab of ['changes', 'messages'] as const) {
          const html = renderToStaticMarkup(<Details tab={tab} {...none} />);
          expect(html).toContain('empty-state__title');
          expect(html).toContain('empty-state__hint');
          expect(html).not.toMatch(/empty-(changes|messages)-(title|hint)/); // a missing message would show its id
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
