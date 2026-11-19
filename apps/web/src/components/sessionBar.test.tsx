import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { SessionModel } from '../model';
import { SessionBar } from './SessionBar';

beforeEach(() => setLanguage('en'));

const opt = (id: string, name: string, skips = false) => ({ id, name, description: '', skipsPermissions: skips });
const session: SessionModel = { modes: [opt('build', 'Build'), opt('yolo', 'Bypass', true)], models: [opt('m1', 'Model Large')], currentMode: 'build', currentModel: 'm1', contextTokens: 50_000, contextWindow: 200_000, costMicros: 23_000, currency: 'USD' };

describe('the agent header', () => {
  it('is one row of menus (not native selects) for what the agent reported, with context, elapsed time and cost', () => {
    const html = renderToStaticMarkup(<SessionBar agentKey="n/a" session={session} startedMs={Date.now() - 125_000} choose={async () => true} />);
    expect(html).not.toContain('<select');
    expect(html).toContain('aria-label="Model: Model Large"');
    expect(html).toContain('aria-label="Mode: Build"');
    expect(html).toContain('aria-haspopup="menu"');
    expect(html).toContain('Context 25%');
    expect(html).toContain('2 min');
    expect(html).toContain('reported by the agent');
  });
  it('never invents a reasoning effort picker: the agent reports modes and models only', () => {
    expect(renderToStaticMarkup(<SessionBar agentKey="n/a" session={session} />).toLowerCase()).not.toContain('effort');
  });
  it('hides a picker for a list the agent did not report, and elapsed time when the start was not seen', () => {
    const html = renderToStaticMarkup(<SessionBar agentKey="n/a" session={{ ...session, models: [], currentModel: '' }} />);
    expect(html).not.toContain('Model:');
    expect(html).toContain('Mode: Build');
    expect(html).not.toContain('How long it has been running');
  });
});
