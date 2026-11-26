import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { ApprovalModel, MachineModel } from '../net/view';
import { ApprovalCard } from './ApprovalCard';
import { AttentionQueue } from './FleetConsole';
import { MachineName } from './machineFacts';

const NOW = 1_000_000;
const noop = () => undefined;
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'codex', kind: 'codex', accent: '#fff', machine: 'laptop', machineId: 'n1',
  workspace: '/work/shop', project: 'shop', state: 'needs-you', preview: '', ...over,
});
const ask = (over: Partial<ApprovalModel> = {}): ApprovalModel => ({
  id: 'r1', node: 'n1', localId: 'r1', agentId: 'n1/a1', agentName: 'codex', action: 'Edit', detail: 'src/a.ts', expiresAtMs: 0, ...over,
});
const attention = (a: AgentTileModel) => renderToStaticMarkup(<AttentionQueue
  items={[{ kind: 'approval', key: 'r1', approval: ask(), agent: a }]}
  machines={[]}
  now={NOW}
  onOpen={noop}
  onResolve={noop}
/>);

beforeEach(() => setLanguage('en'));

describe('attention owner-visible labels make hidden characters visible', () => {
  it('writes hidden characters in Fleet attention agent labels as escapes', () => {
    const html = attention(agent({ name: 'co\u202edex' }));
    expect(html).toContain('co\\u{202e}dex wants to change a file');
    expect(html).toContain('aria-label="Allow: co\\u{202e}dex"');
  });

  it('writes hidden characters in Fleet attention machine locations as escapes', () => {
    expect(attention(agent({ machine: 'lap\u3164top' }))).toContain('lap\\u{3164}top');
  });

  it('writes hidden characters in Fleet attention project locations as escapes', () => {
    expect(attention(agent({ project: 'pro\u2066ject' }))).toContain('pro\\u{2066}ject');
  });

  it('writes hidden characters in approval-card agent labels as escapes', () => {
    const html = renderToStaticMarkup(<ApprovalCard approval={ask()} agent={{ name: 'co\u034fdex', kind: 'codex', accent: '#fff' }} onOpen={noop} />);
    expect(html).toContain('co\\u{34f}dex wants to change a file');
    expect(html).toContain('aria-label="Open co\\u{34f}dex"');
  });

  it('writes hidden characters in displayed machine ids as escapes', () => {
    const html = renderToStaticMarkup(<MachineName m={{ id: 'n\u200b1', name: 'laptop' }} />);
    expect(html).toContain('n\\u{200b}1');
  });
});

describe('attention owner-visible label controls', () => {
  it('keeps an ordinary Fleet attention agent label and action', () => {
    const html = attention(agent());
    expect(html).toContain('codex wants to change a file');
    expect(html).toContain('aria-label="Allow: codex"');
  });

  it('keeps an ordinary Fleet attention machine location', () => {
    expect(attention(agent())).toContain('laptop · shop');
  });

  it('keeps an ordinary Fleet attention project location', () => {
    expect(attention(agent({ project: 'store' }))).toContain('laptop · store');
  });

  it('keeps an ordinary approval-card label and open action', () => {
    const html = renderToStaticMarkup(<ApprovalCard approval={ask()} agent={{ name: 'codex', kind: 'codex', accent: '#fff' }} onOpen={noop} />);
    expect(html).toContain('codex wants to change a file');
    expect(html).toContain('aria-label="Open codex"');
  });

  it('keeps an ordinary machine name and distinct id', () => {
    const html = renderToStaticMarkup(<MachineName m={{ id: 'n1', name: 'laptop' }} />);
    expect(html).toContain('>laptop<');
    expect(html).toContain('>n1<');
  });
});

const verifierEdges = [
  ['\u2069', '\\u{2069}'],
  ['\u0085', '\\u{85}'],
  ['\u{f0000}', '\\u{f0000}'],
] as const;

describe('attention owner-visible label verifier edges', () => {
  it('escapes other hidden characters in Fleet attention agent labels without changing the request', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = attention(agent({ name: `co${hidden}dex` }));
      expect(html).toContain(`co${escaped}dex wants to change a file`);
      expect(html).toContain(`aria-label="Allow: co${escaped}dex"`);
      expect(html).toContain('Edit · src/a.ts');
    }
  });

  it('escapes other hidden characters in Fleet attention machine locations without changing the project', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = attention(agent({ machine: `ma${hidden}chine` }));
      expect(html).toContain(`ma${escaped}chine · shop`);
      expect(html).toContain('aria-label="Allow: codex"');
    }
  });

  it('escapes other hidden characters in Fleet attention project locations without changing the machine', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = attention(agent({ project: `pro${hidden}ject` }));
      expect(html).toContain(`laptop · pro${escaped}ject`);
      expect(html).toContain('aria-label="Allow: codex"');
    }
  });

  it('escapes other hidden characters in approval-card agent labels without changing the request', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<ApprovalCard approval={ask()} agent={{ name: `co${hidden}dex`, kind: 'codex', accent: '#fff' }} onOpen={noop} />);
      expect(html).toContain(`co${escaped}dex wants to change a file`);
      expect(html).toContain(`aria-label="Open co${escaped}dex"`);
      expect(html).toContain('Edit · src/a.ts');
    }
  });

  it('escapes other hidden characters in displayed machine ids without changing the machine name', () => {
    for (const [hidden, escaped] of verifierEdges) {
      const html = renderToStaticMarkup(<MachineName m={{ id: `n${hidden}1`, name: 'laptop' }} />);
      expect(html).toContain(`n${escaped}1`);
      expect(html).toContain('>laptop<');
    }
  });
});
