import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage } from '../i18n';
import { demoAgents } from '../fixtures';
import { Icon } from './Icon';
import { Inspector } from './Inspector';
import { badgeText, loadRailTab, railItems, resolveTab, saveRailTab } from './rail';
import { commandById } from '../keys/commands';
import { EMPTY_FILE, keymap } from '../keys/file';
import { createDispatcher } from '../keys/registry';
import { bindingLabel, parseBinding } from '../keys/strokes';

beforeEach(() => setLanguage('en'));

describe('the right panel rail', () => {
  const counts = { needs: 2, changed: 3, messages: 0, hasBoard: false, hasUsage: false };
  it('always offers Needs you, Changes and Messages; Board and Usage only when there is something', () => {
    expect(railItems(counts).map((x) => x.id)).toEqual(['needs', 'changes', 'messages']);
    expect(railItems({ ...counts, hasBoard: true, hasUsage: true }).map((x) => x.id)).toEqual(['needs', 'changes', 'board', 'messages', 'usage']);
  });
  it('carries the counts as badges and marks only Needs you as an alert', () => {
    const items = railItems(counts);
    expect(items.map((x) => x.badge)).toEqual([2, 3, 0]);
    expect(items.filter((x) => x.alert).map((x) => x.id)).toEqual(['needs']);
  });
  it('caps a long count', () => {
    expect(badgeText(7)).toBe('7');
    expect(badgeText(99)).toBe('99');
    expect(badgeText(100)).toBe('99+');
  });
  it('opens the subject at Alt+Shift+digit (its place in the rail) and ignores every other combination', () => {
    // The keys are the side panel's commands now (docs/ux/keybindings.md); the defaults are the old shortcuts, matched by the key's place.
    const ran: string[] = [];
    const d = createDispatcher({ map: () => keymap(EMPTY_FILE), mac: false, available: () => true, run: (id) => { ran.push(id); return true; } });
    const key = (o: object) => ({ key: '@', altKey: true, shiftKey: true, ctrlKey: false, metaKey: false, code: 'Digit2', ...o });
    d.handle(key({}), 'other');
    d.handle(key({ code: 'Digit0', key: ')' }), 'other');
    d.handle(key({ shiftKey: false, key: '2' }), 'other'); // Alt+digit picks a tab of the pane group
    d.handle(key({ ctrlKey: true }), 'other');
    expect(ran).toEqual(['side.place-2', 'pane.tab-2']);
    expect(bindingLabel(parseBinding(commandById('side.place-1')?.keys[0]) ?? [], false)).toBe('Alt+Shift+1');
  });
  it('falls back to Needs you when the remembered subject is gone', () => {
    expect(resolveTab('board', railItems(counts))).toBe('needs');
    expect(resolveTab('changes', railItems(counts))).toBe('changes');
    expect(resolveTab(undefined, railItems(counts))).toBe('needs');
  });
  it('remembers the subject and survives storage that throws', () => {
    const kept: Record<string, string> = {};
    saveRailTab('changes', { setItem: (k, v) => { kept[k] = v; } });
    expect(loadRailTab({ getItem: (k) => kept[k] ?? null })).toBe('changes');
    expect(loadRailTab({ getItem: () => 'nonsense' })).toBeUndefined();
    expect(() => saveRailTab('usage', { setItem: () => { throw new Error('quota'); } })).not.toThrow();
    expect(loadRailTab({ getItem: () => { throw new Error('blocked'); } })).toBeUndefined();
  });
});

describe('Inspector', () => {
  const draw = (open: boolean) => renderToStaticMarkup(<Inspector attention={demoAgents.filter((a) => a.state === 'needs-you')} approvals={[]} changed={[]} boards={[]} messages={[]} projects={[]}
    onOpen={() => {}} onResolve={() => {}} open={open} onOpenChange={() => {}} tab="needs" onTab={() => {}} />);
  it('draws the rail as a vertical tab list with names and shortcuts, with the panel only while open', () => {
    const closed = draw(false);
    expect(closed).toContain('aria-orientation="vertical"');
    expect(closed).toContain('aria-label="Needs you, 1"');
    expect(closed).toContain('aria-keyshortcuts="Alt+Shift+1"');
    expect(closed).toContain('data-tip="Needs you (1) · Alt+Shift+1"');
    expect(closed).not.toContain('id="inspector-panel"');
    expect(draw(true)).toContain('id="inspector-panel"');
  });
  it('no longer draws a horizontal tab row', () => {
    expect(draw(true)).not.toContain('overflow-x-auto');
  });
  it('shows Usage as bars, never as money', () => {
    const usage = railItems({ needs: 0, changed: 0, messages: 0, hasBoard: false, hasUsage: true }).find((x) => x.id === 'usage');
    expect(usage?.icon).toBe('gauge');
    // The icon set has no coin or dollar glyph left to pick by mistake, and no rail item uses one.
    expect(renderToStaticMarkup(<Icon name="gauge" />)).toContain('M2.5 13.5h11');
    expect(Object.keys(railItems({ needs: 1, changed: 1, messages: 1, hasBoard: true, hasUsage: true })).length).toBeGreaterThan(0);
    expect(railItems({ needs: 1, changed: 1, messages: 1, hasBoard: true, hasUsage: true }).map((x) => x.icon as string)).not.toContain('coin');
  });
  it('does not draw the open subject\'s tip over its own panel (it hid the values at the end of a row)', () => {
    const css = readFileSync('src/design/app.css', 'utf8');
    expect(css).toMatch(/\.rail__btn\[aria-selected='true'\]::after \{ @apply hidden; \}/);
  });
});
