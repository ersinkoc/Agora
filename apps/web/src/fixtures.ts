import type { AgentTileModel } from './model';

// Demo data for the gallery and the first Home screen until the hub connection feeds the store.
export const demoAgents: AgentTileModel[] = [
  { id: 'a1', name: 'claude · amber', accent: '#ffb454', machine: 'Laptop', workspace: 'login-fix', state: 'needs-you', since: 1000, reportedBy: 'agent', preview: '> Allow editing src/auth.rs?\n  1. Yes   2. No' },
  { id: 'a2', name: 'codex · teal', accent: '#4fd1c5', machine: 'Build server', workspace: 'api-tests', state: 'working', preview: 'Running 128 tests…' },
  { id: 'a3', name: 'claude · violet', accent: '#a78bfa', machine: 'Laptop', workspace: 'docs', state: 'finished', preview: 'Done. 3 files changed.' },
  { id: 'a4', name: 'wrongstack · rose', accent: '#fb7185', machine: 'Windows PC', workspace: 'ui-polish', state: 'idle', preview: '$' },
  { id: 'a5', name: 'agent · slate', accent: '#94a3b8', machine: 'Laptop', workspace: 'scratch', state: 'unknown', preview: '' },
];

// Development only (demo window without a hub): enough machines and folders to try the "Start an agent" window.
export const demoMachines = [
  { id: 'laptop', name: 'Laptop', os: 'linux', online: true, agentsRunning: 3, lastSeenMs: 0 },
  { id: 'build', name: 'Build server', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: 0 },
  { id: 'winpc', name: 'Windows PC', os: 'windows', online: false, agentsRunning: 0, lastSeenMs: 0 },
];
export const demoProjects = [
  { node: 'laptop', projectId: 'p1', label: 'login-fix', path: '/home/dev/login-fix' },
  { node: 'laptop', projectId: 'p2', label: 'docs', path: '/home/dev/docs' },
  { node: 'build', projectId: 'p3', label: 'api-tests', path: '/srv/ci/api-tests' },
];

// The demo window's one waiting approval, for the first demo agent.
export const demoApprovals = [{ id: 'demo/1', node: 'demo', localId: '1', agentId: 'a1', agentName: 'claude · amber', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0 }];
