import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import type { ConfigOptionModel, SessionModel } from '../model';
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
  it('makes agent-supplied mode and model labels visible', () => {
    const unsafe: SessionModel = { ...session, modes: [opt('m\u3164ode', 'Bu\u2800ild')], models: [opt('m1', 'Mo\u034fdel')], currentMode: 'm\u3164ode' };
    const html = renderToStaticMarkup(<SessionBar agentKey="n/a" session={unsafe} />);
    expect([...html].some((c) => ['\u3164', '\u2800', '\u034f'].includes(c))).toBe(false);
    expect(html).toContain('Bu\\u{2800}ild');
    expect(html).toContain('Mo\\u{34f}del');
  });
  it('verifies option visibility for fallback ids and multilingual names', () => {
    const unsafe: SessionModel = { ...session, modes: [opt('ça\u202elış', '')], models: [opt('m', 'Grü\u3164ße 😀')], currentMode: 'ça\u202elış', currentModel: 'm' };
    const html = renderToStaticMarkup(<SessionBar agentKey="n/a" session={unsafe} />);
    expect(html).toContain('ça\\u{202e}lış');
    expect(html).toContain('Grü\\u{3164}ße 😀');
    expect([...html].some((c) => ['\u202e', '\u3164'].includes(c))).toBe(false);
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
  it('shows the acts-without-asking sign while the current mode is one that skips, naming the mode, and not when it is a safe one', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, currentMode: 'yolo' }} />);
    expect(html).toContain('data-no-ask="session"');
    expect(html).toContain('aria-label="Runs without asking: Acts without asking you first, until it ends or is switched back. Because of: Mode: ');
    expect(html).not.toContain('Acts freely');
    expect(renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={session} />)).not.toContain('data-no-ask');
  });
  it('does not repeat the badge of an agent that was started that way: one sign, the started one', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, currentMode: 'yolo' }} launchedFree />);
    expect(html.match(/data-no-ask=/g)).toHaveLength(1);
    expect(html).toContain('data-no-ask="started"');
  });
});

// Redesign PR 12: the sign is a red-tinted tag (still the bolt alone, no words) that stands right beside Mode, the control that causes it.
describe('the acts-freely tag', () => {
  const at = (html: string, needle: string) => { const i = html.indexOf(needle); expect(i, needle).toBeGreaterThanOrEqual(0); return i; };
  const one = (html: string) => { expect(html.match(/data-no-ask=/g) ?? []).toHaveLength(1); expect(html).toContain('data-look="tag"'); expect(html).not.toContain('Acts freely'); };
  const cfg = (current: string): ConfigOptionModel[] => [
    { id: 'mode', name: 'Mode', description: '', category: 'mode', type: 'select', current, skipsPermissions: false, choices: [opt('build', 'Build'), opt('yolo', 'Bypass', true)] },
    { id: 'model', name: 'Model', description: '', category: 'model', type: 'select', current: 'm1', skipsPermissions: false, choices: [opt('m1', 'Model Large')] },
  ];
  it('follows the Mode menu, before Model, when the agent was switched to a mode that skips', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, currentMode: 'yolo' }} />);
    one(html);
    expect(at(html, 'aria-label="Mode: ')).toBeLessThan(at(html, 'data-no-ask="session"'));
    expect(at(html, 'data-no-ask="session"')).toBeLessThan(at(html, 'aria-label="Model: '));
  });
  it('follows the Mode menu for an agent that was started that way, also while its mode is a safe one', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={session} launchedFree />);
    one(html);
    expect(at(html, 'aria-label="Mode: Build"')).toBeLessThan(at(html, 'data-no-ask="started"'));
    expect(at(html, 'data-no-ask="started"')).toBeLessThan(at(html, 'aria-label="Model: '));
  });
  it("follows the agent's own Mode setting when that replaces the mode menu", () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, modes: [], models: [], config: cfg('yolo') }} setConfig={async () => true} />);
    one(html);
    expect(at(html, 'aria-label="Mode: ')).toBeLessThan(at(html, 'data-no-ask="session"'));
    expect(at(html, 'data-no-ask="session"')).toBeLessThan(at(html, 'aria-label="Model: '));
  });
  it('leads the row when there is no Mode control at all', () => {
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, modes: [], currentMode: '' }} launchedFree />);
    one(html);
    expect(html).not.toContain('aria-label="Mode: ');
    expect(at(html, 'data-no-ask="started"')).toBeLessThan(at(html, 'aria-label="Model: '));
  });
});

describe('agent-owned config labels', () => {
  it('makes config choice labels visible without changing their ids', () => {
    const config: ConfigOptionModel = { id: 'effort', name: 'Effort', description: '', category: 'thought_level', type: 'select', current: 'raw\u034fvalue', skipsPermissions: false, choices: [opt('raw\u034fvalue', 'Hi\uffa0gh')] };
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, modes: [], models: [], config: [config] }} />);
    expect([...html].some((c) => ['\u034f', '\uffa0'].includes(c))).toBe(false);
    expect(html).toContain('Hi\\u{ffa0}gh');
  });
  it('verifies config visibility for fallback values and multilingual choices', () => {
    const a: ConfigOptionModel = { id: 'effort', name: '', description: '', category: 'thought_level', type: 'select', current: 'a', skipsPermissions: false, choices: [opt('a', 'ça\u202elış')] };
    const b: ConfigOptionModel = { ...a, id: 'model', category: 'model', current: 'v', choices: [opt('v', 'Grü\u3164ße 😀')] };
    const html = renderToStaticMarkup(<SessionBar part="controls" agentKey="n/a" session={{ ...session, modes: [], models: [], config: [a, b] }} />);
    expect(html).toContain('ça\\u{202e}lış');
    expect(html).toContain('Grü\\u{3164}ße 😀');
    expect([...html].some((c) => ['\u202e', '\u3164'].includes(c))).toBe(false);
  });
});

describe('the session state', () => {
  it('is the kit status pill: tint, shape and word, never colour alone', () => {
    const html = renderToStaticMarkup(<SessionBar part="status" agentKey="n/a" session={session} state="needs-you" tokens={215} />);
    expect(html).toMatch(/data-state-pill="needs-you"[^>]*><span class="kit-pill kit-tone-needs"><svg class="state__mark"/);
    expect(html).toContain('Needs you');
    expect(html.match(/215 tokens used/g)).toHaveLength(1);
  });
});
