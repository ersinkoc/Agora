import type { PairingUi } from './components/Pairing';
import type { AgentTileModel } from './model';
import type { MachineModel } from './net/view';
import type { QuotaObs } from './net/quota';

// Demo data for the gallery and the first Home screen until the hub connection feeds the store.
export const demoAgents: AgentTileModel[] = [
  { id: 'a1', workspaceRef: { node: 'laptop', id: 'login-fix' }, name: 'claude · amber', model: 'opus', effort: 'high', provider: 'anthropic', accent: 'var(--agent-amber)', machine: 'Laptop', workspace: 'login-fix', shared: true, state: 'needs-you', since: Date.now() - 8 * 60_000, reportedBy: 'agent', project: 'login-fix', projectId: 'demo-login', filesChanged: 4, preview: '> Allow editing src/auth.rs?\n  1. Yes   2. No' },
  { id: 'a2', workspaceRef: { node: 'build', id: 'api-tests' }, name: 'codex · teal', model: 'gpt-6', effort: 'medium', provider: 'openai', accent: 'var(--agent-teal)', machine: 'Build server', workspace: 'api-tests', project: 'api-tests', state: 'working', preview: 'Running 128 tests…', skipsPermissions: true },
  { id: 'a3', name: 'claude · violet', model: 'sonnet', provider: 'anthropic', accent: 'var(--agent-violet)', machine: 'Laptop', machineId: 'laptop', workspace: 'docs', project: 'docs', state: 'finished', preview: 'Done. 3 files changed.' },
  { id: 'a4', name: 'wrongstack · rose', accent: 'var(--agent-rose)', machine: 'Windows PC', machineId: 'winpc', workspace: 'ui-polish', project: 'ui-polish', state: 'idle', preview: '$' },
  // ADR-0094: a Shell where the person started Claude Code by hand; the machine found it from the program in front of the terminal.
  { id: 'a9', name: 'claude-code · sky', kind: 'claude-code', hostKind: 'shell', runsInside: { kind: 'claude-code', name: 'Claude Code' }, accent: 'var(--agent-sky)', machine: 'Laptop', machineId: 'laptop', workspace: 'login-fix', project: 'login-fix', projectId: 'demo-login', state: 'working', since: Date.now() - 40_000, reportedBy: 'screen', preview: '* Reading src/auth.rs… (esc to interrupt)' },
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
// Redesign PR 5: a helper's start with a long first prompt (audit screenshot 26), summarised on its card and read whole in Review.
const demoLongPrompt = 'Write unit tests for src/auth.rs.\n\nCover these cases:\n1. A token exactly at its expiry is still valid.\n2. A token one second past its expiry is refused.\n3. A token whose signature was made with another key is refused, and the error says which check failed.\n4. A token issued with a clock 40 seconds ahead is accepted within the skew window.\n\nDo not change anything under src/ except tests; if a case cannot be tested without changing the code, write it down in the task board instead of changing the code. Run the tests with cargo test -p login auth:: and report the output.';
export const demoStartLongApproval = { ...demoStartApproval, id: 'demo/5', localId: '5', detail: JSON.stringify({ ...JSON.parse(demoStartDetail), prompt: demoLongPrompt }), start: { ...demoStartApproval.start, prompt: demoLongPrompt } };
// ADR-0119: a start another machine asks this one for (an agent on mac asks the laptop), with "More like this" as a standing grant.
const demoRemoteStartDetail = JSON.stringify({ node: 'laptop', origin_node: 'mac', origin_agent: 'agent-3', origin_agent_kind: 'claude-code', kind: 'codex', folder: '/home/dev/code/login-fix', worktree: false, depth: 0, root_node: 'mac', root_agent: 'agent-3', grant_allowed: true, prompt: 'Run the integration tests on this machine and report what fails.' });
export const demoRemoteStartApproval = { id: 'demo/9', node: 'demo', localId: '9', agentId: '', agentName: '', machineName: 'Laptop', action: 'agent.start.remote', detail: demoRemoteStartDetail, expiresAtMs: 0, start: { agent: 'agent-3', agentKind: 'claude-code', node: 'laptop', kind: 'codex', folder: '/home/dev/code/login-fix', prompt: 'Run the integration tests on this machine and report what fails.', inside: true, worktree: false, depth: 0, grantAllowed: true, settings: [], options: [], remote: { originNode: 'mac', originAgent: 'agent-3', originAgentKind: 'claude-code' } } };

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

// My machines in the gallery (redesign PR 9), on a fixed clock so its screenshots and the visual baselines never drift: two machines
// online with their agents, one offline for two hours with agents still running there, one old (not seen for nine days), and a request
// to join waiting for the owner.
export const GALLERY_NOW = 1_700_000_000_000;
const MB = 1024 * 1024;
export const galleryMachines: MachineModel[] = [
  { id: 'blackview', name: 'blackview', os: 'linux', online: true, agentsRunning: 4, lastSeenMs: GALLERY_NOW - 4_000, memoryBytes: 17 * MB, build: '0.3.35', terminalsOpen: 6 },
  { id: 'mac', name: 'mac', os: 'macos', online: true, agentsRunning: 2, lastSeenMs: GALLERY_NOW - 9_000, memoryBytes: 5.6 * MB, build: '0.3.35', terminalsOpen: 2 },
  { id: 'win', name: 'win', os: 'windows', online: false, agentsRunning: 2, lastSeenMs: GALLERY_NOW - 2 * 3_600_000, build: '0.3.34' },
  { id: 'linux-test', name: 'linux', os: 'linux', online: false, agentsRunning: 0, lastSeenMs: GALLERY_NOW - 9 * 86_400_000, build: '0.3.2' },
];
const onMachine = (id: string, machineId: string, name: string, kind: string, accent: string, rest: Partial<AgentTileModel>): AgentTileModel =>
  ({ id, name, kind, accent: `var(--agent-${accent})`, machine: machineId, machineId, workspace: 'login-fix', state: 'working', preview: '', ...rest });
export const galleryMachineAgents: AgentTileModel[] = [
  onMachine('g1', 'blackview', 'claude · plum', 'claude', 'violet', { state: 'needs-you' }),
  onMachine('g2', 'blackview', 'codex · teal', 'codex', 'teal', { state: 'needs-you' }),
  onMachine('g3', 'blackview', 'opencode · sky', 'opencode', 'sky', { skipsPermissions: true }),
  onMachine('g4', 'blackview', 'claude · amber', 'claude', 'amber', { state: 'finished' }),
  onMachine('g5', 'mac', 'codex · mint', 'codex', 'mint', {}),
  onMachine('g6', 'mac', 'gemini · rose', 'gemini', 'rose', { state: 'finished', closed: true, exitCode: 1 }),
];
export const galleryPairing: PairingUi = {
  cards: [{ id: 'pair-desk', kind: 'machine', name: 'desk', os: 'linux', address: '192.168.1.52', number: '640218', expiresMs: GALLERY_NOW + 240_000, pairId: new Uint8Array([1]) }],
  recent: [],
  window: { open: false, closesAtMs: 0, pending: 1 },
  offline: false,
  onAnswer: async () => 'ok',
  onOpen: async () => true,
};

// The Settings page with nothing live behind it (#/gallery/settings, also served by a production build): three machines of three
// systems (one offline, one that does not trust this browser yet), two other devices of the person's, and what each machine can start.
// Times are fixed against `demoSettingsNow`, so the screenshots of this page do not change from one run to the next.
export const demoSettingsNow = Date.UTC(2026, 10, 22, 21, 40);
export const demoSettingsMachines = [
  { id: 'blackview', name: 'blackview', os: 'linux', online: true, agentsRunning: 3, lastSeenMs: demoSettingsNow - 4000, build: '0.3.35' },
  { id: 'mac', name: 'mac', os: 'macos', online: true, agentsRunning: 1, lastSeenMs: demoSettingsNow - 9000, build: '0.3.35' },
  { id: 'win', name: 'win', os: 'windows', online: false, agentsRunning: 0, lastSeenMs: demoSettingsNow - 2 * 3_600_000, build: '0.3.34' },
];
export const demoSettingsTrustedOn = ['blackview', 'win'];
export const demoSettingsDevices = [
  { id: 'web-7188eda9c2b62f50', name: 'Chrome · Linux', fingerprint: '7188-eda9-c2b6-2f50-9a5a-2a1c', lastUsedMs: demoSettingsNow - 8 * 60_000, trustedOn: [], waiting: true, removed: false },
  { id: 'web-2a9011fe0c3d77b1', name: 'Safari · iOS', fingerprint: '2a90-11fe-0c3d-77b1-e420-9d05', lastUsedMs: demoSettingsNow - 26 * 3_600_000, trustedOn: ['blackview', 'mac'], waiting: false, removed: false },
  { id: 'web-5c31a0e2d9f4b871', name: 'Firefox · Windows', fingerprint: '5c31-a0e2-d9f4-b871-03c6-e15a', lastUsedMs: demoSettingsNow - 40 * 86_400_000, trustedOn: [], waiting: false, removed: true },
];
const skip = { id: 'skip-permissions', label: 'option-skip-permissions', skipsPermissions: true };
const model = { id: 'model', label: 'setting-model', values: [], defaultValue: '' };
const effort = (d = '') => ({ id: 'effort', label: 'setting-effort', values: ['low', 'medium', 'high', 'xhigh', 'max'], defaultValue: d });
export const demoSettingsKinds = [
  { kind: 'claude-code', name: 'Claude Code', installed: true, level: 2, program: 'claude', settings: [model, effort()], options: [skip] },
  { kind: 'codex', name: 'Codex', installed: true, level: 2, program: 'codex', settings: [model, effort('medium')], options: [skip] },
  { kind: 'antigravity-acp', name: 'Antigravity', installed: true, level: 3, program: 'antigravity', settings: [], options: [] },
  { kind: 'wrongstack', name: 'WrongStack', installed: true, level: 2, program: 'wrongstack', settings: [], options: [skip] },
  { kind: 'fake-adapter', name: 'Fake chat agent', installed: true, level: 4, program: 'agora-fake-agent', settings: [], options: [] },
  { kind: 'opencode-acp', name: 'opencode', installed: false, level: 3, program: 'opencode', settings: [], options: [] },
  // ADR-0092: ACP registry agents (untested in Agora): one the machine can start through npx, two it cannot find.
  { kind: 'registry-qwen-code', name: 'Qwen Code', installed: true, level: 3, program: 'npx', settings: [], options: [] },
  { kind: 'registry-goose', name: 'goose', installed: false, level: 3, program: 'goose', settings: [], options: [] },
  { kind: 'registry-kiro', name: 'Kiro CLI', installed: false, level: 3, program: 'kiro-cli', settings: [], options: [] },
];

// Settings → Shared on demo data (ADR-0122): a program whose environment names a secret (refused on every machine until the launcher
// comes), a remote server, and one limited to a project that one machine does not allow. Nothing is signed or sent.
export const demoSharedProjects = [{ projectId: 'login-fix', label: 'login-fix' }, { projectId: 'docs-site', label: 'docs-site' }];
export function demoSharedSource(): import('./net/share').SharedSource {
  let items: import('./net/share').SharedItem[] = [
    { name: 'docs-search', kind: 'mcp_server', hash: 'a1', issuedMs: demoSettingsNow - 3_600_000, signer: 'web-10a3e055eea9e45f',
      scope: { projectId: '', kinds: [] }, mcp: { command: 'npx', args: ['-y', 'docs-mcp'], env: [{ name: 'DOCS_TOKEN', value: '{secret:docs}' }], url: '', headers: [] },
      refused: [{ node: 'blackview', code: 'share.refused', reason: 'secret_needs_launcher' }, { node: 'mac', code: 'share.refused', reason: 'secret_needs_launcher' }] },
    { name: 'issues', kind: 'mcp_server', hash: 'b2', issuedMs: demoSettingsNow - 86_400_000, signer: 'web-10a3e055eea9e45f',
      scope: { projectId: '', kinds: [] }, mcp: { command: '', args: [], env: [], url: 'https://mcp.example.com/issues', headers: [] }, refused: [] },
    { name: 'pg-schema', kind: 'mcp_server', hash: 'c3', issuedMs: demoSettingsNow - 7_200_000, signer: 'web-10a3e055eea9e45f',
      scope: { projectId: 'login-fix', kinds: [] }, mcp: { command: 'uvx', args: ['pg-schema-mcp'], env: [], url: '', headers: [] },
      refused: [{ node: 'win', code: 'share.refused', reason: 'command_not_allowed' }] },
  ];
  return {
    list: async () => items,
    publish: async (item, scope) => {
      items = [...items.filter((x) => x.name !== item.name), { name: item.name, kind: item.kind, hash: 'demo', issuedMs: Date.now(), signer: 'demo', scope, mcp: item.mcp, refused: [] }]
        .sort((a, b) => a.name.localeCompare(b.name));
      return { ok: true };
    },
    remove: async (item) => { items = items.filter((x) => x.name !== item.name); return { ok: true }; },
  };
}
