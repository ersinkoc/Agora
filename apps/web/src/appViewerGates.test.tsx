// @vitest-environment jsdom
// ADR-0106 (#1140): each viewer gate of the whole window, one by one. The window is connected to a stand-in hub whose first frame names the
// person's role, with a running agent that waits on a request, its helper, and a chat agent. A viewer's window offers none of what the hub
// would refuse (and write down in the owner's Activity); a member's window, on the same hub, does, so every "none" here is a real one.
// An older hub (no `Welcome`, role unspecified) keeps the window as it was.
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from './gen/agora.js';
import { App } from './App';
import { forgetLoaded } from './layout/store';
import { attentionPrefs } from './net/attention';
import { CONTROL_KIND, decodeFrame, encodeFrame, EVENT_KIND, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS } from './net/frame';
import { PHONE_QUERY } from './phone/nav';
import { setLanguage, t } from './i18n';

// This browser's device key, held in memory (jsdom has no IndexedDB): the devices settings list the other devices only with one.
vi.mock('./net/approval', async (original) => {
  const m = await original<typeof import('./net/approval')>();
  let held: { id: string; keys: CryptoKeyPair } | undefined;
  return {
    ...m,
    indexedDbStore: () => ({
      load: async () => (held ??= { id: 'web-me', keys: await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign', 'verify']) }),
      save: async (d: { id: string; keys: CryptoKeyPair }) => { held = d; },
    }),
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const v1 = agora.v1;
const { OWNER, MEMBER, VIEWER, UNSPECIFIED } = {
  OWNER: v1.PersonRole.PERSON_ROLE_OWNER, MEMBER: v1.PersonRole.PERSON_ROLE_MEMBER, VIEWER: v1.PersonRole.PERSON_ROLE_VIEWER,
  UNSPECIFIED: v1.PersonRole.PERSON_ROLE_UNSPECIFIED,
};

/** The hub: answers the device list (another device of this person) and nothing else, and keeps what the window sent. */
class HubSocket {
  static last: HubSocket | undefined;
  binaryType = '';
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: Uint8Array[] = [];
  constructor(readonly url: string, readonly protocols: string[]) { HubSocket.last = this; }
  send(d: Uint8Array) {
    this.sent.push(d);
    const f = decodeFrame(d);
    if (!f || f.streamId !== STREAM_COMMANDS || f.kind !== 0) return;
    const r = v1.CommandRequest.decode(f.payload);
    const ok = r.deviceList ? { deviceList: { devices: [{ deviceId: 'web-me', name: 'This browser' }, { deviceId: 'web-other', name: 'Phone' }] } }
      : r.deviceRegister ? {} : undefined;
    if (ok) queueMicrotask(() => this.deliver(STREAM_COMMANDS, 1, v1.CommandResponse.encode({ requestId: r.requestId, ok }).finish()));
  }
  close() {}
  deliver(streamId: number, kind: number, payload: Uint8Array) {
    const b = encodeFrame({ streamId, kind, payload });
    this.onmessage?.({ data: b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) });
  }
}

const OLD_MS = Date.now() - 90 * 24 * 3600 * 1000;
const fleet = (): agora.v1.Snapshot.$Properties => ({
  pos: 3,
  nodes: [
    { nodeId: 'n1', hostname: 'Box', online: true, trustedDeviceIds: ['web-me'] },
    { nodeId: 'n2', hostname: 'Gone', online: false, lastEventMs: OLD_MS },
  ],
  agents: [
    { agentId: 'n1/a1', nodeId: 'n1', kind: 'codex', running: true },
    { agentId: 'n1/h1', nodeId: 'n1', kind: 'codex', running: true, parentAgentId: 'n1/a1' },
    { agentId: 'n1/c1', nodeId: 'n1', kind: 'claude', running: true, modes: [{ id: 'default', name: 'Default' }, { id: 'plan', name: 'Plan' }], currentMode: 'default' },
  ],
  terminals: [{ terminalId: 't-a1', nodeId: 'n1', agentId: 'n1/a1' }, { terminalId: 't-h1', nodeId: 'n1', agentId: 'n1/h1' }],
  approvals: [{ approvalId: 'n1/r1', agentId: 'n1/a1', action: 'Edit', detail: 'src/auth.rs' }],
});

let host: HTMLElement;
let root: Root;
let focused = false;
let phone = false;
beforeEach(() => {
  setLanguage('en');
  attentionPrefs.reset();
  forgetLoaded();
  focused = false;
  phone = false;
  window.localStorage.setItem('agora.hubToken', 'tok');
  vi.stubGlobal('WebSocket', HubSocket);
  vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { status: 200, headers: { 'content-type': 'application/json' } })));
  // A desktop is a wide window (the agents' tree shows); a phone matches the phone's query only.
  vi.stubGlobal('matchMedia', (q: string) => ({ matches: phone ? q === PHONE_QUERY : q.includes('min-width'), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} }));
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  vi.spyOn(document, 'hasFocus').mockImplementation(() => focused);
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  window.localStorage.clear();
  window.location.hash = '';
});

/** The window at `hash`, connected as `role` (`UNSPECIFIED`: an older hub, which sends no `Welcome`). */
async function windowAs(role: agora.v1.PersonRole, hash = '#/') {
  window.location.hash = hash;
  await act(async () => { root.render(<App />); });
  const s = HubSocket.last as HubSocket;
  await act(async () => {
    s.onopen?.();
    if (role !== UNSPECIFIED) s.deliver(STREAM_CONTROL, CONTROL_KIND.WELCOME, v1.Welcome.encode({ chosenGeneration: 1, personRole: role }).finish());
    s.deliver(STREAM_EVENTS, EVENT_KIND.SNAPSHOT, v1.Snapshot.encode(fleet()).finish());
  });
  await tick(3);
}

const tick = async (n: number) => { for (let i = 0; i < n; i++) await act(async () => { await new Promise((r) => setTimeout(r, 100)); }); };
/** Waits (up to 5 s, in steps the window can draw in) until `check` holds: the device key, the device list and the lazy parts come late. */
async function until(check: () => void) {
  for (let i = 0; i < 50; i++) {
    try { check(); return; } catch { await tick(1); }
  }
  check();
}

/** Another window, as if opened afresh (what the last one kept in this browser, its open tabs, is gone). */
function again() {
  act(() => root.unmount());
  window.localStorage.clear();
  window.localStorage.setItem('agora.hubToken', 'tok');
  forgetLoaded();
  root = createRoot(host);
}

/** The commands this window sent the hub, by their schema field name. */
const asked = () => (HubSocket.last?.sent ?? []).map((b) => decodeFrame(b)).filter((f) => f && f.streamId === STREAM_COMMANDS && f.kind === 0)
  .map((f) => v1.CommandRequest.decode(f?.payload as Uint8Array).command ?? 'other');
const buttons = (within: ParentNode | null) => [...(within?.querySelectorAll('button') ?? [])].map((b) => b.textContent?.trim() ?? '');
// Allow, Deny and "Allow always" in both its words (ADR-0101).
const ANSWERS = [t('approval-allow'), t('approval-deny'), t('approval-allow-always'), t('approval-allow-always-kept')];
const answers = (within: ParentNode | null) => buttons(within).filter((b) => ANSWERS.includes(b));
const click = async (el: Element | null) => {
  if (!el) throw new Error('nothing to click');
  await act(async () => { (el as HTMLElement).click(); });
  await tick(2);
};

describe('a viewer\'s window shows each request with no answers (ADR-0106, #1140)', () => {
  it('Home\'s list of what needs you', async () => {
    await windowAs(MEMBER);
    await until(() => expect(answers(document.querySelector('.fleet-ask'))).toEqual([t('approval-deny'), t('approval-allow')]));
    again();
    await windowAs(VIEWER);
    await until(() => expect(document.querySelector('.fleet-ask')?.textContent).toContain('src/auth.rs'));
    expect(answers(document.body)).toEqual([]);
  });

  it('the attention dialog', async () => {
    focused = true;
    await windowAs(MEMBER);
    await until(() => expect(answers(document.querySelector('.attention-card'))).toEqual([t('approval-deny'), t('approval-allow')]));
    again();
    await windowAs(VIEWER);
    await until(() => expect(document.querySelector('.attention-card')?.textContent).toContain('src/auth.rs'));
    const card = document.querySelector('.attention-card');
    expect(answers(card)).toEqual([]);
    expect(card?.querySelector('[data-watching]')?.textContent).toBe(t('approval-watching'));
  });

  it('the agent\'s pane in the window and the side panel', async () => {
    const pane = () => document.querySelector('[data-pane][data-agent="n1/a1"]');
    const panel = () => document.querySelector('.inspector__panel');
    await windowAs(MEMBER);
    await click(document.querySelector('[data-agent-row="n1/a1"]'));
    await until(() => expect(answers(pane())).toEqual([t('approval-deny'), t('approval-allow')]));
    expect(answers(panel())).toEqual([t('approval-deny'), t('approval-allow')]);
    again();
    await windowAs(VIEWER);
    await click(document.querySelector('[data-agent-row="n1/a1"]'));
    await until(() => expect(pane()?.textContent).toContain('src/auth.rs'));
    expect(panel()?.textContent).toContain('src/auth.rs');
    expect(answers(document.body)).toEqual([]);
  });

  it('the agent on its own screen', async () => {
    await windowAs(MEMBER, '#/agent/n1%2Fa1');
    await until(() => expect(answers(document.body)).toEqual([t('approval-deny'), t('approval-allow')]));
    again();
    await windowAs(VIEWER, '#/agent/n1%2Fa1');
    await until(() => expect(document.body.textContent).toContain('src/auth.rs'));
    expect(answers(document.body)).toEqual([]);
  });

  it('the phone\'s list and its bar', async () => {
    phone = true;
    await windowAs(MEMBER);
    await until(() => expect(answers(document.querySelector('.phone-dock'))).toEqual([t('approval-deny'), t('approval-allow')]));
    again();
    await windowAs(VIEWER);
    await until(() => expect(document.querySelector('.phone-dock')?.textContent).toContain('src/auth.rs'));
    const dock = document.querySelector('.phone-dock');
    expect(answers(document.body)).toEqual([]);
    expect(dock?.querySelector('[data-watching]')?.textContent).toBe(t('approval-watching'));
  });

  it('the agent on the phone', async () => {
    phone = true;
    await windowAs(MEMBER, '#/agent/n1%2Fa1');
    await until(() => expect(answers(document.querySelector('.phone-dock'))).toEqual([t('approval-deny'), t('approval-allow')]));
    again();
    await windowAs(VIEWER, '#/agent/n1%2Fa1');
    // While the bar holds it, the card folds to one line (visual pass 2026-11, finding 16): opened, it is still read-only.
    await until(() => expect(document.querySelector('.phone-fold__btn')).not.toBeNull());
    expect(answers(document.body)).toEqual([]);
    await click(document.querySelector('.phone-fold__btn'));
    await until(() => expect(document.querySelector('.phone-req')?.textContent).toContain('src/auth.rs'));
    expect(answers(document.body)).toEqual([]);
  });

  it('an older hub (no role named) keeps the answers', async () => {
    await windowAs(UNSPECIFIED);
    await until(() => expect(answers(document.querySelector('.fleet-ask'))).toEqual([t('approval-deny'), t('approval-allow')]));
  });
});

describe('a viewer\'s window has nothing to type to a chat agent, and its settings are read-only (ADR-0106, #1140)', () => {
  const box = () => document.querySelector('textarea[aria-label]');
  const modeOff = async () => {
    await click(document.querySelector(`[aria-label^="${t('session-mode')}: "]`));
    const items = [...document.querySelectorAll('[role^=menuitem]')].filter((i) => i.textContent?.includes('Plan'));
    expect(items).toHaveLength(1);
    return (items[0] as HTMLButtonElement).disabled || items[0]?.getAttribute('aria-disabled') === 'true';
  };
  for (const [where, hash, open, onPhone] of [
    ['the chat in the window', '#/', true, false],
    ['the chat on its own screen', '#/agent/n1%2Fc1', false, false],
    ['the chat on the phone', '#/agent/n1%2Fc1', false, true],
  ] as const) {
    it(where, async () => {
      phone = onPhone;
      await windowAs(MEMBER, hash);
      if (open) await click(document.querySelector('[data-agent-row="n1/c1"]'));
      await until(() => expect(box()).not.toBeNull());
      expect(buttons(document.body)).toContain(t('prompt-send'));
      expect(document.body.textContent).toMatch(new RegExp(`${t('prompt-intro')}|${t('chat-empty')}`.replace(/[.?]/g, '\\$&')));
      expect(await modeOff()).toBe(false);
      again();
      await windowAs(VIEWER, hash);
      if (open) await click(document.querySelector('[data-agent-row="n1/c1"]'));
      expect(box()).toBeNull();
      expect(buttons(document.body)).not.toContain(t('prompt-send'));
      expect(document.querySelector('[data-watching]')?.textContent).toBe(t('prompt-watching'));
      // Nothing above it tells a viewer to give the agent something to do.
      expect(document.body.textContent).not.toContain(t('prompt-intro'));
      expect(document.body.textContent).not.toContain(t('chat-empty'));
      expect(await modeOff()).toBe(true);
    });
  }
});

describe('a viewer\'s window offers no other change the hub would refuse (ADR-0106, #1141 review)', () => {
  it('a helper\'s grants are not listed to be revoked', async () => {
    await windowAs(MEMBER);
    await click(document.querySelector('[data-agent-row="n1/a1"]'));
    await until(() => expect(asked()).toContain('grantList'));
    again();
    await windowAs(VIEWER);
    await click(document.querySelector('[data-agent-row="n1/a1"]'));
    expect(document.querySelector('[data-agent-row="n1/a1"][aria-pressed="true"]')).not.toBeNull();
    expect(asked()).not.toContain('grantList');
  });

  it('another device is not offered for removal', async () => {
    await windowAs(MEMBER, '#/settings/devices');
    await until(() => expect(buttons(document.body)).toContain(t('device-remove')));
    again();
    await windowAs(VIEWER, '#/settings/devices');
    await until(() => expect(document.body.textContent).toContain('Phone'));
    expect(buttons(document.body)).not.toContain(t('device-remove'));
  });

  it('forgetting a machine is the owner\'s alone (and an older hub\'s window keeps it)', async () => {
    const forget = () => buttons(document.body).filter((b) => b.includes(t('forget-machine-action')));
    for (const [role, offered] of [[OWNER, true], [UNSPECIFIED, true], [MEMBER, false], [VIEWER, false]] as const) {
      await windowAs(role, '#/machines');
      await until(() => expect(document.body.textContent).toContain('Gone'));
      expect(forget().length > 0).toBe(offered);
      again();
    }
  });
});
