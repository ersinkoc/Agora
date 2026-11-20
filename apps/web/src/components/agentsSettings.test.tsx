// @vitest-environment node
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { DevicePage } from '../screens';

describe('Settings → Agents', () => {
  const machines = [{ id: 'laptop', name: 'laptop', os: 'linux', online: true, agentsRunning: 0, lastSeenMs: 1 }];
  it('is a section of the page, linked as #/device/agents, only when the machines can be asked', () => {
    const none = renderToStaticMarkup(<DevicePage device={undefined} onBack={() => {}} section="agents" machines={machines} />);
    expect(none).not.toContain('#/device/agents');
    const some = renderToStaticMarkup(<DevicePage device={undefined} onBack={() => {}} section="agents" machines={machines} loadKinds={async () => []} />);
    expect(some).toContain('href="#/device/agents"');
    expect(some).toContain('id="settings-agents"');
  });
});
