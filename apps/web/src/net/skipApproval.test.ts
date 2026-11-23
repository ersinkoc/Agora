// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { createElement } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { agora } from '../gen/agora.js';
import { setLanguage } from '../i18n';
import { SkipApproval } from '../components/SkipApproval';
import { approvalFailureKey, thisDevice, toHex, type KeyStore } from './approval';
import { failure } from './refusal';
import { approvedHere, forgetSkipCapable, isSkipCapable, markSkipCapable, recordSkipAnswer, skipAnswerOf, parseSkipDetail, signSkip, skipOf, skipSignedMessage } from './skipApproval';
import { applyEvent, approvalList, emptyView } from './view';
import { decidableInline } from './fleet';
import { modalKind } from './attention';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

interface Vector { name: string; node: string; approval_id: string; approved: boolean; agent_id: string; option_id: string; value: string; detail: string; hex: string }
const vectors = JSON.parse(readFileSync('../../schema/fixtures/signing/approval-skip-v1.json', 'utf8')) as { cases: Vector[] };

const detail = (over: Record<string, unknown> = {}) => JSON.stringify({ node: 'laptop', agent: 'agent-1', agent_kind: 'claude-acp', folder: '/home/dev/code/login-fix', option_type: 'mode', option: 'mode', option_name: '', value: 'bypassPermissions', value_name: 'Bypass Permissions', skips: true, scope: 'session', ...over });
const ev = (producer: string, body: Record<string, unknown>) => agora.v1.Event.create({ producerId: producer, ...body });

beforeEach(() => { setLanguage('en'); forgetSkipCapable(); });

describe('agora-approval-skip-v1: the bytes a decision is signed over (ADR-0090 Amendment A)', () => {
  it('reproduces every shared vector byte for byte', async () => {
    expect(vectors.cases.length).toBeGreaterThanOrEqual(4);
    for (const c of vectors.cases) {
      const m = await skipSignedMessage(c.node, c.approval_id, c.approved, c.agent_id, c.option_id, c.value, c.detail);
      expect(toHex(m), c.name).toBe(c.hex);
    }
  });
  it('the vector cases are the ones a card produces: detail read as the node reads it, then signed', async () => {
    for (const c of vectors.cases) {
      const r = parseSkipDetail(c.detail);
      expect(r, c.name).toBeDefined();
      if (!r) continue;
      expect([r.node, r.agent, r.option, r.value], c.name).toEqual([c.node, c.agent_id, c.option_id, c.value]);
      expect(toHex(await skipSignedMessage(r.node, c.approval_id, c.approved, r.agent, r.option, r.value, c.detail)), c.name).toBe(c.hex);
    }
  });
  it('a different verdict, id, option, value or detail changes the bytes', async () => {
    const c = vectors.cases[0] as Vector;
    const base = toHex(await skipSignedMessage(c.node, c.approval_id, c.approved, c.agent_id, c.option_id, c.value, c.detail));
    for (const m of [
      await skipSignedMessage(c.node, c.approval_id, !c.approved, c.agent_id, c.option_id, c.value, c.detail),
      await skipSignedMessage(c.node, 'ap-9', c.approved, c.agent_id, c.option_id, c.value, c.detail),
      await skipSignedMessage(c.node, c.approval_id, c.approved, 'agent-2', c.option_id, c.value, c.detail),
      await skipSignedMessage(c.node, c.approval_id, c.approved, c.agent_id, 'other', c.value, c.detail),
      await skipSignedMessage(c.node, c.approval_id, c.approved, c.agent_id, c.option_id, 'plan', c.detail),
      await skipSignedMessage(c.node, c.approval_id, c.approved, c.agent_id, c.option_id, c.value, `${c.detail} `),
    ]) expect(toHex(m)).not.toBe(base);
  });
  it('the signature verifies against the device key over those bytes', async () => {
    let saved: Awaited<ReturnType<KeyStore['load']>>;
    const device = await thisDevice({ load: async () => saved, save: async (d) => { saved = d; } });
    const c = vectors.cases[0] as Vector;
    const req = parseSkipDetail(c.detail);
    expect(req).toBeDefined();
    if (!req) return;
    const sig = await signSkip(device, c.node, c.approval_id, c.approved, req, c.detail);
    expect(sig).toHaveLength(64);
    const msg = await skipSignedMessage(c.node, c.approval_id, c.approved, c.agent_id, c.option_id, c.value, c.detail);
    expect(await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, device.keys.publicKey, sig as BufferSource, msg as BufferSource)).toBe(true);
  });
});

describe('reading the node\'s card strictly', () => {
  it('accepts the shape the window expects', () => {
    expect(parseSkipDetail(detail())?.value).toBe('bypassPermissions');
    expect(parseSkipDetail(detail({ option_type: 'config', option: 'auto_approve', option_name: 'Auto approve', value: 'true' }))?.optionType).toBe('config');
  });
  it('refuses anything else: not JSON, missing or non-string fields, another type or scope, a mode that names another option', () => {
    for (const bad of ['', 'nope', '[]', 'null', detail({ option_type: 'toggle' }), detail({ scope: 'project' }), detail({ scope: undefined }), detail({ skips: undefined }), detail({ skips: false }), detail({ skips: 'true' }), detail({ value: 1 }), detail({ folder: undefined }), detail({ agent: '' }),
      detail({ option: 'effort' }), detail({ option: '', option_type: 'config' }), detail({ value_name: null })]) {
      expect(parseSkipDetail(bad), bad).toBeUndefined();
    }
  });
  it('a card must name the machine and the agent the hub filed it under, else it is unknown and not answerable', () => {
    expect(skipOf('agent.skip_permissions', detail(), 'laptop', 'laptop/agent-1')).toHaveProperty('ok');
    expect(skipOf('agent.skip_permissions', detail(), 'laptop', 'agent-1')).toHaveProperty('ok');
    expect(skipOf('agent.skip_permissions', detail(), 'other', 'other/agent-1')).toEqual({ unknown: true });
    expect(skipOf('agent.skip_permissions', detail(), 'laptop', 'laptop/agent-2')).toEqual({ unknown: true });
    expect(skipOf('agent.skip_permissions', '{}', 'laptop', 'laptop/agent-1')).toEqual({ unknown: true });
    expect(skipOf('agent.start', detail(), 'laptop', 'laptop/agent-1')).toBeUndefined();
  });
});

describe('option and value are signed exactly as the node gave them', () => {
  it('nothing is trimmed, cleaned or re-encoded between the card and the signed bytes', async () => {
    const d = detail({ option_type: 'config', option: ' a:b ', value: 'é ü ', value_name: 'x' });
    const r = parseSkipDetail(d);
    expect([r?.option, r?.value]).toEqual([' a:b ', 'é ü ']);
    const m = await skipSignedMessage('laptop', 'ap-1', true, r?.agent ?? '', r?.option ?? '', r?.value ?? '', d);
    expect(new TextDecoder().decode(m)).toContain('5: a:b 6:é ü ');
  });
  it('the new refusals have plain words in both languages', async () => {
    const { refusalKey, REFUSAL_KEYS } = await import('./refusal');
    expect(refusalKey('agent.option_unknown')).toBe('refusal-unknown-option');
    expect(refusalKey('node.resource_limit.skip_approvals_agent')).toBe('refusal-skip-agent-limit');
    expect(refusalKey('node.resource_limit')).toBe('refusal-limit');
    expect(REFUSAL_KEYS).toContain('refusal-limit');
  });
});

describe('the view: cards are withdrawn with the node and with the agent', () => {
  const raise = (v: ReturnType<typeof emptyView>, id: string, d = detail()) => applyEvent(v, 1 + v.pos, ev('laptop', { approvalRequested: { approvalId: id, agentId: 'agent-1', action: 'agent.skip_permissions', detail: d, expiresAtMs: 1 } }));
  it('shows the request as the node rendered it, and a card of an unexpected shape as unknown', () => {
    const v = emptyView();
    raise(v, 'ap-1');
    raise(v, 'ap-2', detail({ scope: 'forever' }));
    const [a, b] = approvalList(v);
    expect(a?.skip?.value).toBe('bypassPermissions');
    expect(a?.detail).toBe(detail()); // verbatim: the decision is signed over exactly this
    expect(b?.skip).toBeUndefined();
    expect(b?.skipUnknown).toBe(true);
  });
  it('the node withdrawing it (ApprovalResolved) removes the card', () => {
    const v = emptyView();
    raise(v, 'ap-1');
    applyEvent(v, v.pos + 1, ev('laptop', { approvalResolved: { approvalId: 'ap-1', approved: false } }));
    expect(approvalList(v)).toHaveLength(0);
  });
  it('the agent ending removes its skip cards at once, and leaves other approvals alone', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('laptop', { agentStarted: { agentId: 'agent-1', nodeId: 'laptop', kind: 'acp' } }));
    raise(v, 'ap-1');
    applyEvent(v, v.pos + 1, ev('laptop', { approvalRequested: { approvalId: 'ap-other', agentId: 'agent-1', action: 'run_command', detail: 'ls', expiresAtMs: 1 } }));
    applyEvent(v, v.pos + 1, ev('laptop', { agentExited: { agentId: 'agent-1', exitCode: 0 } }));
    expect(approvalList(v).map((a) => a.localId)).toEqual(['ap-other']);
  });
  it('an interrupted agent (adapter restarted) loses its skip cards too', () => {
    const v = emptyView();
    raise(v, 'ap-1');
    applyEvent(v, v.pos + 1, ev('laptop', { agentInterrupted: { agentId: 'agent-1', kind: 'acp', resumable: true } }));
    expect(approvalList(v)).toHaveLength(0);
  });
});

describe('what the window heard from the machine', () => {
  it('only an agent.approval_needed answer, with its id, shows the machine offers it (and says which card)', () => {
    expect(isSkipCapable('laptop')).toBe(false);
    expect(failure('laptop', { code: 'agent.option_unconfirmed', params: {} })).toBe('agent.option_unconfirmed');
    expect(failure('laptop', { code: 'node.resource_limit', params: { resource: 'skip_approvals_agent', limit: '2' } })).toBe('node.resource_limit.skip_approvals_agent');
    expect(failure('laptop', { code: 'node.resource_limit', params: { resource: 'skip_approvals' } })).toBe('node.resource_limit');
    expect(isSkipCapable('laptop')).toBe(false);
    expect(isSkipCapable('laptop')).toBe(false);
    expect(failure('laptop', { code: 'agent.approval_needed', params: {} })).toBe(false); // no id: nothing to wait for
    expect(isSkipCapable('laptop')).toBe(false);
    expect(failure('laptop', { code: 'agent.approval_needed', params: { approval_id: 'ap-1' } })).toEqual({ approvalNeeded: 'laptop/ap-1' });
    expect(isSkipCapable('laptop')).toBe(true);
    expect(isSkipCapable('desk')).toBe(false);
  });
  it('a decision answered agent.not_found says the request is no longer valid; an expired one says it ran out of time', () => {
    expect(approvalFailureKey('agent.not_found')).toBe('approval-no-longer-valid');
    expect(approvalFailureKey('approval.expired')).toBe('approval-ran-out');
    expect(approvalFailureKey('approval.signature_invalid')).toBe('approval-not-trusted');
  });
});

// The whole card (Review's dialog, the phone): a pane and the side panel show it in short, with Deny and Review (`ReviewRequest`).
describe('the card', () => {
  let host: HTMLElement;
  let root: ReturnType<typeof createRoot>;
  beforeEach(() => { host = document.createElement('div'); document.body.append(host); root = createRoot(host); });
  afterEach(() => { act(() => root.unmount()); host.remove(); });
  const card = (d = detail(), name = 'claude · amber') => {
    const v = emptyView();
    applyEvent(v, 1, ev('laptop', { approvalRequested: { approvalId: 'ap-1', agentId: 'agent-1', action: 'agent.skip_permissions', detail: d, expiresAtMs: 1 } }));
    return { ...(approvalList(v)[0] as ReturnType<typeof approvalList>[number]), agentName: name, machineName: 'Laptop' };
  };
  const buttons = () => [...host.querySelectorAll('button')];
  const dwell = (ms = 650) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
  const named = (text: string) => buttons().find((b) => b.textContent === text) as HTMLButtonElement;
  it('shows the machine\'s own rendering with the real ids beside the agent\'s names, and Allow and Deny', async () => {
    vi.useFakeTimers();
    markSkipCapable('laptop');
    const answers: boolean[] = [];
    act(() => root.render(createElement(SkipApproval, { approval: card(), onResolve: (ok) => { answers.push(ok); } })));
    const text = host.textContent ?? '';
    expect(text).toContain('Let claude · amber act without asking?');
    expect(text).toContain('/home/dev/code/login-fix');
    expect(text).toContain('Bypass Permissions');
    expect(text).toContain('bypassPermissions'); // the real value id
    expect(text).toContain('session only');
    expect(buttons().map((b) => b.textContent?.replace('Read it first, then you can allow.', ''))).toEqual(['Deny', 'Allow']);
    expect(host.contains(document.activeElement) && document.activeElement?.tagName === 'BUTTON').toBe(false); // nothing is focused for you
    await dwell();
    act(() => named('Allow').click());
    expect(answers).toEqual([true]);
    vi.useRealTimers();
  });
  it('Allow stays off for the dwell (with a visible reason), Deny does not, and one click sends one decision', async () => {
    vi.useFakeTimers();
    markSkipCapable('laptop');
    const answers: boolean[] = [];
    act(() => root.render(createElement(SkipApproval, { approval: card(), onResolve: (ok) => { answers.push(ok); } })));
    expect(named('Allow').disabled).toBe(true);
    expect(named('Deny').disabled).toBe(false);
    expect(host.textContent).toContain('Read it first');
    act(() => named('Allow').click()); // a click inside the dwell does nothing
    expect(answers).toEqual([]);
    await dwell(300);
    expect(named('Allow').disabled).toBe(true);
    await dwell(400);
    expect(named('Allow').disabled).toBe(false);
    expect(host.textContent).not.toContain('Read it first');
    act(() => { named('Allow').click(); named('Allow').click(); });
    expect(answers).toEqual([true]);
    expect(named('Allow').disabled && named('Deny').disabled).toBe(true); // after a click both wait
    await dwell(4100);
    expect(named('Allow').disabled).toBe(false); // a failed answer can be tried again
    vi.useRealTimers();
  });
  it('the dwell starts again when the card says something else', async () => {
    vi.useFakeTimers();
    markSkipCapable('laptop');
    act(() => root.render(createElement(SkipApproval, { approval: card(), onResolve: () => {} })));
    await dwell();
    expect(named('Allow').disabled).toBe(false);
    act(() => root.render(createElement(SkipApproval, { approval: card(detail({ value: 'other' })), onResolve: () => {} })));
    expect(named('Allow').disabled).toBe(true);
    vi.useRealTimers();
  });
  it('a config option shows its own id too, and hidden characters in the agent\'s words are written out', () => {
    markSkipCapable('laptop');
    act(() => root.render(createElement(SkipApproval, { approval: card(detail({ option_type: 'config', option: 'auto_approve', option_name: 'Auto‮approve', value: 'true', value_name: 'On' })), onResolve: () => {} })));
    const text = host.textContent ?? '';
    expect(text).toContain('auto_approve');
    expect(text).toContain('\\u{202e}');
    expect(text).not.toContain('‮');
  });
  it('for a machine the window has not heard offer it, nothing can be answered (and the card says why)', () => {
    act(() => root.render(createElement(SkipApproval, { approval: card(), onResolve: () => { throw new Error('answered'); } })));
    expect(buttons().every((b) => b.disabled)).toBe(true);
    expect(host.textContent).toContain('has not heard this machine offer it');
  });
  it('an unknown shape says to update the window and can only be dismissed here: nothing is signed or sent', () => {
    markSkipCapable('laptop');
    const onResolve = vi.fn();
    act(() => root.render(createElement(SkipApproval, { approval: card(detail({ skips: false })), onResolve })));
    expect(buttons().map((b) => b.textContent)).toEqual(['Dismiss']);
    expect(host.textContent).toContain('Update your window');
    expect(host.textContent).toContain('Nothing was signed');
    act(() => buttons()[0]?.click());
    expect(onResolve).not.toHaveBeenCalled();
    expect(host.textContent).toBe('');
  });
});

describe('the attention dialog and the phone bar never answer it', () => {
  it('a skip card (known or not) is read in its own card, never decided inline', () => {
    const v = emptyView();
    applyEvent(v, 1, ev('laptop', { approvalRequested: { approvalId: 'ap-1', agentId: 'agent-1', action: 'agent.skip_permissions', detail: detail(), expiresAtMs: 1 } }));
    applyEvent(v, 2, ev('laptop', { approvalRequested: { approvalId: 'ap-2', agentId: 'agent-1', action: 'agent.skip_permissions', detail: '{}', expiresAtMs: 1 } }));
    for (const a of approvalList(v)) {
      expect(decidableInline(a)).toBe(false);
      expect(modalKind({ kind: 'approval', approval: a } as never)).toBe('review');
    }
  });
});

describe('what is remembered belongs to one connection', () => {
  it('forgetting one machine leaves the others; forgetting all also drops the answers given here', () => {
    markSkipCapable('a'); markSkipCapable('b');
    recordSkipAnswer('a/ap-1', { approved: true, agent: 'a/agent-1', option: 'mode', value: 'yolo' });
    forgetSkipCapable('a');
    expect([isSkipCapable('a'), isSkipCapable('b')]).toEqual([false, true]);
    expect(skipAnswerOf('a/ap-1')).toBe('approved');
    expect(approvedHere('a/agent-1', 'mode', 'yolo')).toBe(true);
    expect(approvedHere('a/agent-1', 'mode', 'other')).toBe(false);
    forgetSkipCapable();
    expect([isSkipCapable('b'), skipAnswerOf('a/ap-1'), approvedHere('a/agent-1', 'mode', 'yolo')]).toEqual([false, undefined, false]);
  });
  it('importing the gallery does not make a machine called "demo" capable', async () => {
    forgetSkipCapable();
    await import('../screens');
    expect(isSkipCapable('demo')).toBe(false);
  });
});
