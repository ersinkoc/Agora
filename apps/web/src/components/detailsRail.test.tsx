// @vitest-environment jsdom
// The details panel's sections: the vertical menu at the window's end, and the title that replaces the tabs
// that used to lie across the panel's top.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage, t } from '../i18n';
import { DetailsRail } from './DetailsRail';
import { detailsSections, type DetailsTab } from './detailsTabs';
import { Inspector } from './Inspector';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

// What needs you, then changes and messages: what the window has to say.
const sections = () => detailsSections({ attention: [], approvals: [], changed: [], boards: [], messages: [] });

describe('the details sections', () => {
  it('are one list: what needs you first, then the rest', () => {
    const list = sections();
    expect(list[0]?.id).toBe('needs');
    expect(list.map((x) => x.id)).toEqual(['needs', 'changes', 'messages']);
    expect(list[0]?.label).toBe(t('attention-title'));
  });

  it('leave out what the window has none of', () => {
    const spent = { perAgent: new Map<string, number>(), perNode: new Map<string, number>(), totalMicroUsd: 0, pricesAtMs: 0 };
    const list = detailsSections({ attention: [], approvals: [], changed: [], boards: [], messages: [], usage: spent });
    expect(list.map((x) => x.id)).toEqual(['needs', 'changes', 'messages']);
  });
});

describe('the menu at the window\'s end', () => {
  it('stands on its own, so the sections are there even with the panel closed', () => {
    const html = renderToStaticMarkup(<DetailsRail sections={sections()} active="needs" open={false} onSelect={() => undefined} />);
    expect(html).toContain('aria-orientation="vertical"');
    expect(html.match(/role="tab"/g)).toHaveLength(3);
    // A closed panel has nothing to point at, so the tabs do not claim a panel that is not there.
    expect(html).not.toContain('aria-controls');
  });

  it('names every section, so an icon is never only a picture', () => {
    const html = renderToStaticMarkup(<DetailsRail sections={sections()} active="needs" open onSelect={() => undefined} />);
    for (const label of [t('attention-title'), t('ide-tab-changes'), t('ide-tab-messages')]) expect(html).toContain(`title="${label}"`);
    expect(html).toContain('aria-controls="details-panel"');
  });

  it('asks for the section that was pressed, so any of them opens the panel', () => {
    const asked: DetailsTab[] = [];
    act(() => root.render(<DetailsRail sections={sections()} active="needs" open={false} onSelect={(id) => asked.push(id)} />));
    const tab = (label: string) => [...host.querySelectorAll('[role="tab"]')].find((x) => x.getAttribute('title') === label) as HTMLButtonElement;
    expect(tab(t('ide-tab-changes'))).toBeTruthy();
    act(() => { tab(t('ide-tab-changes')).click(); });
    expect(asked).toEqual(['changes']);
  });

  it('marks the section that is open', () => {
    act(() => root.render(<DetailsRail sections={sections()} active="changes" open onSelect={() => undefined} />));
    const selected = [...host.querySelectorAll('[role="tab"]')].filter((x) => x.getAttribute('aria-selected') === 'true');
    expect(selected).toHaveLength(1);
    expect(selected[0]?.getAttribute('title')).toBe(t('ide-tab-changes'));
  });
});

describe('the details panel', () => {
  const panel = (tab: DetailsTab) => renderToStaticMarkup(
    <Inspector sections={sections()} tab={tab} attention={[]} approvals={[]} changed={[]} boards={[]} messages={[]} projects={[]}
      onOpen={() => undefined} onResolve={() => undefined} onCollapse={() => undefined} />);

  it('has a title on top, not a row of tabs', () => {
    const html = panel('changes');
    expect(html).toMatch(/<h2[^>]*>Changes<\/h2>/);
    // The tabs moved to the menu at the window's end; the panel itself keeps no strip of them.
    expect(html).not.toContain('role="tablist"');
    expect(html).toContain('role="tabpanel"');
  });

  it('titles itself with the section that is open', () => {
    expect(panel('needs')).toMatch(/<h2[^>]*>Needs you<\/h2>/);
    expect(panel('messages')).toMatch(/<h2[^>]*>Messages<\/h2>/);
  });
});
