import type { AgentTileModel } from './model';
import type { QuotaObs } from './net/quota';

// Demo data for the gallery and the first Home screen until the hub connection feeds the store.
export const demoAgents: AgentTileModel[] = [
  { id: 'a1', workspaceRef: { node: 'laptop', id: 'login-fix' }, name: 'claude · amber', accent: 'var(--agent-amber)', machine: 'Laptop', workspace: 'login-fix', state: 'needs-you', since: Date.now() - 8 * 60_000, reportedBy: 'agent', project: 'login-fix', projectId: 'demo-login', filesChanged: 4, preview: '> Allow editing src/auth.rs?\n  1. Yes   2. No' },
  { id: 'a2', workspaceRef: { node: 'build', id: 'api-tests' }, name: 'codex · teal', accent: 'var(--agent-teal)', machine: 'Build server', workspace: 'api-tests', project: 'api-tests', state: 'working', preview: 'Running 128 tests…', skipsPermissions: true },
  { id: 'a3', name: 'claude · violet', accent: 'var(--agent-violet)', machine: 'Laptop', machineId: 'laptop', workspace: 'docs', project: 'docs', state: 'finished', preview: 'Done. 3 files changed.' },
  { id: 'a4', name: 'wrongstack · rose', accent: 'var(--agent-rose)', machine: 'Windows PC', machineId: 'winpc', workspace: 'ui-polish', project: 'ui-polish', state: 'idle', preview: '$' },
  { id: 'a5', name: 'agent · slate', accent: 'var(--agent-slate)', machine: 'Laptop', machineId: 'laptop', workspace: 'scratch', project: 'scratch', state: 'unknown', preview: '' },
  // The same project (one project id) open on a second machine: the window shows it as one project.
  { id: 'a8', workspaceRef: { node: 'winpc', id: 'login-fix' }, name: 'claude · teal', accent: 'var(--agent-teal)', machine: 'Windows PC', workspace: 'login-fix', project: 'login-fix', projectId: 'demo-login', branch: 'fix/login', state: 'working', since: Date.now() - 95_000, filesChanged: 2, preview: 'Editing src/session.rs…' },
  { id: 'a6', workspaceRef: { node: 'laptop', id: 'login-fix' }, name: 'codex · rose', accent: 'var(--agent-rose)', machine: 'Laptop', workspace: 'login-fix', project: 'login-fix', projectId: 'demo-login', branch: 'fix/login', state: 'finished', preview: '', closed: true, exitCode: 0, startedMs: Date.now() - 1_500_000, endedMs: Date.now() - 600_000, filesChanged: 3, tokens: 48200 },
  { id: 'a7', name: 'claude · mint', accent: 'var(--agent-mint)', machine: 'Build server', machineId: 'build', workspace: 'api-tests', project: 'api-tests', state: 'finished', preview: '', closed: true, exitCode: 1, startedMs: Date.now() - 400_000, endedMs: Date.now() - 90_000, filesChanged: 0, tokens: 9100 },
  // ADR-0060: running when their machine stopped; one can continue its own conversation, the other only start again.
  { id: 'a20', name: 'claude · sky', accent: 'var(--agent-sky)', machine: 'Build server', machineId: 'build', workspace: '/srv/api', project: 'api', state: 'finished', preview: '', closed: true, interrupted: { resumable: true }, startedMs: Date.now() - 3_600_000, endedMs: Date.now() - 120_000, tokens: 21000 },
  { id: 'a21', name: 'codex · slate', accent: 'var(--agent-slate)', machine: 'Build server', machineId: 'build', workspace: '/srv/web', project: 'web', state: 'finished', preview: '', closed: true, interrupted: { resumable: false }, startedMs: Date.now() - 2_000_000, endedMs: Date.now() - 120_000 },
];

// Development only (demo window without a hub): enough machines and folders to try the "Start an agent" window.
export const demoMachines = [
  { id: 'laptop', name: 'Laptop', os: 'linux', online: true, agentsRunning: 3, lastSeenMs: Date.now() - 4000 },
  { id: 'build', name: 'Build server', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: Date.now() - 4000 },
  { id: 'winpc', name: 'Windows PC', os: 'windows', online: false, agentsRunning: 0, lastSeenMs: 0 },
];
export const demoProjects = [
  { node: 'laptop', projectId: 'p1', label: 'login-fix', path: '/home/dev/login-fix' },
  { node: 'laptop', projectId: 'p2', label: 'docs', path: '/home/dev/docs' },
  { node: 'build', projectId: 'p3', label: 'api-tests', path: '/srv/ci/api-tests' },
];

// ADR-0057: a helper the first demo agent started, and that agent's request to start another (for the gallery).
export const demoHelper: AgentTileModel = { id: 'a8', name: 'codex · sky', accent: 'var(--agent-sky)', machine: 'Laptop', workspace: 'login-fix', state: 'working', preview: 'Writing tests…', parentId: 'a1', parentName: 'claude · amber' };
const demoStartDetail = JSON.stringify({ node: 'laptop', agent: 'a1', agent_kind: 'claude', kind: 'codex', folder: '/home/dev/code/login-fix', inside: true, worktree: false, depth: 1, grant_allowed: true, prompt: 'Write unit tests for src/auth.rs.\nCover the expired-token and wrong-signature cases; do not change src/.' });
export const demoStartApproval = { id: 'demo/2', node: 'demo', localId: '2', agentId: 'a1', agentName: 'claude · amber', machineName: 'Laptop', action: 'agent.start', detail: demoStartDetail, expiresAtMs: 0, start: { agent: 'a1', agentKind: 'claude', node: 'laptop', kind: 'codex', folder: '/home/dev/code/login-fix', prompt: 'Write unit tests for src/auth.rs.\nCover the expired-token and wrong-signature cases; do not change src/.', inside: true, worktree: false, depth: 1, grantAllowed: true, settings: [], options: [] } };

// ADR-0090 Amendment A: a (clearly fake) request to switch the rich demo agent to a mode that skips permission questions, as the machine renders it.
const demoSkipDetail = JSON.stringify({ node: 'demo', agent: 'rich', agent_kind: 'claude-acp', folder: '/home/dev/code/login-fix', option_type: 'config', option: 'mode', option_name: 'Mode', value: 'yolo', value_name: 'Bypass Permissions', skips: true, scope: 'session' });
export const demoSkipApproval = { id: 'demo/9', node: 'demo', localId: '9', agentId: 'demo/rich', agentName: 'claude · amber', machineName: 'Laptop', action: 'agent.skip_permissions', detail: demoSkipDetail, expiresAtMs: 0,
  skip: { node: 'demo', agent: 'rich', agentKind: 'claude-acp', folder: '/home/dev/code/login-fix', optionType: 'config' as const, option: 'mode', optionName: 'Mode', value: 'yolo', valueName: 'Bypass Permissions' } };

// The demo window's one waiting approval, for the first demo agent.
export const demoApprovals = [{ id: 'demo/1', node: 'demo', localId: '1', agentId: 'a1', agentName: 'claude · amber', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0 }];
// ADR-0101: a request whose agent offers an answer it remembers as well as a single one, one that offers only that, and an agent that
// holds two such answers ("Asks less").
export const demoAlwaysApproval = { id: 'demo/7', node: 'demo', localId: '7', agentId: 'demo/a9', agentName: 'wstack · teal', action: 'run_command', detail: 'git status', expiresAtMs: 0, always: { label: 'Always allow git status', optionId: 'allow-always-1', only: false } };
export const demoAlwaysOnlyApproval = { id: 'demo/8', node: 'demo', localId: '8', agentId: 'demo/a9', agentName: 'wstack · teal', action: 'write_file', detail: 'src/main.rs', expiresAtMs: 0, always: { label: 'Allow edits this session', optionId: 'edits', only: true } };
export const demoAsksLess: AgentTileModel = { id: 'demo/a9', name: 'wstack · teal', accent: 'var(--agent-teal)', machine: 'Laptop', workspace: 'login-fix', state: 'working', preview: 'Running git status…', alwaysAllowed: ['run_command', 'write_file'] };

// Development only: a small folder tree for the demo window's folder picker.
const demoTree: Record<string, string[]> = {
  '/home/dev': ['code', 'docs', 'Downloads', 'notes'],
  '/home/dev/code': ['login-fix', 'api-tests', 'website', 'scripts'],
  '/home/dev/code/login-fix': ['src', 'tests'],
};
export function demoList(path: string) {
  const at = path || '/home/dev';
  const names = demoTree[at] ?? [];
  return { path: at, parent: at === '/' ? '' : at.slice(0, at.lastIndexOf('/')) || '/', entries: names.map((name) => ({ name, isGit: ['login-fix', 'api-tests', 'website'].includes(name) })), truncated: false };
}
// ADR-0066: the same request asking for a model and an effort (no "more like this": a start with settings is always asked).
const demoSettings = [{ id: 'model', label: 'setting-model', value: 'gpt-6.1-sol' }, { id: 'effort', label: 'setting-effort', value: 'high' }];
export const demoStartSettingsApproval = { ...demoStartApproval, id: 'demo/3', localId: '3', detail: JSON.stringify({ ...JSON.parse(demoStartDetail), grant_allowed: false, settings: demoSettings }), start: { ...demoStartApproval.start, grantAllowed: false, settings: demoSettings } };
// ADR-0059 decision 5: the same request asking for a helper that acts without asking (the card asks for the confirmation before it allows).
const demoNoAsk = [{ id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true }];
export const demoStartNoAskApproval = { ...demoStartApproval, id: 'demo/4', localId: '4', detail: JSON.stringify({ ...JSON.parse(demoStartDetail), grant_allowed: false, launch_options: [{ id: 'skip-permissions', label: 'option-skip-permissions', skips_permissions: true }] }), start: { ...demoStartApproval.start, grantAllowed: false, options: demoNoAsk } };

// ADR-0091: plan-limit observations for the demo window (clearly fake). The Laptop's Codex is getting full, the Build server's almost
// used up (and one older reading), the Windows PC's window just reset; Claude reports nothing, so it shows as unknown.
const MIN = 60_000;
export const demoQuota: QuotaObs[] = [
  { machineId: 'laptop', tool: 'codex', window: 'five_hour', usedPercent: 82, resetsAtMs: Date.now() + 95 * MIN, observedAtMs: Date.now() - 4 * MIN },
  { machineId: 'laptop', tool: 'codex', window: 'seven_day', usedPercent: 41, resetsAtMs: Date.now() + 3 * 86_400_000, observedAtMs: Date.now() - 4 * MIN },
  { machineId: 'build', tool: 'codex', window: 'five_hour', usedPercent: 97, resetsAtMs: Date.now() + 80 * MIN, observedAtMs: Date.now() - 2 * MIN },
  { machineId: 'build', tool: 'codex', window: 'seven_day', usedPercent: 63, resetsAtMs: Date.now() + 5 * 86_400_000, observedAtMs: Date.now() - 2 * MIN },
  { machineId: 'build', tool: 'codex', window: 'minutes:30', usedPercent: 12, resetsAtMs: Date.now() + 10 * MIN, observedAtMs: Date.now() - 50 * MIN },
  { machineId: 'winpc', tool: 'codex', window: 'five_hour', usedPercent: 74, resetsAtMs: Date.now() - 20 * MIN, observedAtMs: Date.now() - 6 * 60 * MIN },
];
