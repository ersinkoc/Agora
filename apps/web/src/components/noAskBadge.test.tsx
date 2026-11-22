// @vitest-environment jsdom
// The acts-without-asking sign is one bolt and no words, so its tooltip is the only place a sighted person reads what it means: it must open
// on hover, on keyboard focus (its own, or its row's when it sits inside one) and on a tap, close on Esc, and be drawn over the page so a
// clipping row cannot cut it. scripts/e2e-helper-rows.mjs checks the same in Chromium, where it is measured.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import { AgentTile } from './AgentTile';
import { NoAskBadge } from './NoAskBadge';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const tip = () => document.querySelector<HTMLElement>('[data-no-ask-tip]');
const FULL = 'Runs without asking: Started so that it acts without asking you first';

describe('the acts-without-asking sign', () => {
  it('opens its tooltip on hover and closes it when the pointer leaves', () => {
    act(() => root.render(<NoAskBadge agent={{ skipsPermissions: true }} />));
    const badge = host.querySelector<HTMLElement>('[data-no-ask]')!;
    expect(tip()).toBeNull();
    act(() => { badge.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
    expect(tip()?.textContent).toBe(FULL);
    expect(tip()?.getAttribute('aria-hidden')).toBe('true'); // the sign's name already says it
    expect(tip()?.parentElement).toBe(document.body); // drawn over the page, outside any clipping row
    vi.useFakeTimers();
    try {
      act(() => { badge.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
      expect(tip()).not.toBeNull(); // a moment, so the pointer can reach the tooltip
      act(() => { vi.advanceTimersByTime(400); });
      expect(tip()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('opens on keyboard focus (and so on a tap) and closes on Esc and on blur', () => {
    act(() => root.render(<NoAskBadge agent={{ skipsPermissions: true }} />));
    const badge = host.querySelector<HTMLElement>('[data-no-ask]')!;
    expect(badge.tabIndex).toBe(0);
    act(() => badge.focus());
    expect(tip()?.textContent).toBe(FULL);
    act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    expect(tip()).toBeNull();
    act(() => { badge.blur(); badge.focus(); });
    expect(tip()).not.toBeNull();
    act(() => badge.blur());
    expect(tip()).toBeNull();
  });

  it('inside a row it is not a stop of its own; the row\'s keyboard focus opens its tooltip', () => {
    act(() => root.render(<button type="button" data-row>claude · plum<NoAskBadge agent={{ skipsPermissions: true }} nested /></button>));
    const row = host.querySelector<HTMLButtonElement>('[data-row]')!;
    expect(host.querySelector('[data-no-ask]')!.hasAttribute('tabindex')).toBe(false);
    act(() => row.focus());
    expect(tip()?.textContent).toBe(FULL);
    act(() => row.blur());
    expect(tip()).toBeNull();
  });

  it('a focus inside a larger focusable box that holds the sign is not the sign\'s control', () => {
    act(() => root.render(<div tabIndex={-1}><NoAskBadge agent={{ skipsPermissions: true }} nested /><button type="button" data-other>Give</button></div>));
    act(() => host.querySelector<HTMLButtonElement>('[data-other]')!.focus());
    expect(tip()).toBeNull();
  });

  // Codex review of #1004: a tap on the sign in an Agents-list row opened the row and showed no tooltip.
  it('a tap on a sign inside a row shows its meaning and does not press the row; a tap elsewhere on the row still does', () => {
    const press = vi.fn();
    act(() => root.render(<button type="button" data-row onClick={press}><span data-name>claude · plum</span><NoAskBadge agent={{ skipsPermissions: true }} nested /></button>));
    const sign = host.querySelector<HTMLElement>('[data-no-ask]')!;
    const tapOn = (el: Element) => act(() => {
      el.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }));
      el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
    });
    tapOn(sign);
    expect(press).not.toHaveBeenCalled();
    expect(tip()?.textContent).toBe(FULL);
    act(() => { document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' })); });
    expect(tip()).toBeNull(); // a tap anywhere else closes it
    tapOn(host.querySelector('[data-name]')!);
    expect(press).toHaveBeenCalledTimes(1);
    // A mouse click on the sign goes to the row as before (hovering already shows the meaning).
    act(() => { sign.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'mouse' })); sign.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(press).toHaveBeenCalledTimes(2);
  });

  it('stays open while the pointer moves from the sign onto the tooltip and rests there (WCAG 1.4.13)', () => {
    vi.useFakeTimers();
    try {
      act(() => root.render(<button type="button" onClick={() => {}}><NoAskBadge agent={{ skipsPermissions: true }} nested /></button>));
      const sign = host.querySelector<HTMLElement>('[data-no-ask]')!;
      act(() => { sign.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
      const bubble = tip()!;
      expect(getComputedStyle(bubble).pointerEvents).not.toBe('none');
      act(() => { sign.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body })); });
      act(() => { bubble.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: document.body })); });
      act(() => { vi.advanceTimersByTime(1000); });
      expect(tip()).not.toBeNull();
      act(() => { tip()!.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body })); });
      act(() => { vi.advanceTimersByTime(1000); });
      expect(tip()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  // Codex re-review of #1004: a row focused by keyboard, then its sign hovered and left, lost the tooltip after the timer.
  it('stays open after the pointer leaves while its row still has keyboard focus, and closes when the row loses it', () => {
    vi.useFakeTimers();
    try {
      act(() => root.render(<button type="button" data-row>claude · plum<NoAskBadge agent={{ skipsPermissions: true }} nested /></button>));
      const row = host.querySelector<HTMLButtonElement>('[data-row]')!;
      const sign = host.querySelector<HTMLElement>('[data-no-ask]')!;
      act(() => row.focus());
      expect(tip()).not.toBeNull();
      act(() => { sign.dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
      act(() => { sign.dispatchEvent(new MouseEvent('mouseout', { bubbles: true, relatedTarget: document.body })); });
      act(() => { vi.advanceTimersByTime(1000); });
      expect(tip()).not.toBeNull();
      act(() => { document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
      expect(tip()).toBeNull(); // still dismissable
      act(() => { row.blur(); row.focus(); });
      expect(tip()).not.toBeNull();
      act(() => row.blur());
      expect(tip()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('a tile with its own label says it in that label, since the label hides the sign\'s name', () => {
    const agent = { id: 'n/a', name: 'claude · plum', accent: '#fff', machine: 'm', workspace: 'w', state: 'working', preview: '' } as const;
    for (const context of [undefined, { group: 'machine' as const }]) {
      expect(renderToStaticMarkup(<AgentTile agent={{ ...agent, skipsPermissions: true }} context={context} />)).toMatch(/<button[^>]*aria-label="[^"]*claude · plum, Working, Runs without asking/);
      expect(renderToStaticMarkup(<AgentTile agent={agent} context={context} />)).not.toContain('Runs without asking');
    }
  });

  it('says which mode makes it act so, when the window knows it', () => {
    act(() => root.render(<NoAskBadge agent={{ skipsPermissions: true }} session cause="Mode: YOLO" />));
    act(() => host.querySelector<HTMLElement>('[data-no-ask]')!.focus());
    expect(tip()?.textContent).toBe('Runs without asking: Acts without asking you first, until it ends or is switched back. Because of: Mode: YOLO');
  });
});
