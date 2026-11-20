// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import { isMachinesHash, isSettingsHash, settingsSectionOf } from '../net/paletteExtras';
import type { MachineModel } from '../net/view';
import { DevicePage } from '../screens';
import { demoSource } from '../terminal/source';
import { Console } from './Console';
import { HeaderLinks } from './HeaderLinks';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); window.location.hash = ''; host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const link = (name: string) => host.querySelector<HTMLAnchorElement>(`a[aria-label="${name}"]`);

describe('Settings and Machines icons in the title bar', () => {
  it('are two links with a name and a tooltip, in English and in Turkish', () => {
    act(() => root.render(<HeaderLinks />));
    const s = link('Settings');
    const m = link('Machines');
    expect(s?.getAttribute('href')).toBe('#/device');
    expect(m?.getAttribute('href')).toBe('#/machines');
    expect(s?.getAttribute('title')).toBe('Settings');
    expect(s?.querySelector('svg')).not.toBeNull();
    expect(s?.textContent).toBe(''); // an icon, no visible text
    setLanguage('tr');
    act(() => root.render(<HeaderLinks hash="" />));
    expect(link('Ayarlar')?.getAttribute('title')).toBe('Ayarlar');
    expect(link('Makineler')).not.toBeNull();
  });

  it('mark the page that is open, and follow the address', () => {
    act(() => root.render(<HeaderLinks />));
    expect(link('Settings')?.getAttribute('aria-current')).toBeNull();
    act(() => { window.location.hash = '#/device/passkeys'; window.dispatchEvent(new HashChangeEvent('hashchange')); });
    expect(link('Settings')?.getAttribute('aria-current')).toBe('page');
    expect(link('Machines')?.getAttribute('aria-current')).toBeNull();
    act(() => { window.location.hash = '#/machines/laptop'; window.dispatchEvent(new HashChangeEvent('hashchange')); });
    expect(link('Machines')?.getAttribute('aria-current')).toBe('page');
    expect(link('Settings')?.getAttribute('aria-current')).toBeNull();
  });

  it('are reachable by keyboard (links, in tab order)', () => {
    act(() => root.render(<HeaderLinks />));
    for (const a of host.querySelectorAll('a')) expect(a.getAttribute('tabindex')).toBeNull();
  });

  it('sit in the window title bar before Lock, replacing the text "This device"', () => {
    const props = { agents: [], machines: [], approvals: [], messages: [], terminalFor: () => demoSource('x'), onOpen: () => {}, onResolve: () => {}, link: 'live' as const };
    const html = renderToStaticMarkup(<Console {...props} onLock={() => {}} />);
    expect(html).toContain('aria-label="Settings"');
    expect(html).toContain('aria-label="Machines"');
    expect(html.indexOf('aria-label="Machines"')).toBeLessThan(html.indexOf('>Lock<'));
    expect(html).not.toContain('>This device<');
  });
});

describe('routes', () => {
  it('keep #/device and #/machines and add sections', () => {
    expect(isSettingsHash('#/device')).toBe(true);
    expect(isSettingsHash('#/settings')).toBe(true);
    expect(isSettingsHash('#/devices')).toBe(false);
    expect(settingsSectionOf('#/device/passkeys')).toBe('passkeys');
    expect(settingsSectionOf('#/settings/machines')).toBe('machines');
    expect(settingsSectionOf('#/device')).toBeUndefined();
    expect(isMachinesHash('#/machines/laptop')).toBe(true);
  });
});

describe('the Settings page', () => {
  const machines: MachineModel[] = [
    { id: 'laptop', name: 'Laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: Date.now(), build: '0.3.28' },
    { id: 'vps', name: 'VPS', os: 'linux', online: false, agentsRunning: 0, lastSeenMs: Date.now() - 3 * 86_400_000 },
  ];
  it('has a section list with an anchor per section and lists the machines with a link to each', () => {
    const html = renderToStaticMarkup(<DevicePage device={undefined} onBack={() => {}} section="machines" machines={machines} trustedOn={['laptop']} security={<p>lock</p>} onAddPasskey={async () => ({ ok: true })} />);
    for (const id of ['appearance', 'notifications', 'passkeys', 'lock', 'machines']) {
      expect(html).toContain(`href="#/device/${id}"`);
      expect(html).toContain(`id="settings-${id}"`);
    }
    expect(html).toContain('aria-current="true"');
    expect(html).toContain('href="#/machines/laptop"');
    expect(html).toContain('href="#/machines/vps"');
    expect(html).toContain('0.3.28');
    expect(html).toContain('does not trust this browser yet');
    expect(html).toContain('This device');
    expect(html).toContain('My machines');
    expect(html).toContain('Manage machines');
  });
});
