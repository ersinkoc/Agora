// @vitest-environment jsdom
// Settings as cards with setting rows (redesign PR 8): each section's controls by role and keyboard, and the Remove confirm.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import { attentionPrefs } from '../net/attention';
import { when } from '../net/time';
import { DevicePage, type MyDevice } from '../screens';
import { themeStore } from '../themes/store';
import type { MachineModel } from '../net/view';
import { demoSettingsKinds } from '../fixtures';
import { AgentsSettings } from './AgentsSettings';
import { AttentionSettings } from './AttentionSettings';
import { ScreenPasswordSettings } from './Locks';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); delete (globalThis as { Notification?: unknown }).Notification; vi.restoreAllMocks(); });

const render = (el: React.ReactElement) => act(() => root.render(el));
const flush = () => act(async () => { await new Promise((r) => setTimeout(r, 0)); });
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })); });
const click = (el: Element) => act(() => { (el as HTMLElement).click(); });
const byName = (role: string, name: string | RegExp, within: ParentNode = document) =>
  [...within.querySelectorAll<HTMLElement>(`[role="${role}"], ${role === 'button' ? 'button:not([role])' : role === 'link' ? 'a[href]' : '_'}`)].find((el) => {
    const n = el.getAttribute('aria-label') ?? (el.getAttribute('aria-labelledby') ? document.getElementById(el.getAttribute('aria-labelledby') ?? '')?.textContent : el.textContent) ?? '';
    return typeof name === 'string' ? n === name : name.test(n);
  });

const NOW = Date.UTC(2026, 10, 22, 12, 0);
const machines: MachineModel[] = [
  { id: 'laptop', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: NOW - 4000, build: '0.3.35' },
  { id: 'mac', name: 'mac', os: 'macos', online: true, agentsRunning: 0, lastSeenMs: NOW - 9000 },
  { id: 'win', name: 'win', os: 'windows', online: false, agentsRunning: 0, lastSeenMs: NOW - 7_200_000 },
];
const other = (over: Partial<MyDevice> = {}): MyDevice => ({ id: 'web-aaaa1111', name: 'Chrome · Linux', fingerprint: '7188-eda9', lastUsedMs: NOW - 60_000, trustedOn: [], waiting: false, removed: false, ...over });
const page = (over: Partial<Parameters<typeof DevicePage>[0]> = {}) =>
  <DevicePage device={undefined} onBack={() => {}} now={NOW} machines={machines} trustedOn={['laptop']} others={[other()]} {...over} />;

describe('the Settings page', () => {
  it('has a 20 px page title, two group titles and one card per section, with no all-caps heading', () => {
    render(page({ security: <p>lock</p>, onAddPasskey: async () => ({ ok: true }) }));
    expect(host.querySelector('h1')?.textContent).toBe('Settings');
    expect([...host.querySelectorAll('h2')].map((h) => h.textContent)).toEqual(['This device', 'My machines']);
    for (const h of host.querySelectorAll('h2')) expect(h.className).toContain('set-group__title');
    const titles = [...host.querySelectorAll('section.set-card h3')].map((h) => h.textContent);
    expect(titles).toEqual(['Appearance', 'Keyboard', 'Needs you', 'Sounds', 'Passkey', 'Your other devices', 'Machines']);
    // Each card is a region named by its title.
    for (const s of host.querySelectorAll('section.set-card')) expect(document.getElementById(s.getAttribute('aria-labelledby') ?? '')?.tagName).toBe('H3');
    // No text is set in capitals: the side index's group labels are sentence case too (visual pass 2026-11, finding 12).
    expect(host.querySelectorAll('[class*="uppercase"]')).toHaveLength(0);
  });

  it('has a side index with an icon per section, the current one marked, and a count of other devices', () => {
    render(page({ section: 'devices', others: [other(), other({ id: 'web-b', removed: true })] }));
    const nav = host.querySelector('nav[aria-label="Settings sections"]')!;
    const current = nav.querySelector('[aria-current="true"]')!;
    expect(current.getAttribute('href')).toBe('#/device/devices');
    expect(current.querySelector('svg')).not.toBeNull();
    expect(current.querySelector('.set-nav__count')?.textContent).toBe('1');
  });

  it('says how many machines trust this browser, and that it is not trusted yet when none does', () => {
    const device = { id: 'web-me', publicKeyHex: '04ab', keys: undefined as unknown as CryptoKeyPair };
    render(page({ device, fingerprint: '91c2-9a46', trustedOn: ['laptop', 'mac'] }));
    expect(host.querySelector('#settings-device')!.textContent).toContain('Trusted on 2 machines');
    render(page({ device, fingerprint: '91c2-9a46', trustedOn: [] }));
    const card = host.querySelector('#settings-device')!;
    expect(card.textContent).toContain('Not trusted yet');
    expect(card.textContent).toContain('agora node trust web-me 04ab');
  });

  it('lists machines as links with their system, version, state and trust', () => {
    render(page());
    const rows = [...host.querySelectorAll<HTMLAnchorElement>('#settings-machines a.set-item')];
    expect(rows.map((a) => a.getAttribute('href'))).toEqual(['#/machines/laptop', '#/machines/mac', '#/machines/win']);
    expect(rows[0]!.textContent).toContain('0.3.35');
    expect(rows[0]!.textContent).toContain('Online');
    expect(rows[0]!.textContent).toContain('trusts this browser');
    expect(rows[1]!.querySelector('.set-item__flag')?.textContent).toBe('does not trust this browser yet');
    expect(rows[2]!.textContent).toContain('Offline');
    expect(host.querySelector('#settings-machines')!.textContent).toContain('2 of 3 online');
    expect(host.querySelector('.set-group__head a[href="#/machines"]')?.textContent).toContain('Manage machines');
  });

  it('makes node-supplied machine names and builds visible', () => {
    const raw: MachineModel = { ...machines[0]!, name: 'lap\u3164top\u202e', build: 'v1\u034f.2\ue000' };
    render(page({ machines: [raw] }));
    const row = host.querySelector('#settings-machines a.set-item')!;
    expect(row.textContent).toContain('lap\\u{3164}top\\u{202e}');
    expect(row.textContent).toContain('v1\\u{34f}.2\\u{e000}');
  });

  it('verifies machine labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u2069', '\\u{2069}'], ['\u0001', '\\u{1}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const raw: MachineModel = { ...machines[0]!, name: `lap${char}top`, build: `v1${char}.2` };
      render(page({ machines: [raw] }));
      const row = host.querySelector('#settings-machines a.set-item')!;
      expect(row.textContent).toContain(`lap${escaped}top`);
      expect(row.textContent).toContain(`v1${escaped}.2`);
      expect(row.getAttribute('href')).toBe('#/machines/laptop');
    }
  });
});

describe('Appearance, This device and Passkey', () => {
  it('chooses the colour scheme with a segmented control named by its row, the same choice as the title bar', () => {
    act(() => themeStore.setMode('system'));
    render(page());
    const group = byName('radiogroup', 'Colour scheme')!;
    expect([...group.querySelectorAll('[role="radio"]')].map((r) => r.textContent)).toEqual(['System', 'Dark', 'Light']);
    key(group.querySelector('[aria-checked="true"]')!, 'ArrowRight');
    expect(themeStore.state().mode).toBe('dark');
    expect(group.querySelector('[aria-checked="true"]')?.textContent).toBe('Dark');
    act(() => themeStore.setMode('system'));
  });

  it('asks another device to approve this one, and says how it went', async () => {
    const device = { id: 'web-me', publicKeyHex: '04ab', keys: undefined as unknown as CryptoKeyPair };
    let ok = false;
    const onAsk = vi.fn(async () => ok);
    render(page({ device, fingerprint: '91c2-9a46', trustedOn: [], onAsk }));
    const ask = byName('button', 'Ask another device to approve')!;
    expect(ask.getAttribute('data-variant')).toBe('primary');
    click(ask);
    await flush();
    expect(host.querySelector('#settings-device [role="alert"]')?.textContent).toBe(t('device-ask-failed'));
    ok = true;
    click(ask);
    await flush();
    expect(host.querySelector('#settings-device [role="status"]')?.textContent).toContain('91c2-9a46');
  });

  it('adds a passkey from the head of its card and says how it went', async () => {
    const onAddPasskey = vi.fn(async () => ({ ok: true as const }));
    render(page({ onAddPasskey }));
    const add = host.querySelector('#settings-passkeys .set-card__head button')!;
    expect(add.textContent).toBe('Add a passkey');
    click(add);
    await flush();
    expect(onAddPasskey).toHaveBeenCalledWith(undefined);
    expect(host.querySelector('#settings-passkeys [role="status"]')?.textContent).toBe(t('device-passkey-added'));
  });
});

describe('Your other devices', () => {
  it('shows each device with its fingerprint, a written date, and its trust as a pill', () => {
    render(page({ others: [other({ trustedOn: ['laptop'] }), other({ id: 'web-w', waiting: true }), other({ id: 'web-r', removed: true })] }));
    const rows = [...host.querySelectorAll('#settings-devices li')];
    expect(rows[0]!.textContent).toContain('7188-eda9');
    expect(rows[0]!.textContent).toContain(`used ${when(NOW - 60_000, NOW)}`);
    expect(rows[0]!.querySelector('.kit-pill')?.textContent).toBe('Trusted on laptop');
    expect(rows[1]!.querySelector('.kit-pill.kit-tone-needs')?.textContent).toBe('Waiting for your approval');
    expect(rows[2]!.querySelector('.kit-pill')?.textContent).toBe('Removed');
  });

  it('makes trusted machine names visible on other-device rows', () => {
    const raw = { ...machines[0]!, name: 'lap\u3164top\u202e\ue000' };
    render(page({ machines: [raw], others: [other({ trustedOn: [raw.id] })] }));
    expect(host.querySelector('#settings-devices .kit-pill')?.textContent).toBe('Trusted on lap\\u{3164}top\\u{202e}\\u{e000}');
  });

  it('verifies trusted machine labels while retaining raw trust ids', () => {
    for (const [char, escaped] of [['\u061c', '\\u{61c}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const raw = { ...machines[0]!, name: `ma${char}chine` };
      const trustedOn = [raw.id];
      render(page({ machines: [raw], others: [other({ trustedOn })] }));
      expect(host.querySelector('#settings-devices .kit-pill')?.textContent).toBe(`Trusted on ma${escaped}chine`);
      expect(trustedOn).toEqual(['laptop']);
    }
  });

  it('offers Remove only from a trusted browser, and never for a device already removed', () => {
    const onRemove = vi.fn(async () => true);
    render(page({ onRemove, trustedOn: [], others: [other()] }));
    expect(byName('button', /^Remove/)).toBeUndefined();
    render(page({ onRemove, others: [other({ removed: true })] }));
    expect(byName('button', /^Remove/)).toBeUndefined();
  });

  it('asks before removing: a danger dialog with Keep it first and focused; neither Keep nor Escape removes', async () => {
    const onRemove = vi.fn(async () => true);
    render(page({ onRemove }));
    const remove = byName('button', 'Remove Chrome · Linux · 1111')!;
    expect(remove.getAttribute('data-variant')).toBe('danger-ghost');
    click(remove);
    await flush();
    expect(onRemove).not.toHaveBeenCalled();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    expect(dialog.textContent?.replace(/[\u2068\u2069]/g, '')).toContain('Remove Chrome · Linux · 1111?');
    const buttons = [...dialog.querySelectorAll('button')];
    expect(buttons.map((b) => b.textContent)).toEqual(['Keep it', 'Remove']);
    expect(buttons[1]!.getAttribute('data-variant')).toBe('danger');
    expect(document.activeElement).toBe(buttons[0]);
    click(buttons[0]!);
    await flush();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    click(remove);
    await flush();
    key(document.querySelector('[role="dialog"]')!, 'Escape');
    await flush();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(onRemove).not.toHaveBeenCalled();
  });

  it('removes only from the red button, and says so when it could not', async () => {
    const onRemove = vi.fn(async () => false);
    render(page({ onRemove }));
    click(byName('button', /^Remove Chrome/)!);
    await flush();
    const confirm = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((b) => b.textContent === 'Remove')!;
    click(confirm);
    await flush();
    expect(onRemove).toHaveBeenCalledWith('web-aaaa1111');
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(host.querySelector('#settings-devices [role="alert"]')?.textContent).toBe('Could not remove it.');
  });
});

describe('Needs you', () => {
  class FakeNotification { static permission: NotificationPermission = 'default'; static requestPermission = vi.fn(async () => 'granted' as NotificationPermission); }
  beforeEach(() => { FakeNotification.permission = 'default'; FakeNotification.requestPermission.mockClear(); (globalThis as { Notification?: unknown }).Notification = FakeNotification; attentionPrefs.setNotify('off'); attentionPrefs.setMode('modal'); });

  it('chooses the mode with a segmented control: one radio group named by its row, arrows move and choose', () => {
    render(<AttentionSettings />);
    const group = byName('radiogroup', 'When an agent needs you')!;
    expect(group.getAttribute('aria-describedby')).toBeTruthy();
    const radios = [...group.querySelectorAll('[role="radio"]')];
    expect(radios.map((r) => r.textContent)).toEqual(['Card', 'Banner', 'Nothing extra']);
    expect(radios.map((r) => r.getAttribute('tabindex'))).toEqual(['0', '-1', '-1']);
    key(radios[0]!, 'ArrowRight');
    expect(attentionPrefs.mode()).toBe('banner');
    key(group.querySelector('[aria-checked="true"]')!, 'End');
    expect(attentionPrefs.mode()).toBe('off');
  });

  it('turns notifications on with a switch, and asks the browser only then', async () => {
    render(<AttentionSettings />);
    expect(FakeNotification.requestPermission).not.toHaveBeenCalled();
    const sw = byName('switch', 'Notifications in the background')!;
    expect(sw.getAttribute('aria-checked')).toBe('false');
    click(sw);
    await flush();
    expect(FakeNotification.requestPermission).toHaveBeenCalledTimes(1);
    expect(attentionPrefs.notify()).toBe('generic');
    expect(byName('switch', 'Notifications in the background')!.getAttribute('aria-checked')).toBe('true');
    // On: names are a second switch, with what they may show.
    const names = byName('switch', 'Include the agent and machine name')!;
    expect(host.textContent).toContain('These names come from the agents and machines themselves');
    click(names);
    expect(attentionPrefs.notify()).toBe('named');
    click(byName('switch', 'Notifications in the background')!);
    expect(attentionPrefs.notify()).toBe('off');
  });

  it('disables the switch and says why when the browser blocks notifications or has none', () => {
    FakeNotification.permission = 'denied';
    render(<AttentionSettings />);
    let sw = byName('switch', 'Notifications in the background') as HTMLButtonElement;
    expect(sw.disabled).toBe(true);
    expect(document.getElementById(sw.getAttribute('aria-describedby') ?? '')?.textContent).toBe(t('attention-notify-blocked'));
    act(() => root.unmount());
    delete (globalThis as { Notification?: unknown }).Notification;
    root = createRoot(host);
    render(<AttentionSettings />);
    sw = byName('switch', 'Notifications in the background') as HTMLButtonElement;
    expect(sw.disabled).toBe(true);
    expect(host.textContent).toContain(t('attention-notify-unsupported'));
  });
});

describe('Screen password', () => {
  it('is a form whose labels name their fields, and keeps the password rules', async () => {
    const onSet = vi.fn(async () => ({ ok: true as const }));
    render(<ScreenPasswordSettings passwordSet={false} onSet={onSet} idle={0} onIdle={() => {}} />);
    const field = (name: string) => host.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
    for (const [name, label] of [['new', 'New password'], ['again', 'Type it again']] as const) {
      expect(host.querySelector(`label[for="${field(name).id}"]`)?.textContent).toBe(label);
    }
    expect(host.textContent).toContain('Not set');
    const type = (el: HTMLInputElement, v: string) => act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); });
    const submit = () => act(() => { host.querySelector('form')!.requestSubmit(); });
    type(field('new'), 'short');
    submit();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('Use at least 8 characters.');
    type(field('new'), 'long enough');
    type(field('again'), 'different!!');
    submit();
    expect(host.querySelector('[role="alert"]')?.textContent).toBe('The two passwords do not match.');
    type(field('again'), 'long enough');
    submit();
    await flush();
    expect(onSet).toHaveBeenCalledWith('long enough', undefined);
    expect(host.querySelector('[role="status"]')?.textContent).toBe('Saved. Every other window was signed out.');
    // The idle lock is a select named by its row.
    const select = host.querySelector('select')!;
    expect(document.getElementById(select.getAttribute('aria-labelledby') ?? '')?.textContent).toBe('Lock this window when idle');
  });
});

describe('Agents', () => {
  const load = vi.fn(async (node: string) => (node === 'mac' ? demoSettingsKinds.slice(0, 2) : demoSettingsKinds));
  it('shows one machine at a time as kind cards: avatars, a risk bolt, a dashed Not installed, details on opening', async () => {
    localStorage.clear();
    render(<AgentsSettings machines={machines} loadKinds={load} />);
    await flush();
    const cards = () => [...host.querySelectorAll('.set-kind')];
    // Hand-made kinds first, the ACP registry's (untested) after them, in each group; the ones it cannot find under "Add an agent".
    expect(cards().map((c) => c.querySelector('.set-kind__title')?.textContent)).toEqual(['Antigravity', 'Claude Code', 'Codex', 'Fake chat agent', 'WrongStack', 'Qwen Code', 'opencode', 'goose', 'Kiro CLI']);
    const claude = cards()[1]!;
    expect(claude.querySelector('.kit-avatar')?.textContent).toBe('C');
    const bolt = claude.querySelector('[role="img"]')!;
    expect(bolt.getAttribute('aria-label')).toContain('skips permission questions');
    expect(cards()[0]!.querySelector('[role="img"]')).toBeNull();
    const missing = cards()[6]!;
    expect(missing.className).toContain('set-kind--missing');
    expect(missing.textContent).toContain('Not installed');
    // The rest is in the card, read by opening it (a native disclosure: Enter and Space open it).
    expect(claude.querySelector('summary')).not.toBeNull();
    expect(claude.querySelector('.set-kind__more')!.textContent).toContain('low, medium, high, xhigh, max');
    expect(claude.querySelector('.set-kind__more')!.textContent).toContain("Don't ask (skips permission questions)");
    expect(missing.querySelector('.set-kind__more')!.textContent).toContain('cannot find opencode');
    // Nothing here starts, installs or saves anything.
    expect([...host.querySelectorAll('button:not([role="radio"])')]).toEqual([]);
    // Another machine: the segmented control at the top.
    const group = byName('radiogroup', 'Machine')!;
    key(group.querySelector('[aria-checked="true"]')!, 'ArrowRight');
    await flush();
    expect(cards().map((c) => c.querySelector('.set-kind__title')?.textContent)).toEqual(['Claude Code', 'Codex']);
    // An offline machine that never answered says so.
    key(group.querySelector('[aria-checked="true"]')!, 'ArrowRight');
    expect(host.textContent).toContain(t('agents-none-known'));
  });
});

describe('Add an agent (ADR-0092)', () => {
  const load = vi.fn(async () => demoSettingsKinds);
  it('lists every kind the machine has or misses, the registry ones with version, licence, a repository link and an Untested mark', async () => {
    localStorage.clear();
    render(<AgentsSettings machines={machines.slice(0, 1)} loadKinds={load} />);
    await flush();
    const add = host.querySelector('.set-kinds-add')!;
    expect(add.querySelector('h4')?.textContent).toBe('Add an agent 3');
    const titles = [...add.querySelectorAll('.set-kind__title')].map((e) => e.textContent);
    expect(titles).toEqual(['opencode', 'goose', 'Kiro CLI']);
    const card = (name: string) => [...host.querySelectorAll('.set-kind')].find((c) => c.querySelector('.set-kind__title')?.textContent === name)!;
    const goose = card('goose');
    // The mark is on the face; a hand-made kind has none.
    expect(goose.querySelector('.set-kind__face')!.textContent).toContain('Untested');
    expect(card('opencode').textContent).not.toContain('Untested');
    expect(card('Codex').textContent).not.toContain('Untested');
    // Its icon is a mask over the text colour, never markup; a kind without one keeps its letter.
    const icon = goose.querySelector<HTMLElement>('.kit-avatar__icon')!;
    expect(icon.style.getPropertyValue('--icon')).toMatch(/^url\(/);
    expect(goose.querySelector('.kit-avatar svg, .kit-avatar img')).toBeNull();
    const more = goose.querySelector('.set-kind__more')!.textContent ?? '';
    expect(more).toContain('Version1.60.2');
    expect(more).toContain('LicenseApache-2.0');
    expect(more).toContain('Listed byACP registry');
    expect(more).toContain('cannot find goose');
    expect(more).toContain('Nobody has tried it in Agora yet');
    // The licence links to its text; the repository is the other link.
    const [licence, link] = [...goose.querySelectorAll<HTMLAnchorElement>('a[href]')];
    expect(licence!.textContent).toBe('Apache-2.0');
    expect(licence!.href).toMatch(/^https:\/\/github\.com\/block\/goose\/.*LICENSE/);
    expect(licence!.rel).toBe('noopener noreferrer');
    expect(link!.href).toBe('https://github.com/block/goose');
    expect(link!.rel).toBe('noopener noreferrer');
    expect(link!.target).toBe('_blank');
    expect(link!.textContent).toBe('github.com/block/goose');
    // One that starts through npx says that it downloads its pinned version the first time.
    expect(card('Qwen Code').querySelector('.set-kind__more')!.textContent).toContain('Starting it runs npx');
    // Nothing here is a button: looking runs nothing, installs nothing.
    expect([...host.querySelectorAll('button')]).toEqual([]);
  });

  it('has the same words in Turkish', async () => {
    setLanguage('tr');
    localStorage.clear();
    render(<AgentsSettings machines={machines.slice(0, 1)} loadKinds={load} />);
    await flush();
    expect(host.querySelector('.set-kinds-add h4')?.textContent).toBe('Ajan ekle 3');
    expect(host.textContent).toContain('Denenmedi');
  });
});

describe('dates', () => {
  it('writes today and yesterday in words, and an older day with its month', () => {
    const now = new Date(2026, 10, 22, 21, 0).getTime();
    expect(when(new Date(2026, 10, 22, 9, 5).getTime(), now)).toMatch(/^today, /);
    expect(when(new Date(2026, 10, 21, 23, 59).getTime(), now)).toMatch(/^yesterday, /);
    expect(when(new Date(2026, 9, 3, 8, 0).getTime(), now)).toMatch(/Oct/);
    expect(when(new Date(2025, 9, 3, 8, 0).getTime(), now)).toMatch(/2025/);
    setLanguage('tr');
    expect(when(new Date(2026, 10, 22, 9, 5).getTime(), now)).toMatch(/^bugün, /);
  });

  it('has a short word for every mode and way of running', () => {
    for (const m of ['modal', 'banner', 'off']) expect(t(`attention-mode-short-${m}`)).not.toBe(`attention-mode-short-${m}`);
    for (const r of ['terminal', 'chat', 'adapter']) expect(t(`agents-runs-short-${r}`)).not.toBe(`agents-runs-short-${r}`);
  });
});
