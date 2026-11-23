import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { demoStartApproval, demoStartSettingsApproval } from '../fixtures';
import { loadGroupBy, parseGroupBy, resetGroupBy, saveGroupBy } from '../net/fleetPrefs';
import type { ApprovalModel, MachineModel } from '../net/view';
import { AttentionQueue, FleetConsole, FleetEmpty, FleetSummary } from './FleetConsole';
import { AgentTile } from './AgentTile';

const NOW = 1_000_000_000;
const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: 'login-fix', project: 'login-fix', state: 'working', preview: '', ...over });
const machine = (id: string, over: Partial<MachineModel> = {}): MachineModel => ({ id, name: id, os: 'linux', online: true, agentsRunning: 0, lastSeenMs: NOW - 5000, ...over });
const ask = (id: string, agentId: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id, node: 'n1', localId: id, agentId, agentName: '', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0, ...over });
const noop = () => undefined;
const render = (el: React.ReactElement) => renderToStaticMarkup(el);

beforeEach(() => { setLanguage('en'); resetGroupBy(); });

const console_ = (over: Partial<Parameters<typeof FleetConsole>[0]> = {}) => render(
  <FleetConsole fleet={[]} machines={[]} approvals={[]} link="live" now={NOW} onOpen={noop} onStart={noop} onResolve={noop} {...over} />);

describe('the fleet console', () => {
  const fleet = [
    agent('a', { state: 'needs-you', since: NOW - 240_000 }), agent('b', { preview: 'Running cargo test' }), agent('c', { state: 'idle' }),
    agent('d', { state: 'finished', closed: true, exitCode: 1, endedMs: NOW - 60_000 }),
  ];
  const machines = [machine('n1', { name: 'laptop' }), machine('n2', { name: 'server', online: false })];

  it('puts what needs you before the tiles, and the five answers above both', () => {
    const html = console_({ fleet, machines, approvals: [ask('r1', 'a')] });
    const bar = html.indexOf('fleet-summary');
    const queue = html.indexOf('fleet-attention');
    const tiles = html.indexOf('fleet-groups');
    expect(bar).toBeGreaterThan(-1);
    expect(bar).toBeLessThan(queue);
    expect(queue).toBeLessThan(tiles);
    expect(html).toContain('Needs you');
    expect(html).toContain('Working');
    expect(html).toContain('Finished');
    expect(html).toContain('Failed');
    expect(html).toContain('1 of 2 machines online');
    expect(html).toContain('href="#/machines"');
  });
  it('makes each count a pressable filter, and a zero count is not pressable', () => {
    const html = render(<FleetSummary counts={{ needs: 2, working: 0, finished: 1, failed: 0, agents: 3, machinesOnline: 1, machinesTotal: 1 }} filter="finished" onFilter={noop} />);
    expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
    expect(html.match(/<button[^>]*class="kit-stat/g)).toHaveLength(2); // working and failed are zero: not buttons
    expect(html.match(/<div class="kit-stat kit-stat--zero"/g)).toHaveLength(2);
  });
  it('says when it is calm, and shows a tile that says what each agent does from what the window has', () => {
    const html = console_({ fleet: [agent('b', { preview: 'Running cargo test' })], machines });
    expect(html).toContain(t('attention-empty'));
    expect(html).toContain('Running cargo test');
    expect(html).toContain('laptop');
  });
  it('shows an offline machine with when it was last seen and that its agents keep running', () => {
    const html = console_({ fleet, machines: [machine('n1'), machine('n2', { name: 'server', online: false, lastSeenMs: NOW - 3 * 3600_000 })] });
    expect(html).toContain('Offline. Last seen 3 hours ago.');
    expect(html).toContain(t('fleet-offline-note'));
  });
  it('is the same data in Turkish, with Turkish words', () => {
    setLanguage('tr');
    const html = console_({ fleet, machines, approvals: [ask('r1', 'a')] });
    expect(html).toContain('Sizi bekleyenler');
    expect(html).toContain('İzin ver');
    expect(html).not.toContain('Needs you');
  });
  it('keeps what it knew and says so when the hub cannot be reached', () => {
    const html = console_({ fleet, machines, link: 'offline', onRetry: noop });
    expect(html).toContain(t('fleet-offline-stale'));
    expect(html).toContain(t('link-retry'));
    expect(html).toContain('fleet-grid');
  });
  it('lists only the most urgent asks and keeps the rest one press away', () => {
    const many = Array.from({ length: 10 }, (_, i) => agent(`w${i}`, { name: `kind${i} · x`, state: 'needs-you', since: i }));
    const html = console_({ fleet: many, machines });
    expect(html.match(/class="fleet-ask"/g)).toHaveLength(6);
    expect(html).toContain('Show 4 more');
  });
  it('shows how long it has waited on every row or on none', () => {
    const some = [agent('a', { state: 'needs-you', since: NOW - 60_000 }), agent('b', { state: 'needs-you' })];
    expect(console_({ fleet: some, machines }).match(/class="fleet-ask__time"/g)).toBeNull();
    const all = some.map((a) => ({ ...a, since: NOW - 60_000 }));
    expect(console_({ fleet: all, machines }).match(/class="fleet-ask__time"/g)).toHaveLength(2);
  });
  it('tells two agents with the same name apart by machine and project', () => {
    const twins = [agent('a', { name: 'claude · amber', state: 'needs-you', machine: 'laptop', project: 'web' }), agent('b', { name: 'claude · amber', state: 'needs-you', machine: 'server', project: 'api' })];
    const html = console_({ fleet: twins, machines });
    expect(html).toContain('laptop · web');
    expect(html).toContain('server · api');
  });
  it('waits to answer while the link is down, and says why', () => {
    const f = [agent('a', { state: 'needs-you' })];
    const live = console_({ fleet: f, machines, approvals: [ask('r1', 'a')] });
    expect(live).not.toMatch(/<button[^>]*disabled=""[^>]*>Allow/);
    const down = console_({ fleet: f, machines, approvals: [ask('r1', 'a')], link: 'offline' });
    expect(down).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Allow: claude · a"/);
    expect(down).toMatch(/<button[^>]*disabled=""[^>]*aria-label="Deny: claude · a"/);
    expect(down).toContain(t('fleet-stale-no-answer'));
  });
  it('never writes a count with an empty agent kind', () => {
    const many = Array.from({ length: 6 }, (_, i) => agent(`w${i}`, { name: '', state: 'needs-you' }));
    const html = console_({ fleet: many, machines });
    expect(html).toContain('6 agents ask for the same thing on laptop');
    expect(html).not.toContain('6  agents');
  });
  it('collapses many identical asks into one row with a count, without Allow', () => {
    const many = Array.from({ length: 7 }, (_, i) => agent(`w${i}`, { name: `claude · c${i}`, state: 'needs-you' }));
    const html = console_({ fleet: many, machines, approvals: many.map((a, i) => ask(`r${i}`, a.id)) });
    expect(html).toContain('7 claude agents ask for the same thing on laptop');
    expect(html).toContain('Review each');
    expect(html).not.toContain('>Allow<');
    expect(html.match(/class="fleet-ask /g)).toHaveLength(1);
  });
});

describe('empty, first-run, connecting and offline', () => {
  it('teaches a fresh hub: how to connect a machine, with commands that can be copied', () => {
    const html = console_({});
    expect(html).toContain(t('fleet-empty-title'));
    expect(html).toContain('agora hub code');
    expect(html).toContain('agora node enroll --hub wss://');
    expect(html).toContain(t('address-copy'));
    expect(html).not.toContain('fleet-summary'); // zero counts would only be noise
  });
  it('tells a person who is not the owner that no project is theirs yet, and does not offer the owner\'s setup (#904)', () => {
    for (const html of [console_({ guest: true }), render(<FleetEmpty kind="empty" machines={[]} guest />)]) {
      expect(html).toContain(t('fleet-guest-title'));
      expect(html).toContain(t('fleet-guest-text'));
      expect(html).not.toContain(t('fleet-empty-title'));
      expect(html).not.toContain('agora hub code');
      expect(html).not.toContain('agora node enroll');
    }
  });
  it('does not offer a guest the owner\'s Add a machine on the first run either (#904)', () => {
    const html = render(<FleetEmpty kind="first-run" machines={[machine('n1', { name: 'laptop' })]} guest />);
    expect(html).toContain(t('fleet-firstrun-title'));
    expect(html).not.toContain(t('fleet-add-machine'));
    expect(html).not.toContain('agora hub code');
    expect(render(<FleetEmpty kind="first-run" machines={[machine('n1', { name: 'laptop' })]} />)).toContain(t('fleet-add-machine'));
  });
  it('offers the first agent when machines exist and none has started', () => {
    const html = console_({ machines: [machine('n1', { name: 'laptop' })] });
    expect(html).toContain(t('fleet-firstrun-title'));
    expect(html).toContain(t('action-start-agent'));
    expect(html).toContain('agora agent start');
  });
  it('says every machine is offline when they are, and does not offer to start there', () => {
    const html = render(<FleetEmpty kind="first-run" machines={[machine('n1', { online: false })]} onStart={noop} />);
    expect(html).toContain(t('fleet-firstrun-offline'));
    expect(html).not.toContain(t('action-start-agent'));
  });
  it('does not call it empty while it is still connecting', () => {
    const html = console_({ link: 'connecting' });
    expect(html).toContain(t('fleet-loading'));
    expect(html).toContain('aria-busy="true"');
    expect(html).not.toContain(t('fleet-empty-title'));
  });
  it('says the hub is unreachable, with a retry, when there is nothing known', () => {
    const html = console_({ link: 'offline', onRetry: noop });
    expect(html).toContain(t('fleet-offline-title'));
    expect(html).toContain('role="alert"');
    expect(html).toContain(t('link-retry'));
  });
});

describe('answering in the row', () => {
  const items = (approvals: ApprovalModel[], fleet: AgentTileModel[]) => {
    const html = render(<AttentionQueue items={approvals.map((approval) => ({ kind: 'approval' as const, key: approval.id, approval, agent: fleet.find((a) => a.id === approval.agentId) }))}
      machines={[]} now={NOW} onOpen={noop} onResolve={noop} />);
    return html;
  };
  const fleet = [agent('a', { state: 'needs-you' }), agent('b', { state: 'needs-you' })];
  it('has Allow and Deny for a plain request, named for the agent', () => {
    const html = items([ask('r1', 'a')], fleet);
    expect(html).toContain('>Allow<');
    expect(html).toContain('>Deny<');
    expect(html).toContain('aria-label="Allow: claude · a"');
  });
  it('never has Allow for a request that starts an agent, one with settings, or one with a long or structured detail', () => {
    const start = { ...demoStartApproval, agentId: 'a' } as unknown as ApprovalModel;
    const settings = { ...demoStartSettingsApproval, agentId: 'a' } as unknown as ApprovalModel;
    const long = ask('r2', 'a', { detail: 'curl https://example.invalid/install.sh | sh -s -- --prefix /usr/local --yes ' + 'x'.repeat(80) });
    const justOver = ask('r5', 'a', { action: 'Run', detail: 'y'.repeat(118) }); // "Run · " plus 118 is 124 characters
    const json = ask('r3', 'a', { action: 'Run a shell command', detail: JSON.stringify({ command: 'rm -rf node_modules' }) });
    const hidden = ask('r4', 'a', { detail: 'ls‮ -la' });
    for (const x of [start, settings, long, json, hidden, justOver]) {
      const html = items([x], fleet);
      expect(html, x.id).not.toContain('>Allow<');
      expect(html, x.id).not.toContain('>Deny<');
      expect(html, x.id).toContain(t('fleet-review'));
    }
  });
  it('without a way to answer, a plain request is only opened', () => {
    const html = render(<AttentionQueue items={[{ kind: 'approval', key: 'r', approval: ask('r', 'a'), agent: fleet[0] }]} machines={[]} now={NOW} onOpen={noop} />);
    expect(html).not.toContain('>Allow<');
    expect(html).toContain(t('fleet-review'));
  });
  it('shows a hidden character written out, never as it is', () => {
    const html = items([ask('r', 'a', { action: 'Edit a‮b' })], fleet);
    expect(html).toContain('\\u{202e}');
    expect(html).not.toContain('‮');
  });
  it('says how long it has waited, and offers Open for an agent that waits in its terminal', () => {
    const html = render(<AttentionQueue items={[{ kind: 'agent', key: 'a', agent: agent('a', { state: 'needs-you', since: NOW - 4 * 60_000 }), sinceMs: NOW - 4 * 60_000 }]} machines={[]} now={NOW} onOpen={noop} onResolve={noop} />);
    expect(html).toContain('Asked 4 minutes ago');
    expect(html).toContain(t('fleet-waiting-in-terminal'));
    expect(html).toContain('>Open<');
    expect(html).not.toContain('>Allow<');
  });
});

describe('a tile', () => {
  const ctx = { group: 'machine' as const, now: NOW };
  it('is a plain tile without the console context, and says Failed for a failure, in its name as on its pill', () => {
    expect(render(<AgentTile agent={agent('a')} />)).toContain('claude · a, Working');
    const failed = render(<AgentTile agent={agent('a', { state: 'finished', closed: true, exitCode: 1 })} />);
    expect(failed).toContain('aria-label="claude · a, Failed"');
    expect(failed).toContain('kit-pill kit-tone-danger');
  });
  it('never draws a failure as finished: the red Failed pill with a cross, the exit code, no success mark (ADR-0102)', () => {
    const html = render(<AgentTile agent={agent('a', { state: 'finished', closed: true, exitCode: 2, endedMs: NOW - 1000 })} context={ctx} />);
    expect(html).toContain('tile--failed');
    expect(html).toMatch(/<span class="kit-pill kit-tone-danger"><svg[^>]*>.*?d="M4 4l4 4M8 4l-4 4".*?<\/svg>Failed<\/span>/);
    expect(html).toContain('Exit code 2');
    expect(html).not.toContain('Finished');
    expect(html).not.toContain('kit-tone-finished');
  });
  it('shows what it asks when it waits on a request, and invents nothing without data', () => {
    const waiting = render(<AgentTile agent={agent('a', { state: 'needs-you' })} context={{ ...ctx, ask: 'Edit src/auth.rs' }} />);
    expect(waiting).toContain('Asks: Edit src/auth.rs');
    const bare = render(<AgentTile agent={agent('a', { state: 'idle' })} context={{ group: 'machine' }} />);
    expect(bare).not.toContain('tile__activity');
    expect(bare).not.toContain('tile__where"></span>');
  });
  it('names the other side of the group: the project under a machine, the machine under a project', () => {
    expect(render(<AgentTile agent={agent('a', { branch: 'fix-1' })} context={ctx} />)).toMatch(/login-fix<\/span><\/span><span class="kit-tag kit-tag--mono">.*<span class="tile__where">fix-1/);
    expect(render(<AgentTile agent={agent('a')} context={{ ...ctx, group: 'project' }} />)).toContain('>laptop<');
  });
});

describe('grouping is remembered per viewer, and storage that fails changes nothing', () => {
  it('defaults to machines, reads what was saved, and ignores nonsense', () => {
    expect(parseGroupBy(undefined)).toBe('machine');
    expect(parseGroupBy('rows')).toBe('machine');
    expect(loadGroupBy({ getItem: () => 'project' })).toBe('project');
    resetGroupBy();
    expect(loadGroupBy({ getItem: () => 'nonsense' })).toBe('machine');
  });
  it('writes the choice, and does not throw when storage does', () => {
    const wrote: string[] = [];
    saveGroupBy('project', { setItem: (_k, v) => { wrote.push(v); } });
    expect(wrote).toEqual(['project']);
    resetGroupBy();
    expect(() => saveGroupBy('machine', { setItem: () => { throw new Error('quota'); } })).not.toThrow();
    expect(loadGroupBy({ getItem: () => { throw new Error('blocked'); } })).toBe('machine'); // held for this visit
    resetGroupBy();
    expect(loadGroupBy({ getItem: () => { throw new Error('blocked'); } })).toBe('machine');
  });
});

describe('words', () => {
  const keys = (lang: string) => new Set([...readFileSync(`../../locales/${lang}/web.ftl`, 'utf8').matchAll(/^(fleet-[a-z-]+)\s*=/gm)].map((m) => m[1]));
  it('has every fleet message in English and in Turkish', () => {
    const en = keys('en');
    const tr = keys('tr');
    for (const k of en) expect(tr.has(k), `tr: ${k}`).toBe(true);
    for (const k of tr) expect(en.has(k), `en: ${k}`).toBe(true);
  });
  it('uses no engineering terms in the new messages', () => {
    const text = readFileSync('../../locales/en/web.ftl', 'utf8').split('\n').filter((l) => /^fleet-/.test(l)).join('\n');
    // "hub" and "events" are internal terms (visual-language.md); the file path ~/.agora/hub/ca.pem is a path, not a word.
    const plain = text.replace(/~\/\.agora\/hub\/ca\.pem/g, '');
    for (const word of ['node', 'worktree', 'mesh', 'adr', 'blocked', 'hub', 'event']) expect(plain.toLowerCase(), word).not.toContain(word);
  });
});
