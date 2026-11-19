// @vitest-environment jsdom
import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import { demoAgents } from '../fixtures';
import type { AgentTileModel } from '../model';
import type { MachineModel } from '../net/view';
import { Explorer } from './Explorer';
import { Inspector } from './Inspector';
import type { RailTab } from './rail';
import { railLabel } from './rail';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const needsYou = demoAgents.filter((a) => a.state === 'needs-you');
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })); });

function Harness({ onTab }: { onTab: (t: RailTab, remember?: boolean) => void }) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<RailTab>('changes');
  return <Inspector attention={needsYou} approvals={[]} changed={[]} boards={[]} messages={[]} projects={[]} onOpen={() => {}} onResolve={() => {}}
    open={open} onOpenChange={setOpen} tab={tab} onTab={(x, r) => { setTab(x); onTab(x, r); }} />;
}

describe('the rail as a drawer', () => {
  it('moves focus into the panel when opened, and Esc closes it and returns focus to its icon', () => {
    act(() => root.render(<Harness onTab={() => {}} />));
    const icon = host.querySelector<HTMLElement>('#rail-changes') as HTMLElement;
    act(() => icon.click());
    const panel = host.querySelector<HTMLElement>('#inspector-panel');
    expect(panel).not.toBeNull();
    expect(document.activeElement).toBe(panel);
    key(panel as HTMLElement, 'Escape');
    expect(host.querySelector('#inspector-panel')).toBeNull();
    expect(document.activeElement).toBe(icon);
  });
  it('Esc on the rail icon itself closes the panel and keeps focus on the icon', () => {
    act(() => root.render(<Harness onTab={() => {}} />));
    const icon = host.querySelector<HTMLElement>('#rail-changes') as HTMLElement;
    act(() => icon.click());
    icon.focus();
    key(icon, 'Escape');
    expect(host.querySelector('#inspector-panel')).toBeNull();
    expect(document.activeElement).toBe(icon);
  });
  it('the close button and pressing the open icon again also return focus to the icon', () => {
    act(() => root.render(<Harness onTab={() => {}} />));
    const icon = host.querySelector<HTMLElement>('#rail-changes') as HTMLElement;
    act(() => icon.click());
    act(() => (host.querySelector('#inspector-panel button') as HTMLElement).click());
    expect(host.querySelector('#inspector-panel')).toBeNull();
    expect(document.activeElement).toBe(icon);
    act(() => icon.click());
    act(() => icon.click());
    expect(host.querySelector('#inspector-panel')).toBeNull();
    expect(document.activeElement).toBe(icon);
  });
});

describe('the rail icons', () => {
  it('carry their count in the accessible name', () => {
    act(() => root.render(<Harness onTab={() => {}} />));
    expect(host.querySelector('#rail-needs')?.getAttribute('aria-label')).toBe('Needs you, 1');
    expect(host.querySelector('#rail-changes')?.getAttribute('aria-label')).toBe('Changes'); // no count, no suffix
    expect(railLabel('Messages', 0)).toBe('Messages');
  });
  it('are moved to Needs you without being remembered when something new needs you', () => {
    const calls: [RailTab, boolean | undefined][] = [];
    const onTab = (t: RailTab, r?: boolean) => { calls.push([t, r]); };
    const first = (list: readonly AgentTileModel[]) => <Inspector attention={list} approvals={[]} changed={[]} boards={[]} messages={[]} projects={[]} onOpen={() => {}} onResolve={() => {}} open onOpenChange={() => {}} tab="changes" onTab={onTab} />;
    act(() => root.render(first([])));
    act(() => root.render(first(needsYou)));
    expect(calls).toEqual([['needs', false]]);
  });
});

describe('the agent list when grouped by something other than machine', () => {
  const sky = demoAgents.find((a) => a.id === 'a20') as AgentTileModel;
  const machines: MachineModel[] = [];
  for (const by of ['project', 'state', 'kind'] as const) {
    it(`still lists an agent its machine's restart interrupted, with Resume, grouped by ${by}`, () => {
      const resume = vi.fn();
      act(() => root.render(<Explorer by={by} onBy={() => {}} spread={new Map()} fleet={demoAgents} machines={machines} open={[]} projects={[]} project="" onProject={() => {}} onToggle={() => {}} onCollapse={() => {}} onDismiss={() => {}} onResume={resume} />));
      expect(host.textContent).toContain(sky.name);
      const button = [...host.querySelectorAll('button')].find((b) => b.textContent === 'Resume') as HTMLElement;
      expect(button).toBeTruthy();
      act(() => button.click());
      expect(resume).toHaveBeenCalledWith(expect.objectContaining({ id: 'a20' }), 'resume');
    });
  }
});
