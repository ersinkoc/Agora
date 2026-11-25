// @vitest-environment jsdom
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import type { AgentTileModel } from '../model';
import { AgentAddress } from './AgentAddress';

const agent = (id: string): AgentTileModel => ({
  id, name: 'codex · amber', accent: '#fff', kind: 'codex', machine: 'Laptop', machineId: 'n1', workspace: '/w', state: 'idle', preview: '',
});

describe('agent address', () => {
  it('makes the hub-qualified address visible', () => {
    const html = renderToStaticMarkup(<AgentAddress agent={agent('n1/co\u3164dex\u202e')} />);
    expect(html).toContain('n1/co\\u{3164}dex\\u{202e}');
    expect(html).not.toContain('\u3164');
    expect(html).not.toContain('\u202e');
  });

  it('verifies the visible address while copying the raw routing identity', async () => {
    const raw = 'n1/ça\u2066t\ue000';
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    const host = document.createElement('div');
    const root = createRoot(host);
    await act(async () => { root.render(<AgentAddress agent={agent(raw)} />); });
    expect(host.textContent).toContain('n1/ça\\u{2066}t\\u{e000}');
    await act(async () => { (host.querySelector('button') as HTMLButtonElement).click(); });
    expect(writeText).toHaveBeenCalledWith(raw);
    act(() => root.unmount());
  });
});
