// @vitest-environment jsdom
// The acts-without-asking sign is one bolt and no words, so its tooltip is the only place a sighted person reads what it means: it must open
// on hover, on keyboard focus (its own, or its row's when it sits inside one) and on a tap, close on Esc, and be drawn over the page so a
// clipping row cannot cut it. scripts/e2e-helper-rows.mjs checks the same in Chromium, where it is measured.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
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
    act(() => { badge.dispatchEvent(new MouseEvent('mouseout', { bubbles: true })); });
    expect(tip()).toBeNull();
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

  it('says which mode makes it act so, when the window knows it', () => {
    act(() => root.render(<NoAskBadge agent={{ skipsPermissions: true }} session cause="Mode: YOLO" />));
    act(() => host.querySelector<HTMLElement>('[data-no-ask]')!.focus());
    expect(tip()?.textContent).toBe('Runs without asking: Acts without asking you first, until it ends or is switched back. Because of: Mode: YOLO');
  });
});
