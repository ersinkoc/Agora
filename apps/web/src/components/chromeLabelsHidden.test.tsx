import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { demoStartApproval } from '../fixtures';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { alwaysText } from '../net/always';
import type { ApprovalModel, MessageModel } from '../net/view';
import { PhoneHome } from '../phone/Phone';
import { AgentAddress } from './AgentAddress';
import { AlwaysNote } from './AllowAlways';
import { ApprovalCard } from './ApprovalCard';
import { TeamChat } from './TeamChat';

const NOW = 1_000_000;
const noop = () => undefined;
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'codex', kind: 'codex', accent: '#fff', machine: 'laptop', machineId: 'n1',
  workspace: '/work/shop', project: 'shop', state: 'needs-you', preview: '', ...over,
});
const ask = (over: Partial<ApprovalModel> = {}): ApprovalModel => ({
  id: 'r1', node: 'n1', localId: 'r1', agentId: 'n1/a1', agentName: 'codex', machineName: 'laptop', action: 'Edit', detail: 'src/a.ts', expiresAtMs: 0, ...over,
});
const phone = (a: AgentTileModel) => renderToStaticMarkup(<PhoneHome
  agents={[a]}
  machines={[{ id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 1, lastSeenMs: NOW }]}
  approvals={[ask()]}
  onResolve={noop}
  link="live"
  onOpen={noop}
/>);
const message = (via: string): MessageModel => ({ from: 'codex', to: 'claude', body: 'done', atMs: NOW, crossMachine: false, via });

beforeEach(() => setLanguage('en'));

describe('remaining chrome labels make hidden characters visible', () => {
  it('writes hidden characters in Review-card action names as escapes', () => {
    const html = renderToStaticMarkup(<ApprovalCard approval={demoStartApproval as ApprovalModel} agent={{ name: 'co\u202edex', kind: 'codex', accent: '#fff' }} onOpen={noop} />);
    expect(html).toContain(`aria-label="${t('approval-open-agent', { name: 'co\\u{202e}dex' })}"`);
    expect(html).toContain(`aria-label="${t('approval-review-for', { name: 'co\\u{202e}dex' })}"`);
  });

  it('writes hidden characters in phone request open-action names as escapes', () => {
    expect(phone(agent({ name: 'co\u3164dex' }))).toContain(`aria-label="${t('approval-open-agent', { name: 'co\\u{3164}dex' })}"`);
  });

  it('writes hidden characters in Allow-always agent names as escapes', () => {
    const html = renderToStaticMarkup(<AlwaysNote approval={ask({ agentName: 'co\u2066dex', always: { label: 'Keep', optionId: 'o1', only: false } })} surface="card" />);
    expect(html).toContain(t(alwaysText('approval-allow-always-note'), { name: 'co\\u{2066}dex' }));
  });

  it('writes hidden characters in parent-agent names as escapes', () => {
    expect(renderToStaticMarkup(<AgentAddress agent={agent({ parentName: 'co\u034fdex' })} />)).toContain(t('helper-started-by', { name: 'co\\u{34f}dex' }));
  });

  it('writes hidden characters in team-message relay names as escapes', () => {
    expect(renderToStaticMarkup(<TeamChat messages={[message('lap\u200btop')]} now={NOW} />)).toContain(t('message-via', { machine: 'lap\\u{200b}top' }));
  });
});

describe('remaining chrome label controls', () => {
  it('keeps ordinary Review-card actions', () => {
    const html = renderToStaticMarkup(<ApprovalCard approval={demoStartApproval as ApprovalModel} agent={{ name: 'codex', kind: 'codex', accent: '#fff' }} onOpen={noop} />);
    expect(html).toContain(`aria-label="${t('approval-open-agent', { name: 'codex' })}"`);
    expect(html).toContain(`aria-label="${t('approval-review-for', { name: 'codex' })}"`);
  });

  it('keeps an ordinary phone request open action', () => {
    expect(phone(agent())).toContain(`aria-label="${t('approval-open-agent', { name: 'codex' })}"`);
  });

  it('keeps an ordinary Allow-always agent name and option label', () => {
    const html = renderToStaticMarkup(<AlwaysNote approval={ask({ always: { label: 'Keep', optionId: 'o1', only: false } })} surface="card" />);
    expect(html).toContain(t(alwaysText('approval-allow-always-note'), { name: 'codex' }));
    expect(html).toContain('Keep');
  });

  it('keeps an ordinary parent-agent name and raw address', () => {
    const html = renderToStaticMarkup(<AgentAddress agent={agent({ parentName: 'parent' })} />);
    expect(html).toContain(t('helper-started-by', { name: 'parent' }));
    expect(html).toContain('n1/a1');
  });

  it('keeps an ordinary team-message relay and body', () => {
    const html = renderToStaticMarkup(<TeamChat messages={[message('laptop')]} now={NOW} />);
    expect(html).toContain(t('message-via', { machine: 'laptop' }));
    expect(html).toContain('done');
  });
});

const verifierEdges = [
  ['\u2069', '\\u{2069}'],
  ['\u0085', '\\u{85}'],
  ['\u{f0000}', '\\u{f0000}'],
] as const;

describe('remaining chrome label verifier edges', () => {
  it('escapes other hidden characters in Review-card actions without changing the request', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<ApprovalCard approval={demoStartApproval as ApprovalModel} agent={{ name: `co${hidden}dex`, kind: 'codex', accent: '#fff' }} onOpen={noop} />);
      expect(html).toContain(`aria-label="${t('approval-open-agent', { name: `co${escaped}dex` })}"`);
      expect(html).toContain(`aria-label="${t('approval-review-for', { name: `co${escaped}dex` })}"`);
      expect(html).toContain('/home/dev/code/login-fix');
    }
  });

  it('escapes other hidden characters in phone request open actions without changing the request', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = phone(agent({ name: `co${hidden}dex` }));
      expect(html).toContain(`aria-label="${t('approval-open-agent', { name: `co${escaped}dex` })}"`);
      expect(html).toContain('src/a.ts');
    }
  });

  it('escapes other hidden characters in Allow-always agent names without changing the option', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<AlwaysNote approval={ask({ agentName: `co${hidden}dex`, always: { label: 'Keep', optionId: 'o1', only: false } })} surface="card" />);
      expect(html).toContain(t(alwaysText('approval-allow-always-note'), { name: `co${escaped}dex` }));
      expect(html).toContain('Keep');
    }
  });

  it('escapes other hidden characters in parent-agent names without changing the address', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<AgentAddress agent={agent({ parentName: `co${hidden}dex` })} />);
      expect(html).toContain(t('helper-started-by', { name: `co${escaped}dex` }));
      expect(html).toContain('n1/a1');
    }
  });

  it('escapes other hidden characters in relay names without changing the message body', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<TeamChat messages={[message(`ma${hidden}chine`)]} now={NOW} />);
      expect(html).toContain(t('message-via', { machine: `ma${escaped}chine` }));
      expect(html).toContain('done');
    }
  });
});
