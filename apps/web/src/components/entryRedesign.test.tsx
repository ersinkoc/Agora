// @vitest-environment jsdom
// The redesign's entry screens (ADR-0102, PR 11): the copy chip, the empty Home, the command palette and the sign-in card.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { MachineModel } from '../net/view';
import { SignInCard, SignInPage } from '../screens';
import { CommandPalette, groupPalette, type PaletteItem } from './CommandPalette';
import { FleetEmpty } from './FleetConsole';
import { CopyChip } from './kit';
import type { PairingUi } from './Pairing';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
const clipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  if (clipboard) Object.defineProperty(navigator, 'clipboard', clipboard); else delete (navigator as { clipboard?: unknown }).clipboard;
  vi.useRealTimers();
});
const render = (el: React.ReactNode) => act(() => root.render(el));
const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector<T>(sel) as T;
const $$ = (sel: string) => Array.from(document.querySelectorAll<HTMLElement>(sel));
const button = (text: string) => $$('button').find((b) => b.textContent?.trim() === text) as HTMLButtonElement;
const setClipboard = (writeText: unknown) => Object.defineProperty(navigator, 'clipboard', { value: writeText === undefined ? undefined : { writeText }, configurable: true });
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })); });
const type = (el: HTMLInputElement, value: string) => act(() => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set?.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
});
const machine = (id: string, over: Partial<MachineModel> = {}): MachineModel => ({ id, name: id, os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 0, ...over });

describe('copy chip', () => {
  it('copies the text and says so', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard(writeText);
    render(<CopyChip text="agora hub login" />);
    await act(async () => { button('Copy').click(); });
    expect(writeText).toHaveBeenCalledWith('agora hub login');
    expect(button('Copied')).toBeTruthy();
    expect(window.getSelection()?.toString()).toBe('');
  });

  it('selects the text when the clipboard refuses, and says to copy it with the keyboard', async () => {
    setClipboard(vi.fn().mockRejectedValue(new DOMException('denied', 'NotAllowedError')));
    render(<CopyChip text="agora hub login" />);
    await act(async () => { button('Copy').click(); });
    expect(window.getSelection()?.toString()).toBe('agora hub login');
    expect($('[role="status"]').textContent).toBe(t('copy-selected'));
    expect($('[role="status"]').className).not.toContain('sr-only');
    expect(button('Copy')).toBeTruthy(); // never claims a copy that did not happen
  });

  it('a refusal right after a copy keeps its note (the Copied timer does not clear it)', async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('denied'));
    setClipboard(writeText);
    render(<CopyChip text="agora hub code" />);
    await act(async () => { button('Copy').click(); });
    await act(async () => { button('Copied').click(); });
    await act(async () => { vi.advanceTimersByTime(2500); });
    expect($('[role="status"]').textContent).toBe(t('copy-selected'));
  });

  it('selects the text where there is no clipboard at all (an insecure origin)', async () => {
    setClipboard(undefined);
    render(<CopyChip text="agora agent start codex" />);
    await act(async () => { button('Copy').click(); });
    expect(window.getSelection()?.toString()).toBe('agora agent start codex');
    expect($('[role="status"]').textContent).toBe(t('copy-selected'));
  });

  it('shows the command as it is typed, left to right, in any language', () => {
    setLanguage('tr');
    render(<CopyChip text="agora hub login" />);
    expect($('code').getAttribute('dir')).toBe('ltr');
    expect($('code').textContent).toBe('agora hub login');
    expect(button('Kopyala')).toBeTruthy();
  });
});

describe('the empty Home', () => {
  const pairing = (): PairingUi => ({ cards: [], recent: [], offline: false, onAnswer: vi.fn(), onOpen: vi.fn().mockResolvedValue(true) });

  it('is centred: a terminal chip, a title, one line, Start an agent and Add a machine, and the command to copy', () => {
    render(<FleetEmpty kind="first-run" machines={[machine('n1')]} onStart={() => {}} />);
    const box = $('.fleet-empty');
    expect(box.querySelector('.kit-chip')).toBeTruthy();
    expect(box.querySelector('h2')?.textContent).toBe(t('fleet-firstrun-title'));
    expect(box.querySelector('.fleet-empty__text')?.textContent).toBe(t('fleet-firstrun-text'));
    const actions = Array.from(box.querySelectorAll<HTMLButtonElement>('.fleet-empty__actions button'));
    expect(actions.map((b) => [b.textContent, b.dataset.variant])).toEqual([[t('action-start-agent'), 'primary'], [t('fleet-add-machine'), 'secondary']]);
    expect(box.querySelector('.kit-copy code')?.textContent).toBe('agora agent start codex --cwd /path/to/project');
  });

  it('Start an agent starts on the one machine that is online', () => {
    const onStart = vi.fn();
    render(<FleetEmpty kind="first-run" machines={[machine('n1'), machine('n2', { online: false })]} onStart={onStart} />);
    act(() => button(t('action-start-agent')).click());
    expect(onStart).toHaveBeenCalledWith('n1');
    render(<FleetEmpty kind="first-run" machines={[machine('n1'), machine('n2')]} onStart={onStart} />);
    act(() => button(t('action-start-agent')).click());
    expect(onStart).toHaveBeenLastCalledWith(undefined); // two machines: the dialog asks which
  });

  it('Add a machine shows how a machine joins, in place, and opens nothing by itself', () => {
    const ui = pairing();
    render(<FleetEmpty kind="first-run" machines={[machine('n1')]} onStart={() => {}} pairing={ui} />);
    const add = button(t('fleet-add-machine'));
    const panel = document.getElementById(add.getAttribute('aria-controls') ?? '') as HTMLElement;
    expect(add.getAttribute('aria-expanded')).toBe('false');
    expect(panel.hidden).toBe(true);
    act(() => add.click());
    expect(add.getAttribute('aria-expanded')).toBe('true');
    expect(panel.hidden).toBe(false);
    expect(Array.from(panel.querySelectorAll('code')).map((c) => c.textContent?.split(' --')[0])).toEqual(['agora hub code', 'agora node enroll']);
    expect(panel.querySelector('.pair-window')).toBeTruthy(); // the owner's own button for the window, one more press away
    expect(ui.onOpen).not.toHaveBeenCalled();
    act(() => add.click());
    expect(panel.hidden).toBe(true);
  });

  it('keeps an open window for new machines in view, with its Close button, even with Add a machine folded', () => {
    const ui: PairingUi = { ...pairing(), window: { open: true, closesAtMs: Date.now() + 60_000, pending: 0 }, onClose: vi.fn().mockResolvedValue(true) };
    render(<FleetEmpty kind="first-run" machines={[machine('n1')]} onStart={() => {}} pairing={ui} />);
    const add = button(t('fleet-add-machine'));
    expect(add.getAttribute('aria-expanded')).toBe('true');
    expect(button(t('pair-window-close'))).toBeTruthy();
  });

  it('with every machine offline it says so and offers no start, only Add a machine', () => {
    render(<FleetEmpty kind="first-run" machines={[machine('n1', { online: false })]} onStart={() => {}} />);
    expect($('.fleet-empty__text').textContent).toBe(t('fleet-firstrun-offline'));
    expect(button(t('action-start-agent'))).toBeUndefined();
    expect(button(t('fleet-add-machine'))).toBeTruthy();
    expect($('.kit-copy')).toBeNull();
  });
});

describe('command palette', () => {
  const items = (run = vi.fn()): PaletteItem[] => [
    { id: 'next', label: 'Next that needs me', run: () => run('next'), group: 'go', icon: 'bell', keys: 'N' },
    { id: 'home', label: 'Go to Home', run: () => run('home'), group: 'go', icon: 'grid' },
    { id: 'start', label: 'Start an agent', run: () => run('start'), icon: 'plus' },
    { id: 'theme-dark', label: 'Theme: Dark', run: () => run('theme'), group: 'appearance', icon: 'sun' },
    { id: 'agent-n1/a1', label: 'Open claude · amber', run: () => run('agent'), group: 'agents', agent: { accent: 'var(--agent-amber)', kind: 'claude' } },
  ];
  const selected = () => $$('[role="option"][aria-selected="true"]').map((o) => o.querySelector('.palette__label')?.textContent);
  const input = () => $<HTMLInputElement>('input[role="combobox"]');

  it('groups the results under headings in a fixed order, with an icon or the agent avatar and the real shortcut', () => {
    render(<CommandPalette items={items()} onClose={() => {}} />);
    const groups = $$('[role="listbox"] > [role="group"]');
    // Agents come second although they are listed last (after the themes), as the headings' order says.
    expect(groups.map((g) => document.getElementById(g.getAttribute('aria-labelledby') ?? '')?.textContent)).toEqual(['Go to', 'Agents', 'Actions', 'Appearance']);
    const agent = $$('[role="option"]').find((o) => o.textContent?.includes('claude')) as HTMLElement;
    expect(agent.querySelector('.kit-avatar')?.textContent).toBe('C');
    expect(agent.querySelector('.palette__icon')).toBeNull();
    const next = document.getElementById('palette-next') as HTMLElement;
    expect(next.querySelector('.palette__icon svg')).toBeTruthy();
    expect(next.getAttribute('aria-keyshortcuts')).toBe('N');
    expect(next.querySelector('kbd')?.textContent).toBe('N');
    expect($$('[role="option"] kbd')).toHaveLength(1); // only shortcuts that exist
    expect($('.palette__foot').textContent).toContain(t('palette-key-move'));
  });

  it('arrows move through the rows across group headings; Enter runs the highlighted row and closes', () => {
    const run = vi.fn();
    const onClose = vi.fn();
    render(<CommandPalette items={items(run)} onClose={onClose} />);
    expect(selected()).toEqual(['Next that needs me']);
    key(input(), 'ArrowDown');
    key(input(), 'ArrowDown'); // from "Go to" into "Agents"
    expect(selected()).toEqual(['Open claude · amber']);
    expect(input().getAttribute('aria-activedescendant')).toBe('palette-agent-n1/a1');
    key(input(), 'ArrowDown');
    key(input(), 'ArrowDown');
    key(input(), 'ArrowDown'); // stops at the last row
    expect(selected()).toEqual(['Theme: Dark']);
    key(input(), 'ArrowUp');
    expect(selected()).toEqual(['Start an agent']);
    key(input(), 'Enter');
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(run).toHaveBeenCalledWith('start');
  });

  it('a query regroups the results; arrows and Enter follow the rows as shown, not the raw ranking', () => {
    const run = vi.fn();
    const three: PaletteItem[] = [
      { id: 'b', label: 'Open beta two', run: () => run('b'), group: 'go' },
      { id: 'a', label: 'Open alpha', run: () => run('a'), group: 'agents' },
      { id: 'c', label: 'Reopen gamma', run: () => run('c'), group: 'agents' },
    ];
    render(<CommandPalette items={three} onClose={() => {}} />);
    type(input(), 'open'); // ranked a, b, c; shown as Agents (a, c) then Go to (b)
    expect($$('[role="option"]').map((o) => o.id)).toEqual(['palette-a', 'palette-c', 'palette-b']);
    key(input(), 'ArrowDown');
    expect(selected()).toEqual(['Reopen gamma']);
    key(input(), 'Enter');
    expect(run).toHaveBeenCalledWith('c');
  });

  it('says when nothing matches, and Enter then does nothing', () => {
    const run = vi.fn();
    const onClose = vi.fn();
    render(<CommandPalette items={items(run)} onClose={onClose} />);
    type(input(), 'zzzz');
    expect($$('[role="option"]')).toHaveLength(0);
    expect($('[role="listbox"]').hidden).toBe(true);
    expect($('.kit-empty .kit-empty__title').textContent).toBe(t('palette-empty'));
    expect($('.kit-empty .kit-empty__hint').textContent).toBe(t('palette-empty-hint'));
    expect(input().getAttribute('aria-activedescendant')).toBeNull();
    key(input(), 'Enter');
    expect(run).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('keeps each group where its best match ranks once there is a query, the fixed order without one; ungrouped entries are actions', () => {
    const a = (id: string, group?: PaletteItem['group']): PaletteItem => ({ id, label: id, run: () => {}, group });
    const ranked = [a('t', 'appearance'), a('x', 'agents'), a('y'), a('z', 'agents'), a('w', 'go')];
    const shape = (g: ReturnType<typeof groupPalette>) => g.map((x) => [x.group, x.items.map((i) => i.id)]);
    expect(shape(groupPalette(ranked))).toEqual([['appearance', ['t']], ['agents', ['x', 'z']], ['actions', ['y']], ['go', ['w']]]);
    expect(shape(groupPalette(ranked, true))).toEqual([['go', ['w']], ['agents', ['x', 'z']], ['actions', ['y']], ['appearance', ['t']]]);
  });

  it('speaks Turkish', () => {
    setLanguage('tr');
    render(<CommandPalette items={items()} onClose={() => {}} />);
    expect($$('.palette__heading').map((h) => h.textContent)).toEqual(['Şuraya git', 'Ajanlar', 'Eylemler', 'Görünüm']);
  });
});

describe('sign-in card', () => {
  const visibleProse = () => {
    const card = $('.signin__card').cloneNode(true) as HTMLElement;
    card.querySelectorAll('code, select').forEach((c) => c.remove()); // the command is typed as it is; the language names are names
    return card.textContent ?? '';
  };

  it('asks for a code: the mark and the title, the command to copy, a mono code field, Sign in, a divider, the passkey, the language', () => {
    render(<SignInPage onSubmit={() => {}} onPasskey={() => {}} />);
    expect($('main.signin > .signin__card')).toBeTruthy();
    expect($('h1').textContent).toBe('Sign in to your Agora');
    expect($('.signin__mark').getAttribute('aria-hidden')).toBe('true');
    expect($('form .kit-copy code').textContent).toBe('agora hub login');
    expect($$('input:not([type])')).toHaveLength(1); // the one text box (browser tests fill it by role)
    expect($('input.signin__code').getAttribute('autocomplete')).toBe('off');
    expect($('form button[type="submit"]').textContent).toBe('Sign in');
    expect($('.signin__or').textContent).toBe('or');
    expect(button('Use a passkey')).toBeTruthy();
    expect($('.signin__foot select')).toBeTruthy();
    expect($('.signin__foot label')?.textContent).toContain('Language');
    expect(visibleProse()).not.toMatch(/\bhub\b/i);
  });

  it('sends the code (and the screen password when asked) exactly as before', () => {
    const onSubmit = vi.fn();
    render(<SignInCard onSubmit={onSubmit} passwordRequired initialCode="AB12-CD34" />);
    act(() => $<HTMLFormElement>('form').requestSubmit());
    expect(onSubmit).not.toHaveBeenCalled(); // no password yet
    type($<HTMLInputElement>('input[type="password"]'), 'secret');
    act(() => $<HTMLFormElement>('form').requestSubmit());
    expect(onSubmit).toHaveBeenCalledWith('AB12-CD34', 'secret');
    expect($('.signin__hint').textContent).toBe(t('signin-password-hint'));
    expect(visibleProse()).not.toMatch(/\bhub\b/i);
  });

  it('shows an error as a danger callout that is announced', () => {
    render(<SignInCard onSubmit={() => {}} notice={t('signin-refused')} />);
    const alert = $('[role="alert"]');
    expect(alert.className).toContain('kit-tone-danger');
    expect(alert.textContent).toBe(t('signin-refused'));
  });

  it('while it waits: the number to compare, no form, a code one click away, the passkey still there', () => {
    const onPasskey = vi.fn();
    render(<SignInCard onSubmit={() => {}} onPasskey={onPasskey} approval={{ number: '731004', state: 'waiting' }} />);
    expect($('.signin-wait__digits').textContent).toBe('731-004');
    expect($('.signin-wait').getAttribute('aria-live')).toBe('polite');
    expect($('form')).toBeNull();
    expect(button(t('signin-ask-again'))).toBeUndefined();
    act(() => button('Use a passkey').click());
    expect(onPasskey).toHaveBeenCalledTimes(1);
    act(() => button(t('signin-use-code')).click());
    expect($('form input.signin__code')).toBeTruthy();
  });

  it('refused: a danger callout and Ask again; out of time: a neutral one; the old number is gone', () => {
    const onAskAgain = vi.fn();
    render(<SignInCard onSubmit={() => {}} approval={{ number: '731004', state: 'denied' }} onAskAgain={onAskAgain} />);
    expect($('[role="alert"]').className).toContain('kit-tone-danger');
    expect($('[role="alert"]').textContent).toBe(t('signin-denied'));
    expect(document.body.textContent).not.toContain('731-004');
    expect(button(t('signin-ask-again')).dataset.variant).toBe('primary');
    act(() => button(t('signin-ask-again')).click());
    expect(onAskAgain).toHaveBeenCalledTimes(1);
    render(<SignInCard onSubmit={() => {}} approval={{ number: '731004', state: 'expired' }} />);
    expect($('[role="alert"]').className).toContain('kit-tone-accent');
    expect($('[role="alert"]').textContent).toBe(t('signin-expired'));
  });

  it('speaks Turkish without the word hub', () => {
    setLanguage('tr');
    render(<SignInCard onSubmit={() => {}} onPasskey={() => {}} passwordRequired />);
    expect($('h1').textContent).toBe('Agora’nıza giriş yapın');
    expect(button('Geçiş anahtarı kullan')).toBeTruthy();
    expect(visibleProse()).not.toMatch(/\bhub/i);
  });
});
