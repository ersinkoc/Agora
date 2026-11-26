import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ReactElement } from 'react';
import { setLanguage, t } from './i18n';
import { EXIT_UNKNOWN, endedWithError, sharesName, shownExitCode, type AgentTileModel } from './model';
import { BUILT_IN, contrast } from './themes/tokens';
import { byFolder, firstVisible, parseDiff, type DiffFile } from './net/diff';
import { ClosedSummary } from './components/ClosedSummary';
import { StartApproval } from './components/StartApproval';
import { StateBadge } from './components/StateBadge';
import { ProjectBoard } from './components/ProjectBoard';
import { NoAskBadge } from './components/NoAskBadge';
import { demoStartApproval } from './fixtures';
import { PRESETS, reduce, workspaceOf } from './layout/engine';
import { PICTOGRAM_BOX, pictogram, presetOf } from './layout/preset';
import { defaultLayout, panesOf } from './layout/tree';

const read = (p: string) => readFileSync(p, 'utf8').replace(/\r\n/g, '\n');
const closed = (over: Partial<AgentTileModel>): AgentTileModel => ({ id: 'x', name: 'claude · olive', accent: '#fff', machine: 'm', workspace: 'w', state: 'finished', preview: '', closed: true, ...over });

describe('quick win: a failed agent is never drawn as success', () => {
  // The values the node really reports (crates/agora-node/src/daemon.rs): the program's status, -signal, -1, and i32::MIN for a stop or
  // an unknown status.
  it('is a failure only for a genuine non-zero exit status or a crash', () => {
    for (const code of [1, 2, 127, 137, 143]) expect(endedWithError(closed({ exitCode: code })), String(code)).toBe(true);
    for (const signal of [4, 6, 7, 8, 11]) expect(endedWithError(closed({ exitCode: -signal })), `-${signal}`).toBe(true); // it crashed
  });
  it('is a failure for a Windows crash, which arrives as an NTSTATUS cast to i32', () => {
    const asI32 = (u: number) => u | 0;
    expect(asI32(0xc0000005)).toBe(-1073741819);
    for (const status of [0xc0000005, 0xc0000409, 0xc00000fd, 0xc0000374, 0xe06d7363, 0xc0000142]) expect(endedWithError(closed({ exitCode: asI32(status) })), status.toString(16)).toBe(true);
    expect(endedWithError(closed({ exitCode: -1073741819 }))).toBe(true);
    expect(endedWithError(closed({ exitCode: 2147483647 }))).toBe(true);
    expect(shownExitCode({ exitCode: asI32(0xc0000005) })).toBeUndefined(); // no negative number is shown
  });
  it('is not a failure for Ctrl+C on Windows (0xC000013A), nor for the edges of the range that are signals or a stop', () => {
    expect(endedWithError(closed({ exitCode: 0xc000013a | 0 }))).toBe(false);
    expect(endedWithError(closed({ exitCode: -1073741510 }))).toBe(false);
    for (const code of [EXIT_UNKNOWN, -1, -15, -0x10000000, 0xfffffff1 | 0, 0x80000001 | 0]) expect(endedWithError(closed({ exitCode: code })), String(code)).toBe(false);
    expect(endedWithError(closed({ exitCode: 0xc0000005 | 0, interrupted: { resumable: true } }))).toBe(false);
    expect(endedWithError({ closed: false, exitCode: 0xc0000005 | 0 })).toBe(false);
  });
  it('is not a failure for a stop, an unknown status, a signal the node sends itself, a clean exit or a running agent', () => {
    expect(endedWithError(closed({ exitCode: EXIT_UNKNOWN }))).toBe(false); // stopped on request, or never known
    for (const signal of [15, 9, 1, 2]) expect(endedWithError(closed({ exitCode: -signal })), `-${signal}`).toBe(false); // TERM, KILL, HUP, INT
    expect(endedWithError(closed({ exitCode: -1 }))).toBe(false);
    expect(endedWithError(closed({ exitCode: 0 }))).toBe(false);
    expect(endedWithError(closed({}))).toBe(false);
    expect(endedWithError({ closed: false, exitCode: 1 })).toBe(false); // still running
    expect(endedWithError(closed({ exitCode: 1, interrupted: { resumable: true } }))).toBe(false); // stopped with its machine: its own wording
    expect(endedWithError(closed({ exitCode: EXIT_UNKNOWN, interrupted: { resumable: false } }))).toBe(false);
  });
  it('shows an exit code only when it is the program\'s own', () => {
    expect(shownExitCode({ exitCode: 3 })).toBe(3);
    for (const c of [0, -1, -15, EXIT_UNKNOWN, undefined]) expect(shownExitCode({ exitCode: c }), String(c)).toBeUndefined();
  });
  const summary = (a: AgentTileModel) => renderToStaticMarkup(<ClosedSummary agent={a} now={Date.now()} onDismiss={() => {}} />);
  beforeEach(() => setLanguage('en'));
  it('says it ended with an error, with a cross, in the neutral colour', () => {
    const html = summary(closed({ exitCode: 1 }));
    expect(html).toContain('Ended with an error (exit code 1)');
    expect(html).toContain('state--failed');
    expect(html).not.toContain('text-finished');
    expect(html).toContain('M4 4l4 4M8 4l-4 4'); // the cross
  });
  it('a stopped agent is Finished, with no error and no number', () => {
    const html = summary(closed({ exitCode: EXIT_UNKNOWN }));
    expect(html).not.toContain('Ended with an error');
    expect(html).not.toContain('2147483648');
    expect(html).toContain('text-finished');
  });
  it('a crash by signal says it ended with an error, without a negative exit code', () => {
    const html = summary(closed({ exitCode: -11 }));
    expect(html).toContain('Ended with an error');
    expect(html).not.toContain('exit code');
  });
  it('negative control: a clean exit is still the Finished check', () => {
    const html = summary(closed({ exitCode: 0 }));
    expect(html).toContain('text-finished');
    expect(html).not.toContain('state--failed');
    expect(html).not.toContain('Ended with an error');
  });
  it('makes closed-agent identity and location labels visible', () => {
    const html = summary(closed({ name: 'clau\u3164de\u202e', machine: 'lap\u034ftop', workspace: 'wo\u2066rk', branch: 'fi\ue000x' }));
    expect(html).toContain('clau\\u{3164}de\\u{202e}');
    expect(html).toContain('lap\\u{34f}top');
    expect(html).toContain('wo\\u{2066}rk');
    expect(html).toContain('fi\\u{e000}x');
  });
  it('verifies closed-agent labels across isolate, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u2069', '\\u{2069}'], ['\u0007', '\\u{7}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const html = summary(closed({ name: `co${char}dex`, machine: `ma${char}chine`, workspace: `wo${char}rk`, branch: `br${char}anch` }));
      for (const [start, end] of [['co', 'dex'], ['ma', 'chine'], ['wo', 'rk'], ['br', 'anch']] as const) expect(html).toContain(`${start}${escaped}${end}`);
    }
  });
  it('a badge for a failed agent names the error in both languages', () => {
    expect(renderToStaticMarkup(<StateBadge state="finished" failed />)).toContain('Ended with an error');
    setLanguage('tr');
    expect(renderToStaticMarkup(<StateBadge state="finished" failed />)).toContain('Hatayla bitti');
    setLanguage('en');
  });
});

describe('quick win: review opens on the first visible file', () => {
  const diff = (path: string) => `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-a\n+b\n`;
  const files = (...paths: string[]): DiffFile[] => parseDiff(paths.map(diff).join(''));
  const secret = (list: DiffFile[], ...names: string[]) => list.map((f) => (names.includes(f.path) ? { ...f, secret: true } : f));
  it('skips a hidden file that would sort first', () => {
    const list = secret(files('.env', 'notes.md', 'src/a.ts'), '.env');
    expect(firstVisible(list)).toBe('notes.md');
    expect(byFolder(list)[0]?.files.map((f) => f.path)).toEqual(['notes.md', '.env']); // hidden last in its folder
  });
  it('puts a folder of hidden files after every other folder', () => {
    const list = secret(files('config/.env', 'src/a.ts'), 'config/.env');
    expect(byFolder(list).map((g) => g.folder)).toEqual(['src', 'config']);
    expect(firstVisible(list)).toBe('src/a.ts');
  });
  it('negative control: with nothing hidden the order is the usual one', () => {
    expect(firstVisible(files('b.ts', 'a.ts'))).toBe('a.ts');
    expect(byFolder(files('z/b.ts', 'a.ts')).map((g) => g.folder)).toEqual(['', 'z']);
  });
  it('opens on the first file when every file is hidden, and on nothing when there are none', () => {
    expect(firstVisible(secret(files('.env'), '.env'))).toBe('.env');
    expect(firstVisible([])).toBeUndefined();
  });
});

describe('quick win: state colours are readable as text on every surface', () => {
  const surfaces = ['surface-0', 'surface-1', 'surface-2'] as const;
  const states = ['state-working', 'state-needs-you', 'state-finished', 'state-idle', 'state-unknown'] as const;
  for (const b of BUILT_IN) {
    it(`${b.id}: every state token is at least 4.5:1 on every surface`, () => {
      for (const s of states) for (const bg of surfaces) expect(contrast(b.tokens[s], b.tokens[bg]), `${b.id} ${s} on ${bg}`).toBeGreaterThanOrEqual(4.5);
    });
  }
  it('the diff colour for removed lines reads on every surface and on its own tinted row, in both schemes', () => {
    const css = read('src/design/tokens.css');
    const removed = (scheme: 'dark' | 'light') => new RegExp(`${scheme === 'dark' ? ":root,\\n\\[data-theme='dark'\\]" : "\\[data-theme='light'\\]"} \\{[^}]*--diff-removed:\\s*(#[0-9a-f]{6})`).exec(css)?.[1] ?? '';
    const mix = (a: string, b: string, share: number) => `#${[1, 3, 5].map((i) => Math.round(parseInt(a.slice(i, i + 2), 16) * share + parseInt(b.slice(i, i + 2), 16) * (1 - share)).toString(16).padStart(2, '0')).join('')}`;
    for (const scheme of ['dark', 'light'] as const) {
      const red = removed(scheme);
      expect(red, scheme).toMatch(/^#[0-9a-f]{6}$/);
      const tokens = BUILT_IN.find((b) => b.id === scheme)!.tokens;
      for (const s of surfaces) {
        expect(contrast(red, tokens[s]), `${scheme} on ${s}`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(red, mix(red, tokens[s], 0.15)), `${scheme} on a removed row over ${s}`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
  it('amber is not used for deletions any more', () => {
    const view = read('src/components/ChangesView.tsx');
    expect(view).not.toMatch(/del[^\n]*needs-you|needs-you[^\n]*removed|deleted: 'text-needs-you'/);
    expect(view).toContain("del: 'bg-removed/12'");
  });
});

describe('quick win: 44px hit areas on touch', () => {
  const css = read('src/design/app.css');
  const block = /@media \(pointer: coarse\) \{([\s\S]*?)\n\}/.exec(css)?.[1] ?? '';
  it('buttons, tabs, rows, icon buttons, chips and inputs get --tap as their least height under a coarse pointer', () => {
    for (const sel of ['.button', '.ide-iconbtn', '.ide-tab', '.ide-row', '.menu-tile', 'button.ide-chip', '.ide-bar > a', '.ide-bar > button', '.touch-row', '.kit-btn', '.kit-row', '.kit-seg__item', 'button.kit-stat', '.kit-toggle::before']) expect(block, sel).toContain(sel);
    expect(block).toMatch(/min-height: var\(--tap\)/);
    expect(block).toMatch(/\.ide-iconbtn, \.menu-tile \{ min-width: var\(--tap\)/);
    expect(read('src/design/tokens.css')).toMatch(/--tap: 44px;/);
  });
  it('only on touch: nothing outside the coarse block sets that height', () => {
    expect(css.replace(block, '')).not.toContain('min-height: var(--tap)');
  });
});

describe('quick win: copy', () => {
  beforeEach(() => setLanguage('en'));
  const card = () => renderToStaticMarkup(<StartApproval approval={demoStartApproval} start={demoStartApproval.start} onResolve={() => {}} />);
  it('names the asking agent by its display name, with no id, no kind in brackets and no Depth', () => {
    const html = card();
    expect(html).toContain('claude · amber on Laptop asks to start a codex agent');
    expect(html).not.toMatch(/a1 \(claude\)/);
    expect(html).not.toContain('Depth');
  });
  it('never shows an id when a name is missing, and says so in words', () => {
    const bare = (over: object) => { const x = { ...demoStartApproval, ...over }; return renderToStaticMarkup(<StartApproval approval={x} start={x.start} onResolve={() => {}} />); };
    const noMachine = bare({ machineName: '' });
    expect(noMachine).toContain('claude · amber asks to start a codex agent');
    expect(noMachine).not.toMatch(/laptop|\ba1\b|demo/);
    const noAgent = bare({ agentName: '', machineName: 'Laptop' });
    expect(noAgent).toContain('An agent on Laptop asks to start a codex agent');
    expect(noAgent).not.toMatch(/\ba1\b/);
    const neither = bare({ agentName: '   ', machineName: undefined });
    expect(neither).toContain('An agent asks to start a codex agent');
    expect(neither).not.toMatch(/laptop|\ba1\b|demo/);
    setLanguage('tr');
    expect(bare({ agentName: '', machineName: '' })).toContain('Bir ajan, bir codex ajanı başlatmak istiyor');
  });
  it('shows that the asker is a helper only when it is one', () => {
    expect(card()).not.toContain('itself a helper');
    const deep = { ...demoStartApproval, start: { ...demoStartApproval.start, depth: 2 } };
    expect(renderToStaticMarkup(<StartApproval approval={deep} start={deep.start} onResolve={() => {}} />)).toContain('itself a helper');
  });
  it('renames the two labels in English and Turkish, with no internal term', () => {
    expect(t('agent-does-not-ask')).toBe('Runs without asking');
    expect(t('interrupted-section')).toBe('Stopped when the machine shut down');
    setLanguage('tr');
    expect(t('agent-does-not-ask')).toBe('Onay istemeden çalışır');
    expect(t('interrupted-section')).toBe('Makine kapanınca durdu');
    expect(t('start-approval-title', { name: 'claude · amber', machine: 'laptop', kind: 'codex' })).toBe('laptop üzerindeki claude · amber, bir codex ajanı başlatmak istiyor');
  });
});

describe('quick win: the safety badge is one sign and says all of it on demand', () => {
  beforeEach(() => setLanguage('en'));
  it('is a red bolt and no words; the whole meaning is its name, and it never looks like the quota chip', () => {
    const full = 'Runs without asking: Started so that it acts without asking you first';
    for (const html of [renderToStaticMarkup(<NoAskBadge agent={{ skipsPermissions: true }} />), renderToStaticMarkup(<NoAskBadge agent={{ skipsPermissions: true }} nested />)]) {
      expect(html).toContain(`aria-label="${full}"`);
      expect(html).toContain('role="img"');
      expect(html).toContain('text-risk');
      expect(html).not.toContain('needs-you'); // amber is "needs you" and the quota chip
      expect(html).toMatch(/<path d="M9 1\.5L3\.5 9H8l-1 5\.5L12\.5 7H8z"/); // the bolt, not the warning sign
      expect(html.replace(/<[^>]*>/g, '')).toBe(''); // no visible words
      expect(html).not.toContain('Acts freely');
    }
    expect(renderToStaticMarkup(<NoAskBadge agent={{}} />)).toBe(''); // negative control: an agent that asks shows nothing
  });
  it('the risk colour is its own role: the fixed red, not the amber of "needs you"', () => {
    const css = read('src/design/app.css');
    expect(css).toContain('--color-risk: var(--diff-removed);');
  });
  it('can be focused (so tapped) to show the explanation, except inside another control', () => {
    expect(renderToStaticMarkup(<NoAskBadge agent={{ skipsPermissions: true }} />)).toContain('tabindex="0"');
    expect(renderToStaticMarkup(<NoAskBadge agent={{ skipsPermissions: true }} nested />)).not.toContain('tabindex');
  });
  it('keeps the started, switched and switched-with-approval meanings apart, and names the cause when known', () => {
    const label = (el: ReactElement) => /aria-label="([^"]*)"/.exec(renderToStaticMarkup(el))?.[1];
    const started = label(<NoAskBadge agent={{ skipsPermissions: true }} />);
    const neutral = label(<NoAskBadge agent={{ skipsPermissions: true }} session />);
    const approved = label(<NoAskBadge agent={{ skipsPermissions: true }} session approved />);
    expect(new Set([started, neutral, approved]).size).toBe(3);
    expect(neutral).toBe('Runs without asking: Acts without asking you first, until it ends or is switched back');
    expect(approved).toContain('with your approval');
    expect(label(<NoAskBadge agent={{ skipsPermissions: true }} session cause={'Mode: YOLO\u202e'} />)).toBe('Runs without asking: Acts without asking you first, until it ends or is switched back. Because of: Mode: YOLO\\u{202e}');
  });
  it('says the same in Turkish', () => {
    setLanguage('tr');
    const html = renderToStaticMarkup(<NoAskBadge agent={{ skipsPermissions: true }} />);
    expect(html).toContain('Onay istemeden çalışır');
    expect(html).not.toContain('Serbest');
    setLanguage('en');
  });
});

describe('quick win: layout pictograms', () => {
  it('draws one rectangle per pane of the preset, inside its box, and the layout says which preset it is', () => {
    const panes = { '1x1': 1, '2x1': 2, '1x2': 2, '2x2': 4, '3x2': 6, focus: 4 } as const;
    for (const p of PRESETS) {
      const rects = pictogram(p);
      expect(rects.length, p).toBe(panes[p]);
      for (const [x, y, w, h] of rects) { expect(x).toBeGreaterThanOrEqual(0); expect(y).toBeGreaterThanOrEqual(0); expect(x + w).toBeLessThanOrEqual(PICTOGRAM_BOX.width); expect(y + h).toBeLessThanOrEqual(PICTOGRAM_BOX.height); }
      const ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: p, agents: ['a', 'b', 'c', 'd', 'e', 'f'] });
      expect(presetOf(ws.layout.root), p).toBe(p);
      expect(p === '1x1' ? 1 : panesOf(ws.layout.root).length, p).toBeGreaterThan(0);
    }
  });
  it('a hand-made layout is none of them', () => {
    let ws = reduce(workspaceOf(defaultLayout()), { type: 'preset', preset: '2x1', agents: ['a', 'b'] });
    ws = reduce(ws, { type: 'split', pane: panesOf(ws.layout.root)[0]!.id, direction: 'column' });
    expect(presetOf(ws.layout.root)).toBeUndefined();
  });
});

describe('quick win: a machine shows its id only when its name is shared', () => {
  const m = (id: string, name: string) => ({ id, name });
  it('decides by shared name', () => {
    expect(sharesName(m('a', 'Laptop'), [m('a', 'Laptop'), m('b', 'Server')])).toBe(false);
    expect(sharesName(m('a', 'Laptop'), [m('a', 'Laptop'), m('b', 'Laptop')])).toBe(true);
    expect(sharesName(m('a', 'Laptop'), [])).toBe(false);
  });
  const board = (machines: { id: string; name: string }[]) => ({ projectId: 'p', project: 'api', tasks: [], leases: [], folders: machines.map((x) => ({ nodeId: x.id, machine: x.name, path: '/srv/api' })) });
  it('the project board follows the same rule', () => {
    setLanguage('en');
    const two = [m('n1', 'Laptop'), m('n2', 'Laptop')];
    expect(renderToStaticMarkup(<ProjectBoard board={board(two)} machines={two} />)).toContain('Laptop · n1');
    const distinct = [m('n1', 'Laptop'), m('n2', 'Server')];
    const html = renderToStaticMarkup(<ProjectBoard board={board(distinct)} machines={distinct} />);
    expect(html).toContain('>Laptop<');
    expect(html).not.toContain('Laptop · n1');
  });
});

describe('quick win: text is never smaller than 12 px', () => {
  it('counts and times use tabular figures', () => expect(read('src/design/app.css')).toMatch(/body \{[^}]*font-variant-numeric: tabular-nums/));
  it('no 10 or 11 px text size is left in the window', () => {
    
    const hits = execSync("grep -rnE 'text-\\[(0\\.6[0-9]*|0\\.5[0-9]*)rem\\]|text-\\[1[01]px\\]' src --include=*.tsx --include=*.css || true", { encoding: 'utf8' });
    expect(hits).toBe('');
  });
});
