import type { AgentTileModel } from './model';

// Demo data for the gallery and the first Home screen until the hub connection feeds the store.
export const demoAgents: AgentTileModel[] = [
  { id: 'a1', name: 'claude · amber', accent: '#ffb454', machine: 'Laptop', workspace: 'login-fix', state: 'needs-you', since: 1000, reportedBy: 'agent', preview: '> Allow editing src/auth.rs?\n  1. Yes   2. No' },
  { id: 'a2', name: 'codex · teal', accent: '#4fd1c5', machine: 'Build server', workspace: 'api-tests', state: 'working', preview: 'Running 128 tests…' },
  { id: 'a3', name: 'claude · violet', accent: '#a78bfa', machine: 'Laptop', workspace: 'docs', state: 'finished', preview: 'Done. 3 files changed.' },
  { id: 'a4', name: 'wrongstack · rose', accent: '#fb7185', machine: 'Windows PC', workspace: 'ui-polish', state: 'idle', preview: '$' },
  { id: 'a5', name: 'agent · slate', accent: '#94a3b8', machine: 'Laptop', workspace: 'scratch', state: 'unknown', preview: '' },
];
