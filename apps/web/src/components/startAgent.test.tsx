// @vitest-environment jsdom
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage, t } from '../i18n';
import type { DirectoryListing } from '../net/directory';
import { CommandError } from '../net/live';
import type { MachineModel } from '../net/view';
import { StartAgent, refusalWords, type KindChoice } from './StartAgent';
import { missingNote } from './startChoice';

// The start window with the widget kit (redesign PR 10): agent cards, the machine as a segmented control, "How it acts" with its red risk
// state and its confirmation (ADR-0059), the not-installed note only for the chosen kind (finding L9), and the name preview.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;
beforeEach(() => { setLanguage('en'); localStorage.clear(); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

const machines: MachineModel[] = [
  { id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 0 },
  { id: 'n2', name: 'mini', os: 'macos', online: true, agentsRunning: 0, lastSeenMs: 0 },
  { id: 'n3', name: 'box', os: 'plan9', online: true, agentsRunning: 0, lastSeenMs: 0 },
];
const skip = { id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true };
const KINDS: KindChoice[] = [
  { kind: 'codex', name: 'Codex', installed: true, level: 2, options: [skip] },
  { kind: 'gemini', name: 'Gemini CLI', installed: false, level: 2, program: 'gemini' },
  { kind: 'claude', name: 'Claude Code', installed: true, level: 3 },
  { kind: 'old', name: 'Old agent', installed: false },
];
type OnStart = Parameters<typeof StartAgent>[0]['onStart'];

const flush = () => act(async () => { await Promise.resolve(); await Promise.resolve(); });
const win = () => document.querySelector('[role=dialog][aria-label="Start an agent"]') as HTMLElement;
const button = (text: string) => [...win().querySelectorAll('button')].find((b) => b.textContent?.trim() === text) as HTMLButtonElement | undefined;
const click = (el: Element | null | undefined) => { if (!el) throw new Error('missing element'); act(() => { (el as HTMLElement).click(); }); };
const key = (el: Element, k: string) => act(() => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true })); });
const group = (name: string) => win().querySelector(`[role=radiogroup][aria-label="${name}"]`) as HTMLElement;
const radios = (name: string) => [...group(name).querySelectorAll<HTMLButtonElement>('[role=radio]')];
const checked = (name: string) => radios(name).find((r) => r.getAttribute('aria-checked') === 'true');
const card = (name: string) => radios('Which agent').find((r) => r.getAttribute('aria-label') === name) as HTMLButtonElement;

async function open(over: Partial<Parameters<typeof StartAgent>[0]> = {}) {
  const onStart = vi.fn<OnStart>(async () => undefined);
  act(() => root.render(<StartAgent projects={[{ node: 'n1', projectId: 'p', label: 'shop', path: '/w/shop' }]} machines={machines}
    loadKinds={async () => KINDS} inspect={async (_n, path) => ({ path, projectId: 'p', gitRoot: path, branch: 'main', portable: false })}
    onStart={onStart} onClose={() => {}} {...over} />));
  await flush();
  click(button('Use this folder'));
  await flush();
  return onStart;
}

describe('the start window, agent cards', () => {
  it('lists installed kinds first, then the ones the machine cannot find, each a card named by its name with Terminal or Chat', async () => {
    await open();
    expect(radios('Which agent').map((r) => r.getAttribute('aria-label'))).toEqual(['Codex', 'Claude Code', 'Gemini CLI', 'Old agent']);
    const text = group('Which agent').textContent ?? '';
    expect(text.indexOf('Installed')).toBeLessThan(text.indexOf('Codex'));
    expect(text.indexOf('Not installed')).toBeGreaterThan(text.indexOf('Claude Code'));
    // How it runs and "not installed" are the card's description, never its name.
    const said = (r: HTMLElement) => document.getElementById(r.getAttribute('aria-describedby') ?? '')?.textContent;
    expect(said(card('Codex'))).toBe('Terminal');
    expect(said(card('Claude Code'))).toBe('Chat');
    expect(said(card('Gemini CLI'))).toBe('Terminalnot installed');
    expect(card('Gemini CLI').className).toContain('border-dashed');
    expect(card('Codex').querySelector('.kit-avatar')?.textContent).toBe('C');
    // The first installed kind is chosen, as before; the chosen card is the accent-ringed kit card.
    expect(checked('Which agent')).toBe(card('Codex'));
    expect(card('Codex').hasAttribute('data-selected')).toBe(true);
  });

  it('puts installed ACP registry kinds in their own Untested group, leaves out the ones it cannot find and never preselects one (ADR-0092)', async () => {
    const kinds: KindChoice[] = [
      { kind: 'registry-amp', name: 'Amp', installed: true, level: 3, program: 'npx' },
      ...KINDS,
      { kind: 'registry-goose', name: 'goose', installed: false, level: 3, program: 'goose' },
    ];
    await open({ loadKinds: async () => kinds });
    expect(radios('Which agent').map((r) => r.getAttribute('aria-label'))).toEqual(['Codex', 'Claude Code', 'Amp', 'Gemini CLI', 'Old agent']);
    const text = group('Which agent').textContent ?? '';
    expect(text.indexOf('Claude Code')).toBeLessThan(text.indexOf('Untested'));
    expect(text.indexOf('Untested')).toBeLessThan(text.indexOf('Amp'));
    expect(card('Amp').textContent).toContain('Untested');
    expect(checked('Which agent')?.getAttribute('aria-label')).toBe('Codex');
    // Only registry kinds: still nothing untested is chosen for the person.
    act(() => root.unmount()); root = createRoot(host);
    await open({ loadKinds: async () => [kinds[0]!] });
    expect(checked('Which agent')).toBeUndefined();
  });

  it('has no group labels when every kind is installed', async () => {
    await open({ loadKinds: async () => KINDS.filter((k) => k.installed) });
    expect(group('Which agent').textContent).not.toContain('Installed');
  });

  it('makes agent kind display names visible while retaining the raw kind id', async () => {
    await open({ loadKinds: async () => [{ kind: 'raw-kind', name: 'Grü\u3164ße\u202e', installed: true, level: 2 }] });
    const choice = radios('Which agent')[0] as HTMLButtonElement;
    expect(choice.getAttribute('aria-label')).toBe('Grü\\u{3164}ße\\u{202e}');
    expect(choice.textContent).toContain('Grü\\u{3164}ße\\u{202e}');
    expect(choice.dataset.kind).toBe('raw-kind');
    expect([...choice.textContent ?? ''].filter((c) => ['\u3164', '\u202e'].includes(c))).toEqual([]);
  });

  it('verifies visible kind names while starting the raw kind id', async () => {
    const onStart = await open({ loadKinds: async () => [{ kind: 'raw-kind', name: 'İş\u2066\u0001', installed: true, level: 2 }] });
    const choice = radios('Which agent')[0] as HTMLButtonElement;
    expect(choice.getAttribute('aria-label')).toBe('İş\\u{2066}\\u{1}');
    click(button('Start')); await flush();
    expect(onStart).toHaveBeenCalledWith('n1', expect.anything(), 'raw-kind', false, false, expect.anything());
  });

  it('is chosen by keyboard: one tab stop, the arrows move and choose (installed into not installed), Home and End go to the ends', async () => {
    await open();
    expect(radios('Which agent').map((r) => r.tabIndex)).toEqual([0, -1, -1, -1]);
    key(card('Codex'), 'ArrowRight');
    expect(checked('Which agent')).toBe(card('Claude Code'));
    expect(document.activeElement).toBe(card('Claude Code'));
    key(card('Claude Code'), 'ArrowDown');
    expect(checked('Which agent')).toBe(card('Gemini CLI'));
    key(card('Gemini CLI'), 'End');
    expect(checked('Which agent')).toBe(card('Old agent'));
    key(card('Old agent'), 'ArrowRight'); // wraps
    expect(checked('Which agent')).toBe(card('Codex'));
    key(card('Codex'), 'ArrowLeft');
    expect(checked('Which agent')).toBe(card('Old agent'));
    key(card('Old agent'), 'Home');
    expect(checked('Which agent')).toBe(card('Codex'));
    expect(radios('Which agent').map((r) => r.tabIndex)).toEqual([0, -1, -1, -1]);
    group('Which agent').style.direction = 'rtl';
    key(card('Codex'), 'ArrowLeft'); // right to left: left is forward
    expect(checked('Which agent')).toBe(card('Claude Code'));
  });
});

describe('the start window, a kind the machine cannot find', () => {
  it('says why only when that kind is chosen, offers nothing for it and never starts it', async () => {
    const onStart = await open();
    const note = () => win().querySelector('[data-start-missing]');
    expect(note()).toBeNull();
    expect(win().textContent).not.toContain('cannot find');
    click(card('Gemini CLI'));
    expect(note()?.textContent).toBe(t('start-not-installed-program', { program: 'gemini' }));
    expect(button('Start')?.disabled).toBe(true);
    expect(win().querySelector('[data-start-preview]')).toBeNull();
    expect(group('How it acts')).toBeNull();
    expect(win().textContent).not.toContain(t('start-separate-worktree'));
    click(button('Start'));
    await flush();
    expect(onStart).not.toHaveBeenCalled();
    // A kind from an older machine that does not name its program gets the general sentence.
    click(card('Old agent'));
    expect(note()?.textContent).toBe(t('start-not-installed-why'));
    click(card('Claude Code'));
    expect(note()).toBeNull();
    expect(button('Start')?.disabled).toBe(false);
  });

  it('makes missing program names visible', async () => {
    await open({ loadKinds: async () => [{ kind: 'broken', name: 'Broken', installed: false, level: 2, program: 'ge\u202em\u3164ini' }] });
    click(card('Broken'));
    const note = win().querySelector('[data-start-missing]') as HTMLElement;
    expect(note.textContent).toContain('ge\\u{202e}m\\u{3164}ini');
    expect([...note.textContent].filter((c) => ['\u202e', '\u3164'].includes(c))).toEqual([]);
  });

  it('verifies missing program text in selection and runtime refusal paths', () => {
    const program = 'ça\u034f\ue000';
    expect(missingNote({ installed: false, program })).toEqual(['start-not-installed-program', { program: 'ça\\u{34f}\\u{e000}' }]);
    expect(refusalWords('start-program-missing', program)).toEqual(['start-not-installed-program', { program: 'ça\\u{34f}\\u{e000}' }]);
  });
});

describe('the start window, how it acts (ADR-0059, ADR-0102)', () => {
  it('starts on Ask; Don\'t ask only opens the confirmation, and turns the control and its note red once confirmed', async () => {
    const onStart = await open();
    const control = () => group('How it acts');
    expect(checked('How it acts')?.textContent).toBe('Ask before acting');
    expect(control().className).not.toContain('kit-seg--risk');
    click(radios('How it acts')[1]);
    // Not chosen yet: the confirmation is open, Keep asking has the focus, Start waits, nothing is red.
    const confirm = win().querySelector('[role=alertdialog]') as HTMLElement;
    expect(confirm.getAttribute('aria-label')).toBe(t('session-confirm-title'));
    expect(checked('How it acts')?.textContent).toBe('Ask before acting');
    expect(control().className).not.toContain('kit-seg--risk');
    expect(document.activeElement?.textContent).toBe(t('session-confirm-no'));
    expect(button('Start')?.disabled).toBe(true);
    expect(win().querySelector('[data-start-risk]')).toBeNull();
    // The answer that lets it act freely is the red button.
    expect(button(t('start-confirm-skip-yes'))?.className).toContain('kit-btn--danger');
    click(button(t('session-confirm-no')));
    expect(win().querySelector('[role=alertdialog]')).toBeNull();
    expect(checked('How it acts')?.textContent).toBe('Ask before acting');
    // The arrow keys open the same confirmation; they never choose Don't ask on their own.
    key(radios('How it acts')[0]!, 'ArrowRight');
    expect(win().querySelector('[role=alertdialog]')).not.toBeNull();
    expect(checked('How it acts')?.textContent).toBe('Ask before acting');
    click(button(t('start-confirm-skip-yes')));
    expect(checked('How it acts')?.textContent).toBe("Don't ask");
    expect(control().className).toContain('kit-seg--risk');
    const note = win().querySelector('[data-start-risk]') as HTMLElement;
    expect(note.textContent).toBe(t('start-does-not-ask-note'));
    expect(note.className).toContain('text-danger');
    click(button('Start'));
    await flush();
    expect(onStart).toHaveBeenCalledTimes(1);
    expect(onStart.mock.calls[0]![5]).toEqual({ ids: ['skip-permissions'], confirmed: ['skip-permissions'], settings: [] });
  });

  it('sends nothing that skips permissions without the confirmation, and a new kind starts over on Ask', async () => {
    const onStart = await open();
    click(radios('How it acts')[1]);
    click(button(t('start-confirm-skip-yes')));
    click(card('Claude Code'));
    click(card('Codex'));
    expect(checked('How it acts')?.textContent).toBe('Ask before acting');
    click(button('Start'));
    await flush();
    expect(onStart.mock.calls[0]![5]).toEqual({ ids: [], confirmed: [], settings: [] });
  });
});

describe('the start window, machine, preview and errors', () => {
  it('shows the machines as a segmented control with their system, named by the machine alone', async () => {
    await open();
    expect(radios('Machine').map((r) => r.textContent)).toEqual(['laptop', 'mini', 'box']);
    expect(checked('Machine')?.textContent).toBe('laptop');
    // Each has an icon (a system this build has no outline for gets the plain machine).
    for (const r of radios('Machine')) expect(r.querySelector('svg')).not.toBeNull();
    const paths = radios('Machine').map((r) => r.querySelector('path')?.getAttribute('d'));
    expect(new Set(paths).size).toBe(3);
  });

  it('previews the new agent\'s name with its avatar, by its kind, and says its colour comes when it starts', async () => {
    await open();
    const preview = () => win().querySelector('[data-start-preview]') as HTMLElement | null;
    expect(preview()?.textContent).toBe('CWill appear as codex · …, with a colour picked when it starts.');
    expect(preview()?.querySelector('strong')?.textContent).toBe('codex · …');
    expect(preview()?.querySelector('.kit-avatar')).not.toBeNull();
    click(card('Claude Code'));
    expect(preview()?.querySelector('strong')?.textContent).toBe('claude · …');
    // The words follow the language (the window re-renders on the next change); the name keeps its order inside the sentence.
    setLanguage('tr');
    click(document.querySelector('[data-kind="codex"]'));
    expect(document.querySelector('[role=dialog] [data-start-preview]')?.textContent).toBe('Ccodex · … olarak görünecek; rengi başlarken seçilir.');
  });

  it('makes the raw kind id visible in the agent name preview', async () => {
    const raw = 'co\u3164dex\u202e';
    await open({ loadKinds: async () => [{ kind: raw, name: 'Codex', installed: true, level: 2 }] });
    const preview = win().querySelector('[data-start-preview]') as HTMLElement;
    expect(preview.querySelector('strong')?.textContent).toBe('co\\u{3164}dex\\u{202e} · …');
    expect(preview.textContent).not.toContain('\u3164');
    expect(preview.textContent).not.toContain('\u202e');
  });

  it('verifies the visible preview while starting with the raw kind id', async () => {
    const raw = 'cla\u2066ude\ue000';
    const onStart = await open({ loadKinds: async () => [{ kind: raw, name: 'Claude', installed: true, level: 3 }] });
    const preview = win().querySelector('[data-start-preview]') as HTMLElement;
    expect(preview.querySelector('strong')?.textContent).toBe('cla\\u{2066}ude\\u{e000} · …');
    click(button('Start')); await flush();
    expect(onStart).toHaveBeenCalledWith('n1', expect.anything(), raw, false, false, expect.anything());
  });

  it('shows a refused start in red, and still offers the device approval ask (ADR-0061)', async () => {
    const ask = vi.fn(async () => false);
    await open({ onStart: async () => 'approval-not-trusted', trust: { id: 'd', key: 'k' }, askApproval: { can: () => true, ask } });
    click(button('Start'));
    await flush();
    const alert = win().querySelector('[role=alert]') as HTMLElement;
    expect(alert.textContent).toBe(t('trust-hint', { machine: 'laptop' }));
    expect(alert.className).toContain('text-danger');
    click(button(t('trust-ask')));
    await flush();
    expect(ask).toHaveBeenCalledTimes(1);
    const failed = [...win().querySelectorAll('[role=alert]')].find((a) => a.textContent === t('device-ask-failed'));
    expect(failed?.className).toContain('text-danger');
    expect(win().textContent).toContain('agora node trust --from-hub');
  });
});

describe('StartAgent', () => {
  it('makes board task title and project visible', async () => {
    await open({ forTask: { projectId: 'p', project: 'Pr\u2800oject\u2066', title: 'Ta\u3164sk\u034f' } });
    const note = win().querySelector('[data-start-for-task]') as HTMLElement;
    for (const shown of ['Pr\\u{2800}oject\\u{2066}', 'Ta\\u{3164}sk\\u{34f}']) expect(note.textContent).toContain(shown);
    expect([...note.textContent].filter((c) => ['\u2800', '\u2066', '\u3164', '\u034f'].includes(c))).toEqual([]);
  });

  it('verifies visible board task text in the other-project warning', async () => {
    await open({ forTask: { projectId: 'other', project: 'İş\u202e\ue000', title: '模型\u2066\u000b' } });
    expect(win().querySelector('[data-start-for-task]')?.textContent).toContain('模型\\u{2066}\\u{b}');
    const warning = win().querySelector('[role=alert]') as HTMLElement;
    expect(warning.textContent).toContain('İş\\u{202e}\\u{e000}');
    expect([...win().textContent ?? ''].filter((c) => ['\u202e', '\ue000', '\u2066', '\u000b'].includes(c))).toEqual([]);
  });

  it('makes folder-browser paths and entries visible', async () => {
    await act(async () => {
      root.render(<StartAgent projects={[]} machines={machines.slice(0, 1)} initialNode="n1" loadKinds={() => Promise.resolve([])}
        inspect={() => Promise.reject(new Error('unused'))}
        list={() => Promise.resolve({ path: '/Grü\u3164ße', parent: '/', entries: [{ name: 'ça\u2800lış', isGit: false }], truncated: false })}
        onStart={() => Promise.resolve(undefined)} onClose={() => {}} />);
    });
    const browseBtn = Array.from(document.body.querySelectorAll('button')).find((b) => b.textContent?.includes(t('start-browse')));
    await act(async () => { browseBtn!.click(); });
    const shown = win().textContent ?? '';
    expect([...shown].some((c) => ['\u3164', '\u2800'].includes(c))).toBe(false);
    expect(shown).toContain('/Grü\\u{3164}ße');
    expect(shown).toContain('ça\\u{2800}lış');
  });
  it('verifies folder display while navigating with the raw path', async () => {
    const list = vi.fn(async () => ({ path: '/Grü\u3164ße', parent: '/', entries: [{ name: '漢\u202e字', isGit: false }], truncated: false }));
    await act(async () => {
      root.render(<StartAgent projects={[]} machines={machines.slice(0, 1)} initialNode="n1" loadKinds={() => Promise.resolve([])} inspect={() => Promise.reject(new Error('unused'))}
        list={list} onStart={() => Promise.resolve(undefined)} onClose={() => {}} />);
    });
    const browseBtn = Array.from(document.body.querySelectorAll('button')).find((b) => b.textContent?.includes(t('start-browse')));
    await act(async () => { browseBtn!.click(); });
    expect(win().textContent).toContain('Grü\\u{3164}ße');
    const folder = Array.from(win().querySelectorAll('button')).find((b) => b.textContent?.includes('漢\\u{202e}字'));
    await act(async () => { folder!.click(); });
    expect(list).toHaveBeenLastCalledWith('n1', '/Grü\u3164ße/漢\u202e字');
  });

  it('ignores stale directory listings from a previous machine after switching', async () => {
    const machines: MachineModel[] = [
      { id: 'n1', name: 'Node 1', online: true, os: 'linux', binary: '', agentsRunning: 0, terminalsOpen: 0, memoryBytes: 0, lastSeenMs: 0 },
      { id: 'n2', name: 'Node 2', online: true, os: 'linux', binary: '', agentsRunning: 0, terminalsOpen: 0, memoryBytes: 0, lastSeenMs: 0 },
    ];
    let resolveN1!: (v: DirectoryListing) => void;
    const list = (node: string) =>
      node === 'n1'
        ? new Promise<DirectoryListing>((r) => { resolveN1 = r; })
        : Promise.resolve({ path: '/n2-home', parent: '/', entries: [{ name: 'n2-folder', isGit: false }], truncated: false });

    await act(async () => {
      root.render(
        <StartAgent
          projects={[]}
          machines={machines}
          initialNode="n1"
          loadKinds={() => Promise.resolve([])}
          inspect={() => Promise.reject(new Error('unused'))}
          list={list}
          onStart={() => Promise.resolve(undefined)}
          onClose={() => {}}
        />,
      );
    });

    const browseBtn = Array.from(document.body.querySelectorAll('button')).find((b) => b.textContent?.includes(t('start-browse')));
    await act(async () => { browseBtn!.click(); });

    // The machine is a segment of the Machine control, named by the machine alone.
    const n2Btn = radios('Machine').find((b) => b.textContent === 'Node 2');
    await act(async () => { n2Btn!.click(); });

    await act(async () => {
      resolveN1({ path: '/n1-stale-home', parent: '/', entries: [{ name: 'n1-stale-folder', isGit: false }], truncated: false });
    });

    expect(document.body.textContent ?? '').not.toContain('n1-stale-folder');
  });

  it('says the person\'s role when the hub refused the folder picker for it, and the device sentence only for a device (#1044)', async () => {
    const machines: MachineModel[] = [{ id: 'n1', name: 'Node 1', online: true, os: 'linux', binary: '', agentsRunning: 0, terminalsOpen: 0, memoryBytes: 0, lastSeenMs: 0 }];
    const shown = async (refusal: Error) => {
      await act(async () => {
        root.render(
          <StartAgent key={refusal.message + JSON.stringify((refusal as CommandError).params ?? {})} projects={[]} machines={machines} initialNode="n1" loadKinds={() => Promise.resolve([])}
            inspect={() => Promise.reject(new Error('unused'))} list={() => Promise.reject(refusal)} onStart={() => Promise.resolve(undefined)} onClose={() => {}} />,
        );
      });
      const browseBtn = Array.from(document.body.querySelectorAll('button')).find((b) => b.textContent?.includes(t('start-browse')));
      await act(async () => { browseBtn!.click(); });
      return document.body.querySelector('[role="alert"]')?.textContent ?? '';
    };
    // The hub: this person's role may not look into folders (ADR-0049), named by the refused command.
    expect(await shown(new CommandError('auth.forbidden', { action: 'DirectoryList' }))).toBe(t('start-directory-role'));
    // The machine: this device is limited to other projects.
    expect(await shown(new CommandError('auth.forbidden', { action: 'directory' }))).toBe(t('start-directory-forbidden'));
    expect(await shown(new CommandError('node.offline', { node_id: 'n1' }))).toBe(t('start-browse-failed'));
  });
});
