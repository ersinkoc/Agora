import type { AgentTileModel } from './model';

// Demo data for the gallery and the first Home screen until the hub connection feeds the store.
export const demoAgents: AgentTileModel[] = [
  { id: 'a1', name: 'claude · amber', accent: '#ffb454', machine: 'Laptop', workspace: 'login-fix', state: 'needs-you', since: 1000, reportedBy: 'agent', preview: '> Allow editing src/auth.rs?\n  1. Yes   2. No' },
  { id: 'a2', name: 'codex · teal', accent: '#4fd1c5', machine: 'Build server', workspace: 'api-tests', state: 'working', preview: 'Running 128 tests…' },
  { id: 'a3', name: 'claude · violet', accent: '#a78bfa', machine: 'Laptop', workspace: 'docs', state: 'finished', preview: 'Done. 3 files changed.' },
  { id: 'a4', name: 'wrongstack · rose', accent: '#fb7185', machine: 'Windows PC', workspace: 'ui-polish', state: 'idle', preview: '$' },
  { id: 'a5', name: 'agent · slate', accent: '#94a3b8', machine: 'Laptop', workspace: 'scratch', state: 'unknown', preview: '' },
  { id: 'a6', name: 'codex · rose', accent: '#f472b6', machine: 'Laptop', workspace: 'login-fix', project: 'login-fix', branch: 'fix/login', state: 'finished', preview: '', closed: true, exitCode: 0, startedMs: Date.now() - 1_500_000, endedMs: Date.now() - 600_000, filesChanged: 3, tokens: 48200 },
  { id: 'a7', name: 'claude · olive', accent: '#a3e635', machine: 'Build server', workspace: 'api-tests', project: 'api-tests', state: 'finished', preview: '', closed: true, exitCode: 1, startedMs: Date.now() - 400_000, endedMs: Date.now() - 90_000, filesChanged: 0, tokens: 9100 },
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
export const demoHelper: AgentTileModel = { id: 'a8', name: 'codex · sky', accent: '#38bdf8', machine: 'Laptop', workspace: 'login-fix', state: 'working', preview: 'Writing tests…', parentId: 'a1', parentName: 'claude · amber' };
const demoStartDetail = JSON.stringify({ node: 'laptop', agent: 'a1', agent_kind: 'claude', kind: 'codex', folder: '/home/dev/code/login-fix', inside: true, worktree: false, depth: 1, grant_allowed: true, prompt: 'Write unit tests for src/auth.rs.\nCover the expired-token and wrong-signature cases; do not change src/.' });
export const demoStartApproval = { id: 'demo/2', node: 'demo', localId: '2', agentId: 'a1', agentName: 'claude · amber', action: 'agent.start', detail: demoStartDetail, expiresAtMs: 0, start: { agent: 'a1', agentKind: 'claude', node: 'laptop', kind: 'codex', folder: '/home/dev/code/login-fix', prompt: 'Write unit tests for src/auth.rs.\nCover the expired-token and wrong-signature cases; do not change src/.', inside: true, worktree: false, depth: 1, grantAllowed: true } };

// The demo window's one waiting approval, for the first demo agent.
export const demoApprovals = [{ id: 'demo/1', node: 'demo', localId: '1', agentId: 'a1', agentName: 'claude · amber', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0 }];

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
