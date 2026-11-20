import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { SessionModel } from '../model';
import { SessionBar } from './SessionBar';
import { isChange } from './sessionChoice';

beforeEach(() => setLanguage('en'));

const opt = (id: string, name: string, skips = false) => ({ id, name, description: '', skipsPermissions: skips });
const session: SessionModel = { modes: [opt('build', 'Build'), opt('yolo', 'Bypass', true)], models: [opt('m1', 'Model Large')], currentMode: 'build', currentModel: 'm1', contextTokens: 50_000, contextWindow: 200_000 };

describe('the agent header', () => {
  it('is one row of menus (not native selects) for what the agent reported, with context, elapsed time and cost', () => {
    const html = renderToStaticMarkup(<SessionBar agentKey="n/a" session={session} startedMs={Date.now() - 125_000} choose={async () => true} />);
    expect(html).not.toContain('<select');
    expect(html).toContain('aria-label="Model: Model Large"');
    expect(html).toContain('aria-label="Mode: Build"');
    expect(html).toContain('aria-haspopup="menu"');
    expect(html).toContain('Context 25%');
    expect(html).toContain('2 min');
    expect(html).not.toMatch(/reported by the agent|\$|USD/); // no money in the header (it lives behind a disclosure in Usage)
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

describe('choosing what is already in use', () => {
  it('is not a change: nothing is sent and no confirmation opens', () => {
    expect(isChange('yolo', 'yolo')).toBe(false);
    expect(isChange('build', '')).toBe(false); // the "not reported" placeholder
    expect(isChange('build', 'yolo')).toBe(true);
    expect(isChange('', 'build')).toBe(true);
  });
});

describe('a mode that skips permission questions', () => {
  it('is an ordinary choice in the menu: marked, enabled, with no "not available" text and no confirmation of its own', () => {
    const html = renderToStaticMarkup(<SessionBar agentKey="n/a" session={session} choose={async () => true} />);
    expect(html).not.toContain('not available');
    expect(html).not.toContain('alertdialog');
    expect(html).not.toContain('Switch anyway');
  });
  it('says "Acts freely" while the current mode is one that skips, and not when it is a safe one', () => {
    expect(renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, currentMode: 'yolo' }} />)).toContain('Acts freely');
    expect(renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={session} />)).not.toContain('Acts freely');
  });
  it('does not repeat the badge of an agent that was started that way', () => {
    expect(renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, currentMode: 'yolo' }} launchedFree />)).not.toContain('Acts freely');
  });
});
