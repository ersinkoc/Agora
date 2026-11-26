// What a plugin is shown (ADR-0114 decision 3; layout.md, "Not for plugins as it is"): the bridge's own view, never the built-in panels'
// context, state or actions, and answers that are plain data with no requests for approval, no terminal data and nothing to act with.
import { isValidElement, type ReactElement } from 'react';
import { describe, expect, it } from 'vitest';
import type { AgentTileModel } from '../model';
import type { ChatLine } from '../net/chat';
import type { PanelContext, PanelDef } from '../layout/panels';
import { CAPABILITIES } from './capabilities';
import { checkPackage } from './package';
import { MAX_AGENTS, MAX_ENTRIES, project, projectChanges, projectChat, viewOf } from './projection';
import { pluginPanelId, pluginPanels } from './registry';
import type { LoadedPlugin } from './store';
import { bundle } from './testPackage';

const SECRET = 'TERMINAL-SECRET-sk-live-1234';
const agent = (id: string, extra: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id, name: `claude · ${id}`, accent: 'var(--agent-teal)', kind: 'claude', machine: 'Laptop', machineId: 'laptop', workspace: '/home/me/proj', project: 'proj',
  state: 'needs-you', preview: `$ export KEY=${SECRET}`, tokens: 1200, startedMs: 1000, skipsPermissions: true, alwaysAllowed: ['rm -rf'], filesChanged: 2,
  session: { modes: [], models: [], currentMode: 'm', currentModel: 'x', contextTokens: 50, contextWindow: 100, costMicros: 99, currency: 'USD' },
  ...extra,
});
const fleet = [agent('n/a'), agent('n/b', { state: 'working', closed: true, endedMs: 5000 })];
const boards = [{ projectId: 'p', project: 'proj', folders: [{ nodeId: 'n', machine: 'Laptop', path: '/home/me/proj' }], tasks: [{ id: 't1', title: 'Fix login', status: 'open' as const, owner: 'claude · n/a', held: ['src/a.ts'] }], leases: [{ path: 'src/a.ts', holder: 'x', expiresAtMs: 1 }] }];
const theme = () => ({ scheme: 'dark' as const, tokens: { accent: '#fff' } });

// Everything reachable from a value: no function anywhere, and no string holding the terminal's text.
function walk(v: unknown, path = '$', out: string[] = []): string[] {
  if (typeof v === 'function') out.push(`${path}: function`);
  else if (typeof v === 'string') { if (v.includes(SECRET)) out.push(`${path}: terminal text`); }
  else if (Array.isArray(v)) v.forEach((x, i) => walk(x, `${path}[${i}]`, out));
  else if (v && typeof v === 'object') for (const [k, x] of Object.entries(v)) { if (/approval|preview|terminal|skip|always|cost|currency/i.test(k)) out.push(`${path}.${k}`); walk(x, `${path}.${k}`, out); }
  return out;
}

describe('the plugin view and its answers', () => {
  const view = viewOf({ pane: 'p1', kind: 'tool', agent: fleet[0], fleet, boards, now: 10_000, chatOf: () => ({ connect: () => ({ close() {} }) }), loadDiff: async () => '' });

  it('answers each capability with plain data: no approvals, no terminal data, no actions', () => {
    for (const cap of CAPABILITIES) {
      const data = project(cap, view, theme);
      expect(walk(data), cap).toEqual([]);
      expect(JSON.parse(JSON.stringify(data ?? null)), cap).toEqual(data ?? null);
    }
    expect(project('agents.read', view, theme)).toEqual({ agents: [
      { id: 'n/a', name: 'claude · n/a', kind: 'claude', state: 'needs-you', machine: 'Laptop', project: 'proj', closed: false },
      { id: 'n/b', name: 'claude · n/b', kind: 'claude', state: 'working', machine: 'Laptop', project: 'proj', closed: true },
    ] });
    expect(project('usage.read', view, theme)).toEqual({ agents: [
      { id: 'n/a', name: 'claude · n/a', tokens: 1200, contextPercent: 50, activeMs: 9000 },
      { id: 'n/b', name: 'claude · n/b', tokens: 1200, contextPercent: 50, activeMs: 4000 },
    ], totalTokens: 2400 });
    expect(project('tasks.read', view, theme)).toEqual({ boards: [{ project: 'proj', tasks: [{ id: 't1', title: 'Fix login', status: 'open', owner: 'claude · n/a' }] }] });
    expect(project('pane.context', view, theme)).toEqual({ pane: { id: 'p1', kind: 'tool', agent: { id: 'n/a', name: 'claude · n/a' } } });
    expect(project('theme.read', view, theme)).toEqual(theme());
    // The two read from the node are answered by their own projections.
    expect(project('chat.read', view, theme)).toBeUndefined();
    expect(project('changes.read', view, theme)).toBeUndefined();
  });

  it('keeps the view itself to the plugin facts: no slices of the window state, no actions, no terminal', () => {
    expect(Object.keys(view).sort()).toEqual(['agents', 'boards', 'changesKey', 'chat', 'diff', 'pane', 'usage']);
    expect(walk({ ...view, chat: undefined, diff: undefined })).toEqual([]);
    expect(viewOf({ pane: 'p', kind: 'global', fleet: Array.from({ length: MAX_AGENTS + 5 }, (_, i) => agent(`a${i}`)), boards: [], now: 0 }).agents).toHaveLength(MAX_AGENTS);
    // Bound to no agent: no conversation and no changes to read.
    const unbound = viewOf({ pane: 'p', kind: 'global', fleet, boards, now: 0, chatOf: () => ({ connect: () => ({ close() {} }) }), loadDiff: async () => '' });
    expect(unbound.chat).toBeUndefined();
    expect(unbound.diff).toBeUndefined();
  });

  it('shows the conversation without requests for approval and only the newest entries', () => {
    const line = (id: number, extra: Partial<ChatLine> = {}): ChatLine => ({ id, atMs: id, role: 'agent', kind: 'text', text: `t${id}`, path: '', ...extra });
    const lines = [line(1), line(2, { kind: 'permission', approvalId: 'ap-1', outcome: 'pending' }), line(3, { kind: 'tool', approvalId: 'ap-2', output: 'ok', callKind: 'run', status: 'completed' })];
    const out = projectChat('n/a', lines);
    expect(out).toEqual({ agent: 'n/a', entries: [
      { id: 1, atMs: 1, role: 'agent', kind: 'text', text: 't1' },
      { id: 3, atMs: 3, role: 'agent', kind: 'tool', text: 't3', callKind: 'run', status: 'completed', output: 'ok' },
    ] });
    expect(walk(out)).toEqual([]);
    expect(projectChat('n/a', Array.from({ length: MAX_ENTRIES + 50 }, (_, i) => line(i))).entries).toHaveLength(MAX_ENTRIES);
  });

  it('shows changes as file lists and diffs, and a secret file by its counts only', () => {
    const diff = 'diff --git a/.env b/.env\n--- a/.env\n+++ b/.env\n@@ -1 +1,2 @@\n-KEY=old\n+KEY=new\n+B=2\ndiff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1 +1 @@\n-old\n+new\n';
    const out = projectChanges('n/a', { diff, secretFiles: [{ path: '.env', added: 2, removed: 1 }] });
    expect(out.agent).toBe('n/a');
    expect(out.files.find((f) => f.path === 'src/a.ts')?.lines.map((l) => [l.kind, l.text])).toEqual([['hunk', '@@ -1 +1 @@'], ['del', 'old'], ['add', 'new']]);
    expect(out.files.find((f) => f.path === '.env')).toMatchObject({ secret: true, added: 2, removed: 1, lines: [] });
    expect(JSON.stringify(out)).not.toContain('KEY=');
  });
});

describe('a plugin panel in the registry', () => {
  it('has the facts the window sets, a namespaced id, and gives its frame only the plugin view', async () => {
    const r = await checkPackage(bundle(), '0.3.39');
    if (!r.ok) throw new Error(r.code);
    const loaded: LoadedPlugin = { plugin: r.plugin, text: '', dev: true, state: 'ready', granted: ['agents.read'] };
    const [def] = pluginPanels([loaded]) as [PanelDef];
    expect(def).toMatchObject({ id: pluginPanelId('demo', 'main'), label: 'plugin-panel', kind: 'global', binds: 'nothing', streams: false, lone: false, plugin: { name: 'Demo', dev: true } });
    expect(def.id).toBe('plugin.demo.main');
    // A held plugin registers nothing.
    expect(pluginPanels([{ ...loaded, state: 'held', granted: [] }])).toEqual([]);

    const ctx = {
      pane: 'p1', panel: def.id, agent: undefined, terminal: () => { throw new Error('no terminal for a plugin'); },
      state: { fleet, machines: [], approvals: [{ id: 'ap-1' }], attention: [], changed: [], messages: [], boards, projects: [], now: 0, link: 'live' },
      sources: {}, actions: { resolve: () => { throw new Error('never'); }, sendPrompt: () => { throw new Error('never'); } },
    } as unknown as PanelContext;
    const el = def.render?.(ctx) as ReactElement<Record<string, unknown>>;
    expect(isValidElement(el)).toBe(true);
    expect(Object.keys(el.props).sort()).toEqual(['dev', 'granted', 'panel', 'plugin', 'view']);
    for (const v of Object.values(el.props)) for (const bad of [ctx, ctx.state, ctx.actions, ctx.sources]) expect(v).not.toBe(bad);
    expect(walk(el.props.view)).toEqual([]);
    expect(JSON.stringify(el.props.view)).not.toContain('ap-1');
  });
});
