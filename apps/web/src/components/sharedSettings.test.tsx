// @vitest-environment jsdom
// Settings → Shared (ADR-0122): what each machine refused is said in words on the item, a publish signs exactly what the form shows, a
// form the rules refuse is never sent, an address warns before it is shared, and a removal is asked first.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLanguage } from '../i18n';
import type { ShareItemDef, ShareScopeDef, SharedItem, SharedSource } from '../net/share';
import { draftItem, SharedSettings } from './SharedSettings';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const docs: SharedItem = {
  name: 'docs-search', kind: 'mcp_server', hash: 'a1', issuedMs: 1000, signer: 'web-1', scope: { projectId: '', kinds: [] },
  mcp: { command: 'npx', args: ['-y', 'docs-mcp'], env: [{ name: 'DOCS_TOKEN', value: '{secret:docs}' }], url: '', headers: [] },
  refused: [{ node: 'n-blackview', code: 'share.refused', reason: 'secret_needs_launcher' }, { node: 'n-win', code: 'share.refused', reason: 'signature' }],
};

function fake(items: SharedItem[]) {
  const published: { item: ShareItemDef; scope: ShareScopeDef; after?: number }[] = [];
  const removed: string[] = [];
  const source: SharedSource = {
    list: vi.fn(async () => items),
    publish: vi.fn(async (item, scope, after) => { published.push({ item, scope, after }); return { ok: true as const }; }),
    remove: vi.fn(async (item) => { removed.push(item.name); return { ok: true as const }; }),
  };
  return { source, published, removed };
}

let root: Root;
let host: HTMLDivElement;
beforeEach(() => { setLanguage('en'); host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
afterEach(() => { act(() => root.unmount()); host.remove(); });

async function render(source: SharedSource, canSign = true) {
  await act(async () => {
    root.render(<SharedSettings source={source} canSign={canSign} machineName={(id) => id.replace('n-', '')} projects={[{ projectId: 'p1', label: 'login-fix' }]} />);
  });
}
const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text);
async function click(el: Element | undefined) { expect(el).toBeTruthy(); await act(async () => { (el as HTMLElement).click(); }); }
async function type(label: string, value: string) {
  const input = [...host.querySelectorAll('input, textarea')].find((i) => host.querySelector(`label[for="${i.id}"]`)?.textContent === label) as HTMLInputElement;
  expect(input, label).toBeTruthy();
  const proto = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('Settings → Shared', () => {
  it("says on the item, in words, why each machine does not give it to its agents", async () => {
    await render(fake([docs]).source);
    const text = host.textContent ?? '';
    expect(text).toContain('npx -y docs-mcp');
    expect(text).toContain('Every project');
    expect(text).toContain('Refused on 2 machines');
    expect(text).toContain('blackview: needs a secret: not given to agents yet');
    expect(text).toContain('win: this machine does not trust the device that published it');
  });

  it('signs and shares what the form says, for the chosen project, and lists it again', async () => {
    const f = fake([]);
    await render(f.source);
    expect(host.textContent).toContain('Nothing is shared yet');
    await click(button('Add a server'));
    await type('Name', 'pg-schema');
    await type('Program', 'uvx');
    await type('Arguments', 'pg-schema-mcp\n--read-only\n');
    await type('Environment', 'PGHOST=db.local');
    await act(async () => {
      const select = host.querySelector('select') as HTMLSelectElement;
      select.value = 'p1';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await click(button('Sign and share'));
    expect(f.published).toEqual([{
      item: { name: 'pg-schema', kind: 'mcp_server', mcp: { command: 'uvx', args: ['pg-schema-mcp', '--read-only'], env: [{ name: 'PGHOST', value: 'db.local' }], url: '', headers: [] } },
      scope: { projectId: 'p1', kinds: [] }, after: undefined,
    }]);
    expect(f.source.list).toHaveBeenCalledTimes(2);
    expect(host.querySelector('form')).toBeNull();
  });

  it('warns before an address is shared, and about a secret it names', async () => {
    await render(fake([]).source);
    await click(button('Add a server'));
    await click(button('An address'));
    expect(host.textContent).toContain('Agents may send your code there');
    expect(host.textContent).not.toContain('Not given to agents yet');
    await type('Headers', 'Authorization: Bearer {secret:issues}');
    expect(host.textContent).toContain('Not given to agents yet');
  });

  it('never sends a form the rules refuse, and says what to fix', async () => {
    const f = fake([]);
    await render(f.source);
    await click(button('Add a server'));
    await type('Name', 'Bad Name');
    await click(button('Sign and share'));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain('lowercase letters');
    expect(f.published).toEqual([]);
  });

  it('a change is signed after the word it replaces; a removal is asked first', async () => {
    const f = fake([docs]);
    await render(f.source);
    await click(button('Change'));
    await click(button('Sign and share the change'));
    expect(f.published[0]?.after).toBe(1000);
    await click(button('Remove docs-search'));
    expect(f.removed).toEqual([]);
    await click(button('Stop sharing'));
    expect(f.removed).toEqual(['docs-search']);
  });

  it('without a device key shows the list and offers no change', async () => {
    await render(fake([docs]).source, false);
    expect(button('Add a server')).toBeUndefined();
    expect(button('Change')).toBeUndefined();
    expect(host.textContent).toContain('docs-search');
  });
});

describe('draftItem', () => {
  const base = { name: 'x', runs: 'program' as const, command: 'npx', args: '', env: '', url: '', headers: '', projectId: '' };
  it('keeps a secret off the command line and an address to https', () => {
    expect(draftItem({ ...base, args: '--token={secret:t}' })).toBe('shared-check-args-secret');
    expect(draftItem({ ...base, env: 'NO_SEPARATOR' })).toBe('shared-check-env');
    expect(draftItem({ ...base, runs: 'address', url: 'http://plain.example' })).toBe('shared-check-url');
    expect(draftItem({ ...base, runs: 'address', url: 'https://ok.example/mcp', headers: 'A: 1\nA: 2' })).toBe('shared-check-headers');
    expect(draftItem({ ...base, runs: 'address', url: 'https://ok.example/mcp', command: 'ignored' })).toEqual({
      name: 'x', kind: 'mcp_server', mcp: { command: '', args: [], env: [], url: 'https://ok.example/mcp', headers: [] },
    });
  });
});
