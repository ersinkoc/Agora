// @vitest-environment jsdom
import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it } from 'vitest';
import { setLanguage } from '../i18n';
import type { AgentTileModel } from '../model';
import type { MachineModel } from '../net/view';
import { Details } from './Inspector';
import { MachineFacts } from './machineFacts';
import { NetworkView } from './NetworkView';

const NOW = 1_000_000;
const edges = [['\u2069', '\\u{2069}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const;
const machine = (name: string, build = ''): MachineModel => ({ id: 'n1', name, build, os: 'linux', online: true, agentsRunning: 0, lastSeenMs: NOW });
const agent = (over: Partial<AgentTileModel> = {}): AgentTileModel => ({
  id: 'n1/a1', name: 'codex', kind: 'codex', accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: '/work/login', state: 'needs-you', preview: '', filesChanged: 2, ...over,
});
const details = (tab: 'needs' | 'changes', over: Partial<Parameters<typeof Details>[0]> = {}) => renderToStaticMarkup(<Details
  tab={tab} fleet={[]} attention={[]} approvals={[]} changed={[]} boards={[]} messages={[]} onOpen={() => {}} {...over}
/>);

beforeEach(() => setLanguage('en'));

describe('secondary owner-visible labels make hidden characters visible', () => {
  it('writes hidden characters in the machine menu accessible name as escapes', () => {
    const html = renderToStaticMarkup(<NetworkView now={NOW} machines={[machine('lap‮top')]} agents={[]} hubName="hub" />);
    expect(html).toContain('aria-label="More for lap\\u{202e}top"');
  });

  it('writes hidden characters in the machine build as escapes', () => {
    expect(renderToStaticMarkup(<MachineFacts now={NOW} m={machine('laptop', 'v1ㅤbeta')} />)).toContain('v1\\u{3164}beta');
  });

  it('writes hidden characters in Needs-you agent names as escapes', () => {
    expect(details('needs', { attention: [agent({ name: 'co⁦dex' })] })).toContain('co\\u{2066}dex');
  });

  it('writes hidden characters in Changes agent names as escapes', () => {
    expect(details('changes', { changed: [agent({ name: 'co​dex' })] })).toContain('co\\u{200b}dex');
  });

  it('writes hidden characters in Changes location labels as escapes', () => {
    const html = details('changes', { changed: [agent({ machine: 'lap͏top', workspace: '/wo⁦rk' })] });
    expect(html).toContain('lap\\u{34f}top');
    expect(html).toContain('/wo\\u{2066}rk');
  });
});

describe('secondary owner-visible label controls', () => {
  it('keeps an ordinary machine menu name and machine id', () => {
    const html = renderToStaticMarkup(<NetworkView now={NOW} machines={[machine('laptop')]} agents={[]} hubName="hub" />);
    expect(html).toContain('aria-label="More for laptop"');
    expect(html).toContain('machine-n1');
  });

  it('keeps an ordinary build and heard time', () => {
    const html = renderToStaticMarkup(<MachineFacts now={NOW} m={machine('laptop', 'v1.2.3')} />);
    expect(html).toContain('v1.2.3');
    expect(html).toContain('heard now');
  });

  it('keeps an ordinary Needs-you name and state', () => {
    const html = details('needs', { attention: [agent()] });
    expect(html).toContain('codex');
    expect(html).toContain('Needs you');
  });

  it('keeps an ordinary Changes name and file count', () => {
    const html = details('changes', { changed: [agent()] });
    expect(html).toContain('codex');
    expect(html).toContain('2 files');
  });

  it('keeps ordinary Changes locations and the agent action', () => {
    const html = details('changes', { changed: [agent()] });
    expect(html).toContain('laptop');
    expect(html).toContain('/work/login');
    expect(html).toContain('button');
  });
});

describe('secondary owner-visible label multi-edge verifier', () => {
  it('covers machine menu names without changing machine identity', () => {
    for (const [char, escaped] of edges) {
      const html = renderToStaticMarkup(<NetworkView now={NOW} machines={[machine(`ma${char}chine`)]} agents={[]} hubName="hub" />);
      expect(html).toContain(`aria-label="More for ma${escaped}chine"`);
      expect(html).toContain('machine-n1');
    }
  });

  it('covers build labels without changing the heard time', () => {
    for (const [char, escaped] of edges) {
      const html = renderToStaticMarkup(<MachineFacts now={NOW} m={machine('laptop', `v1${char}beta`)} />);
      expect(html).toContain(`v1${escaped}beta`);
      expect(html).toContain('heard now');
    }
  });

  it('covers Needs-you names without changing their state', () => {
    for (const [char, escaped] of edges) {
      const html = details('needs', { attention: [agent({ name: `co${char}dex` })] });
      expect(html).toContain(`co${escaped}dex`);
      expect(html).toContain('Needs you');
    }
  });

  it('covers Changes names without changing the file count', () => {
    for (const [char, escaped] of edges) {
      const html = details('changes', { changed: [agent({ name: `co${char}dex` })] });
      expect(html).toContain(`co${escaped}dex`);
      expect(html).toContain('2 files changed');
    }
  });

  it('covers Changes locations without changing the agent row', () => {
    for (const [char, escaped] of edges) {
      const html = details('changes', { changed: [agent({ machine: `ma${char}chine`, workspace: `/wo${char}rk` })] });
      expect(html).toContain(`ma${escaped}chine`);
      expect(html).toContain(`/wo${escaped}rk`);
      expect(html).toContain('codex');
    }
  });
});
