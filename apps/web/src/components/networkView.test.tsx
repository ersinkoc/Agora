// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { GALLERY_NOW, galleryMachineAgents, galleryMachines, galleryPairing } from '../fixtures';
import { setLanguage } from '../i18n';
import type { MachineModel } from '../net/view';
import { NetworkView } from './NetworkView';
import type { PairingUi } from './Pairing';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = GALLERY_NOW;
const machine = (id: string, os: string): MachineModel => ({ id, name: id, os, online: true, agentsRunning: 0, lastSeenMs: NOW });
const systems = (machines: MachineModel[]) => {
  const html = renderToStaticMarkup(<NetworkView now={NOW} machines={machines} agents={[]} hubName="hub" />);
  return [...html.matchAll(/<bdi data-fact="os">([^<]*)<\/bdi>/g)].map((m) => m[1]);
};
const page = (over: Partial<Parameters<typeof NetworkView>[0]> = {}) =>
  renderToStaticMarkup(<NetworkView now={NOW} machines={galleryMachines} agents={galleryMachineAgents} hubName="localhost:7444" onForget={async () => new Map()} {...over} />);
// The text a person reads (tags and the isolating marks around names stripped).
const text = (html: string) => html.replace(/<[^>]+>/g, ' ').replace(/[⁨⁩]/g, '').replace(/\s+/g, ' ');
// One machine's card, as markup.
const card = (html: string, id: string) => {
  const start = html.indexOf(`<article id="machine-${id}"`);
  return html.slice(start, html.indexOf('</article>', start));
};

describe('My machines: the system (#944, RM-03)', () => {
  afterEach(() => setLanguage('en'));

  it('names each system through the os-* messages, not as the node reports it', () => {
    setLanguage('en');
    expect(systems([machine('a', 'windows'), machine('b', 'macos'), machine('c', 'linux')])).toEqual(['Windows', 'macOS', 'Linux']);
    setLanguage('tr');
    expect(systems([machine('a', 'windows'), machine('b', 'macos'), machine('c', 'linux')])).toEqual(['Windows', 'macOS', 'Linux']);
  });

  it('shows a system it has no words for as reported, and an unknown one as a question mark', () => {
    setLanguage('en');
    expect(systems([machine('a', 'freebsd'), machine('b', '')])).toEqual(['freebsd', '?']);
    // What a node sends is made safe to show: no control or reordering characters.
    expect(systems([machine('a', 'free‮bsd\u0007')])).toEqual(['free bsd']);
  });
});

describe('My machines as cards (redesign PR 9)', () => {
  beforeEach(() => setLanguage('en'));

  it('has no animated or decorative links: no diagram, no dashes, nothing that loops', () => {
    // The state marks are the one motion allowed (the working pulse, the needs-you mark); everything else is still.
    const html = page({ pairing: galleryPairing }).replace(/<svg class="state__mark".*?<\/svg>/g, '');
    expect(html).not.toContain('net-link');
    expect(html).not.toContain('stroke-dasharray');
    expect(html).not.toMatch(/<animate|<path d="M0 /);
    // And the stylesheet no longer has the loop that moved them.
    const css = readFileSync('src/design/app.css', 'utf8');
    expect(css).not.toContain('net-link');
    expect(css).not.toMatch(/@keyframes flow\b/);
  });

  it('says "Your Agora", never "hub", and counts machines online, agents running, what needs you and memory', () => {
    const t = text(page());
    expect(t).toContain('Your Agora');
    expect(t).not.toMatch(/\bhub\b/i);
    // Each count is said name first (a description list), and shown number first.
    expect(t).toContain('machines online 2 of 3');
    expect(t).toContain('agents running 5');
    expect(t).toContain('need you 2');
    expect(t).toContain('memory used by Agora 23 MB');
  });

  it('gives an online machine its system, version and when it was heard, a pill, its agents as rows, a meter and its actions', () => {
    const c = card(page({ onStart: () => {}, onNewTerminal: () => {} }), 'blackview');
    const t = text(c);
    expect(t).toMatch(/blackview Linux · 0\.3\.35 · heard 4 seconds ago Online/);
    expect(c).toContain('kit-pill kit-tone-finished');
    for (const a of ['claude · plum', 'codex · teal', 'opencode · sky', 'claude · amber']) expect(c).toContain(`>${a}<`);
    expect(c.match(/class="kit-row"/g)).toHaveLength(4);
    expect(t).toContain('Agora on this machine 17 MB · 6 terminals open');
    expect(c).toContain('aria-label="75 % of the memory Agora uses on all your machines"');
    expect(t).toContain('Start an agent here');
    expect(t).toContain('New terminal');
    expect(c).toContain('aria-label="More for blackview"');
    // Forget is for an offline machine only (ADR-0073).
    expect(t).not.toContain('Forget');
    // A failed agent is red with its word (ADR-0102).
    expect(card(page(), 'mac')).toContain('kit-tone-danger');
  });

  it('offers to start an agent or a terminal only with a live link', () => {
    const t = text(card(page(), 'blackview'));
    expect(t).not.toContain('Start an agent here');
    expect(t).not.toContain('New terminal');
  });

  it('shows the id once, and only when the name does not already say it', () => {
    const same = text(card(page(), 'blackview'));
    expect(same.split('blackview').length - 1).toBe(1);
    const other = text(card(renderToStaticMarkup(<NetworkView now={NOW} machines={[{ ...machine('node-7f3a', 'linux'), name: 'laptop' }]} agents={[]} hubName="h" />), 'node-7f3a'));
    expect(other.split('node-7f3a').length - 1).toBe(1);
    expect(other).toContain('laptop');
  });

  it('draws an offline machine dashed, says what was running there, and offers a red Forget…', () => {
    const c = card(page(), 'win');
    expect(c).toContain('machine-card--away');
    const t = text(c);
    expect(t).toContain('Windows · 0.3.34 · last seen 2 hours ago Offline');
    expect(t).toContain('2 agents were running');
    expect(t).toContain('They keep going on win. What they did appears here when it connects again.');
    expect(c).toContain('kit-callout kit-tone-accent');
    expect(t).toContain('Not coming back?');
    expect(c).toMatch(/data-variant="danger-ghost"[^>]*>.*Forget…/);
    // Without the forget command (the demo), no Forget at all; with no agents running there, no callout.
    expect(text(card(page({ onForget: undefined }), 'win'))).not.toContain('Forget');
    expect(card(page({ machines: [{ ...galleryMachines[2]!, agentsRunning: 0 }] }), 'win')).not.toContain('kit-callout');
  });

  it('counts as running only what runs on a machine online now, and says "machine" for one', () => {
    const away = { id: 'g9', name: 'codex · rose', kind: 'codex', accent: 'var(--agent-rose)', machine: 'win', machineId: 'win', workspace: 'w', state: 'working' as const, preview: '' };
    expect(text(page({ agents: [...galleryMachineAgents, away] }))).toContain('agents running 5');
    const one = text(renderToStaticMarkup(<NetworkView now={NOW} machines={[machine('solo', 'linux')]} agents={[]} hubName="h" />));
    expect(one).toContain('machine online 1 of 1');
  });

  it('tells two old machines with one host name apart by their ids, on the rows and in the clean-up dialog', () => {
    const old = (id: string): MachineModel => ({ id, name: 'laptop', os: 'linux', online: false, agentsRunning: 0, lastSeenMs: NOW - 3 * 86_400_000 });
    const t = text(renderToStaticMarkup(<NetworkView now={NOW} machines={[old('laptop-a1'), old('laptop-b2')]} agents={[]} hubName="h" />));
    expect(t).toContain('laptop laptop-a1');
    expect(t).toContain('laptop laptop-b2');
  });

  it('keeps old machines apart and folded, with Clean up… (ADR-0073)', () => {
    const html = page();
    expect(html).toMatch(/<details class="kit-card machines-old"><summary[^>]*>One old machine<\/summary>/);
    expect(html).not.toContain('<article id="machine-linux-test"');
    expect(text(html)).toContain('Clean up…');
  });

  it('ends the list with an "Add a machine" card that names agora join', () => {
    const t = text(page());
    expect(t).toContain('Add a machine Allow new machines, run agora join on it, then compare the number here.');
  });

  it('shows a request to join in full, in the needs-you tint, with the number before Deny and "Numbers match…"', () => {
    const html = page({ pairing: galleryPairing });
    const start = html.indexOf('pair-card');
    expect(start).toBeGreaterThan(-1);
    expect(html).toContain('pair-card--wide w-full pair-card kit-tone-needs');
    // Who asks: the name, the system and the address each on a row of their own.
    const t = text(html);
    expect(t).toContain('A new machine wants to join Name desk System Linux Address 192.168.1.52');
    const number = html.indexOf('class="pair-number"');
    expect(number).toBeGreaterThan(start);
    expect(number).toBeLessThan(html.indexOf('>Deny<'));
    expect(number).toBeLessThan(html.indexOf('>Numbers match…<'));
    expect(html).toContain('<span>640</span><span>218</span>');
    // It is in place, not floating over the page.
    expect(html).not.toContain('pair-card--float');
  });
});

describe('My machines in a browser', () => {
  let host: HTMLElement;
  let root: Root;
  beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(() => { act(() => root.unmount()); host.remove(); document.body.innerHTML = ''; });
  const button = (label: string | RegExp, within: ParentNode = document) => {
    const b = [...within.querySelectorAll('button')].find((x) => (typeof label === 'string' ? x.textContent === label : label.test(x.textContent ?? '')));
    if (!b) throw new Error(`no button ${label}`);
    return b as HTMLButtonElement;
  };
  const strip = (s: string | null | undefined) => (s ?? '').replace(/[⁨⁩]/g, '');

  it('"Add a machine" shows the steps with agora join and the switch, and never opens the switch by itself', () => {
    const onOpen = vi.fn(async () => true);
    const pairing: PairingUi = { ...galleryPairing, cards: [], onOpen };
    act(() => root.render(<NetworkView now={NOW} machines={galleryMachines} agents={[]} hubName="h" pairing={pairing} />));
    const add = host.querySelector('button[aria-controls="machines-add"][aria-expanded]') as HTMLButtonElement;
    expect(add.getAttribute('aria-expanded')).toBe('false');
    expect(add.dataset.variant).toBe('primary');
    act(() => add.click());
    expect(add.getAttribute('aria-expanded')).toBe('true');
    const steps = host.querySelector('#machines-add') as HTMLElement;
    expect(steps.textContent).toContain('agora join');
    expect(steps.querySelector('code')?.textContent).toBe('agora join');
    expect(button('Allow a new machine to join', steps)).toBeTruthy();
    expect(onOpen).not.toHaveBeenCalled();
    act(() => button('Allow a new machine to join', steps).click());
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('Forget… opens a danger dialog: Keep first and focused, the machine as a row, the refuse box, then "Forget win" in red', async () => {
    const onForget = vi.fn(async (ids: string[]) => new Map(ids.map((id) => [id, undefined])));
    act(() => root.render(<NetworkView now={NOW} machines={galleryMachines} agents={galleryMachineAgents} hubName="h" onForget={onForget} />));
    act(() => button('Forget…', host.querySelector('#machine-win') as HTMLElement).click());
    const dialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog).toBeTruthy();
    expect(strip(dialog.textContent)).toContain('Forget win?');
    expect(dialog.querySelector('.kit-chip.kit-tone-danger')).toBeTruthy();
    // The machine as a row: its system, when it was last seen, what was running there then.
    expect(strip(dialog.querySelector('.forget-row')?.textContent)).toContain('winWindows · 0.3.34 · last seen 2 hours ago · 2 agents were running then; they may still run there');
    const actions = [...dialog.querySelectorAll('button')].filter((b) => b.dataset.variant);
    expect(actions.map((b) => strip(b.textContent))).toEqual(['Keep', 'Forget win']);
    expect(actions.map((b) => b.dataset.variant)).toEqual(['secondary', 'danger']);
    expect(document.activeElement).toBe(actions[0]);
    // The refuse option is a bordered row with its box, its line and its hint.
    const refuse = dialog.querySelector('label.forget-row--check') as HTMLLabelElement;
    expect(refuse.textContent).toBe('Also refuse it if it connects againIt would need a new join code.');
    act(() => (refuse.querySelector('input') as HTMLInputElement).click());
    await act(async () => { actions[1]!.click(); });
    expect(onForget).toHaveBeenCalledWith(['win'], true);
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  });

  it('Keep forgets nothing', () => {
    const onForget = vi.fn(async () => new Map());
    act(() => root.render(<NetworkView now={NOW} machines={galleryMachines} agents={[]} hubName="h" onForget={onForget} />));
    act(() => button('Forget…', host.querySelector('#machine-win') as HTMLElement).click());
    act(() => button('Keep').click());
    expect(onForget).not.toHaveBeenCalled();
    expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  });

  it('Clean up… lists every old machine ticked, refuses with the plural wording, and shows a refusal in red', async () => {
    const onForget = vi.fn(async (ids: string[]) => new Map(ids.map((id) => [id, 'node.online'])));
    act(() => root.render(<NetworkView now={NOW} machines={galleryMachines} agents={[]} hubName="h" onForget={onForget} />));
    act(() => button('Clean up…').click());
    const dialog = document.querySelector('[role="alertdialog"]') as HTMLElement;
    expect(dialog.textContent).toContain('Clean up one old machine?');
    // The row names the machine and, as on its card, its id once when the name does not say it.
    expect(dialog.querySelector('.forget-row')?.textContent).toContain('linuxlinux-test');
    const boxes = [...dialog.querySelectorAll('ul input[type="checkbox"]')] as HTMLInputElement[];
    expect(boxes.map((b) => b.checked)).toEqual([true]);
    expect(dialog.querySelector('label.forget-row--check')?.textContent).toBe('Also refuse them if they connect againEach would need a new join code.');
    const forget = button('Forget', dialog);
    expect(forget.dataset.variant).toBe('danger');
    act(() => boxes[0]!.click());
    expect(forget.disabled).toBe(true);
    act(() => boxes[0]!.click());
    await act(async () => { forget.click(); });
    expect(onForget).toHaveBeenCalledWith(['linux-test'], false);
    const alert = dialog.querySelector('[role="alert"]') as HTMLElement;
    expect(alert.textContent).toBe('It is connected again, so it was not forgotten.');
    expect(alert.className).toContain('text-danger');
  });
});
