// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { Menu } from './Menu';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
const proto = HTMLElement.prototype;
const saved = { rect: proto.getBoundingClientRect, ow: Object.getOwnPropertyDescriptor(proto, 'offsetWidth'), sh: Object.getOwnPropertyDescriptor(Element.prototype, 'scrollHeight') };
let buttonTop = 0;

beforeEach(() => {
  setLanguage('en');
  Object.defineProperty(window, 'innerWidth', { value: 390, configurable: true });
  Object.defineProperty(window, 'innerHeight', { value: 800, configurable: true });
  // jsdom has no layout: the trigger sits where the test says, the list is 224 by 200.
  proto.getBoundingClientRect = function (this: HTMLElement) {
    const top = this.tagName === 'BUTTON' && this.getAttribute('aria-haspopup') === 'menu' && !this.closest('[role=menu]') ? buttonTop : 0;
    return { left: 20, right: 100, top, bottom: top + 28, width: 80, height: 28, x: 20, y: top, toJSON() {} } as DOMRect;
  };
  Object.defineProperty(proto, 'offsetWidth', { value: 224, configurable: true });
  Object.defineProperty(Element.prototype, 'scrollHeight', { value: 200, configurable: true });
  host = document.createElement('div');
  host.style.overflow = 'hidden';
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  proto.getBoundingClientRect = saved.rect;
  if (saved.ow) Object.defineProperty(proto, 'offsetWidth', saved.ow); else delete (proto as { offsetWidth?: number }).offsetWidth;
  if (saved.sh) Object.defineProperty(Element.prototype, 'scrollHeight', saved.sh); else delete (Element.prototype as { scrollHeight?: number }).scrollHeight;
});

const items = [{ label: 'One' }, { label: 'Two' }, { label: 'Three' }];
const open = () => {
  act(() => root.render(<Menu label="Mode" items={items} align="start"><span>Mode</span></Menu>));
  act(() => { host.querySelector('button')?.click(); });
};

describe('Menu placement', () => {
  it('opens downward with room below, in a portal outside clipping ancestors', () => {
    buttonTop = 100;
    open();
    const menu = document.body.querySelector<HTMLElement>('[role=menu]');
    expect(menu).not.toBeNull();
    expect(host.contains(menu)).toBe(false);
    expect(menu?.style.position).toBe('fixed');
    expect(Number.parseFloat(menu?.style.top ?? '')).toBe(132);
  });
  it('opens upward when the trigger is at the bottom of the screen', () => {
    buttonTop = 760;
    open();
    const menu = document.body.querySelector<HTMLElement>('[role=menu]');
    const top = Number.parseFloat(menu?.style.top ?? '');
    expect(top + 200).toBeLessThanOrEqual(756); // its bottom edge is above the button
    expect(top).toBeGreaterThanOrEqual(0);
  });
  it('keeps the keyboard behaviour: first item focused, End, Escape closes and returns focus', () => {
    buttonTop = 760;
    open();
    expect(document.activeElement).toBe(document.body.querySelector('[role=menuitem]'));
    act(() => { document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true })); });
    expect(document.activeElement?.textContent).toBe('Three');
    act(() => { document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); });
    expect(document.body.querySelector('[role=menu]')).toBeNull();
    expect(document.activeElement).toBe(host.querySelector('button'));
  });
});
