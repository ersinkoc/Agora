import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from './i18n';
import type { AgentTileModel } from './model';
import type { ApprovalModel } from './net/view';
import { Console } from './components/Console';
import { TerminalFloat } from './terminal/TerminalFloat';
import { isMachinesHash, machineOfHash, machinesHash, paletteExtras, projectsOf, type PaletteExtras } from './net/paletteExtras';
import { demoSource } from './terminal/source';

beforeEach(() => setLanguage('en'));

const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: 'login-fix', project: 'login-fix', projectId: 'p1', state: 'needs-you', preview: '', ...over });
const ask = (agentId: string): ApprovalModel => ({ id: `r-${agentId}`, node: 'n1', localId: 'r', agentId, agentName: '', action: 'Edit src/auth.rs', detail: '', expiresAtMs: 0 });
const noop = () => undefined;

describe('"Needs you" is never hidden', () => {
  const props = { agents: [agent('a')], machines: [], approvals: [ask('a')], messages: [], terminalFor: () => demoSource('x'), onOpen: noop, onResolve: noop, link: 'live' as const };
  const panel = (html: string) => html.slice(html.lastIndexOf('<aside'), html.lastIndexOf('</aside>'));

  it('on Home the console lists it and the right panel keeps its Needs you tab and list too', () => {
    const html = renderToStaticMarkup(<Console {...props} />);
    expect(html).toContain('fleet-attention');
    const aside = panel(html);
    expect(aside).toContain('role="tablist"');
    expect(aside).toContain('>Needs you');
    expect(aside).toContain('Edit src/auth.rs');
  });
  it('the status bar count opens that panel', () => {
    expect(renderToStaticMarkup(<Console {...props} />)).toContain('1 needs you');
  });
});

describe('the terminal\'s bottom area', () => {
  const float = (over: Partial<Parameters<typeof TerminalFloat>[0]> = {}) =>
    renderToStaticMarkup(<TerminalFloat atBottom={false} onJump={noop} onCopy={noop} onIgnore={noop} link="https://example.com/a/very/long/address" {...over} />);

  it('shows Jump to latest and the link address together, in one column, the address last', () => {
    const html = float();
    const jump = html.indexOf('terminal-jump');
    const link = html.indexOf('terminal-link');
    expect(jump).toBeGreaterThan(-1);
    expect(link).toBeGreaterThan(jump);
    expect(html.match(/class="terminal-float"/g)).toHaveLength(1);
    // Both are children of the same stack: a closing of the stack is the last thing in the markup.
    expect(html.indexOf('</div>', link)).toBe(html.length - '</div>'.length);
    expect(html).toContain(t('terminal-jump-bottom'));
  });
  it('keeps the address after a clipboard offer too, and draws nothing when there is nothing to show', () => {
    const html = float({ offer: 'some text' });
    expect(html.indexOf('terminal-offer')).toBeLessThan(html.indexOf('terminal-link'));
    expect(float({ atBottom: true, link: undefined })).toBe('<div class="terminal-float"></div>');
  });
  it('lays the address out in the flow of that column, never positioned over the buttons', () => {
    const css = readFileSync(new URL('./design/app.css', import.meta.url), 'utf8');
    const rule = css.split('\n').find((l) => l.trim().startsWith('.terminal-link {')) ?? '';
    expect(rule).not.toMatch(/\babsolute\b/);
    expect(rule).not.toMatch(/\bbottom-0\b/);
    const view = readFileSync(new URL('./terminal/TerminalView.tsx', import.meta.url), 'utf8');
    expect(view).not.toContain('className="terminal-link"'); // only TerminalFloat draws it
  });
});

describe('command palette extras', () => {
  const base = (over: Partial<PaletteExtras> = {}): PaletteExtras => ({
    machines: [{ id: 'n1', name: 'laptop' }, { id: 'n2', name: 'server' }], projects: [{ id: 'p1', name: 'shop' }], hub: true, phone: false, passwordSet: true, canPair: true,
    go: vi.fn(), showProject: vi.fn(), showPairing: vi.fn(), lock: vi.fn(), ...over,
  });
  const labels = (x: PaletteExtras) => paletteExtras(x).map((i) => i.label);

  it('jumps to a machine and to a project by name', () => {
    const x = base();
    const items = paletteExtras(x);
    expect(labels(x)).toContain('Machine: laptop');
    expect(labels(x)).toContain('Project: shop');
    items.find((i) => i.id === 'machine-n2')?.run();
    expect(x.go).toHaveBeenCalledWith('#/machines/n2');
    items.find((i) => i.id === 'project-p1')?.run();
    expect(x.showProject).toHaveBeenCalledWith('p1');
  });
  it('has Settings, and Allow a new machine to join only where it can be done', () => {
    const x = base();
    expect(labels(x)).toContain('Settings');
    paletteExtras(x).find((i) => i.id === 'settings')?.run();
    expect(x.go).toHaveBeenCalledWith('#/device');
    expect(labels(x)).toContain('Allow a new machine to join');
    expect(labels(base({ canPair: false }))).not.toContain('Allow a new machine to join');
    expect(labels(base({ hub: false }))).not.toContain('Allow a new machine to join');
  });
  it('shows Lock only when a screen password is set, and runs the existing lock action', () => {
    const x = base();
    expect(labels(x)).toContain('Lock');
    paletteExtras(x).find((i) => i.id === 'lock')?.run();
    expect(x.lock).toHaveBeenCalledTimes(1);
    expect(labels(base({ passwordSet: false }))).not.toContain('Lock');
    expect(labels(base({ hub: false }))).not.toContain('Lock');
  });
  it('a phone has no project jump', () => {
    expect(labels(base({ phone: true })).some((l) => l.startsWith('Project:'))).toBe(false);
  });
  it('"Allow a new machine to join" only shows where that is done; it opens nothing and answers nothing itself', () => {
    const x = base();
    paletteExtras(x).find((i) => i.id === 'pair')?.run();
    expect(x.showPairing).toHaveBeenCalledTimes(1);
    expect(x.go).not.toHaveBeenCalled();
    expect(x.lock).not.toHaveBeenCalled();
    // The builder is given no approval, pairing answer or window switch at all, so no entry can perform one.
    expect(Object.keys(base()).sort()).toEqual(['canPair', 'go', 'hub', 'lock', 'machines', 'passwordSet', 'phone', 'projects', 'showPairing', 'showProject']);
  });
  it('entries carry names and routes only, never a token, code or key', () => {
    const items = paletteExtras(base({ machines: [{ id: 'n1', name: 'laptop' }] }));
    for (const i of items) expect(`${i.id} ${i.label}`).not.toMatch(/token|secret|password|code=|[0-9a-f]{32}/i);
  });
  it('has the machine and project routes and names', () => {
    expect(machinesHash('n/1 x')).toBe('#/machines/n%2F1%20x');
    expect(machineOfHash(machinesHash('n/1 x'))).toBe('n/1 x');
    expect(machineOfHash('#/machines')).toBeUndefined();
    expect(isMachinesHash('#/machines') && isMachinesHash('#/machines/n1') && !isMachinesHash('#/machinesx')).toBe(true);
    expect(projectsOf([agent('a'), agent('b'), agent('c', { projectId: 'p2', project: 'blog' }), agent('d', { projectId: undefined })])).toEqual([{ id: 'p1', name: 'login-fix' }, { id: 'p2', name: 'blog' }]);
  });
  it('decodes a malformed machine hash to nothing instead of throwing while rendering', () => {
    // The hash is untrusted input (a link from anywhere); a bad escape must not take the page down.
    expect(machineOfHash('#/machines/%')).toBeUndefined();
    expect(machineOfHash('#/machines/%zz')).toBeUndefined();
    expect(machineOfHash('#/machines/%E0%80')).toBeUndefined();
    expect(machineOfHash(machinesHash('n/1 x'))).toBe('n/1 x');
  });
});

describe('Start an agent and New terminal on My machines', () => {
  it('go to the window first, where the Start window is drawn, so it opens at once (#1116)', () => {
    const app = readFileSync(new URL('./App.tsx', import.meta.url), 'utf8');
    const page = app.slice(app.indexOf('<MachinesPage'), app.indexOf('/>', app.indexOf('<MachinesPage')));
    expect(page).toContain('onStart={hub && api && live ? (node) => { toWindow(); openStart(node); }');
    expect(page).toContain('onNewTerminal={hub && api && live ? (node) => { toWindow(); openStart(node, true); }');
  });
});
