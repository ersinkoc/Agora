// @vitest-environment jsdom
// Redesign PR 2, "Home without stripes" (ADR-0102): tiles and request cards carry colour as a surface (tint, hairline, avatar, pill), never
// as a bar on one side; the counts are stat tiles; the request cards keep every safety rule of the rows they replace (Allow only for a
// request shown whole, Review for anything else, so the confirm step for acting freely stays in the full card) and add the dwell (T44).
// The rendered side-bar check (computed style, every element of a tile and a card) is scripts/e2e-home-no-stripes.mjs, in a real browser.
import { readFileSync } from 'node:fs';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { demoSkipApproval, demoStartApproval, demoStartNoAskApproval } from '../fixtures';
import { setLanguage, t } from '../i18n';
import type { AgentTileModel } from '../model';
import { ATTENTION_DWELL_MS } from '../net/attention';
import { attentionList, type FleetCounts } from '../net/fleet';
import type { ApprovalModel } from '../net/view';
import { AgentTile } from './AgentTile';
import { AttentionQueue, FleetSummary } from './FleetConsole';
import { GroupByControl } from './GroupByControl';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const NOW = 1_000_000_000;
const agent = (id: string, over: Partial<AgentTileModel> = {}): AgentTileModel =>
  ({ id, name: `claude · ${id}`, accent: 'var(--agent-teal)', kind: 'claude', machine: 'laptop', machineId: 'n1', workspace: 'login-fix', project: 'login-fix', state: 'working', preview: '', ...over });
const ask = (id: string, agentId: string, over: Partial<ApprovalModel> = {}): ApprovalModel =>
  ({ id, node: 'n1', localId: id, agentId, agentName: '', machineName: 'laptop', action: 'Edit src/auth.rs', detail: '+12 −3', expiresAtMs: 0, ...over });
const ctx = { group: 'machine' as const, now: NOW };
const html = (el: React.ReactElement) => renderToStaticMarkup(el);

let host: HTMLElement;
let root: Root;
beforeEach(() => {
  setLanguage('en');
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

describe('a tile without a stripe', () => {
  it('shows who the agent is as an avatar beside its name and its state as a pill, with no colour on its edge', () => {
    const out = html(<AgentTile agent={agent('a', { preview: 'Running cargo test' })} context={ctx} />);
    expect(out).toMatch(/<span class="tile__head"><span class="kit-avatar" style="--avatar:var\(--agent-teal\)" aria-hidden="true">C<\/span><span class="tile__name">claude · a<\/span><span class="kit-pill kit-tone-working">/);
    expect(out).not.toContain('--tile-accent');
    expect(out).toContain('<span class="tile__activity" dir="auto">Running cargo test</span>');
    // The gallery's plain tile too.
    expect(html(<AgentTile agent={agent('a')} />)).not.toContain('--tile-accent');
    expect(html(<AgentTile agent={agent('a')} />)).toContain('kit-avatar');
  });
  it('puts where it is, its changed files and the time in the foot as grey tags', () => {
    const out = html(<AgentTile agent={agent('a', { branch: 'fix/token', filesChanged: 6, since: NOW - 60_000 })} context={ctx} />);
    const foot = out.slice(out.indexOf('tile__foot'));
    expect(foot).toMatch(/<span class="kit-tag">.*?<span class="tile__where">login-fix<\/span><\/span>/);
    expect(foot).toMatch(/<span class="kit-tag kit-tag--mono">.*?<span class="tile__where">fix\/token<\/span><\/span>/);
    expect(foot).toContain('<span class="kit-tag">6 files changed</span>');
    expect(foot).toMatch(/class="tile__when" title="Working for 1 min">/);
  });
  it('says needs you with the amber tint (tile--needs-you) and the amber pill, and its ask in the well', () => {
    const out = html(<AgentTile agent={agent('a', { state: 'needs-you' })} context={{ ...ctx, ask: 'Edit src/auth.rs' }} />);
    expect(out).toMatch(/^<button type="button" class="tile tile--compact tile--needs-you"/);
    expect(out).toContain('<span class="kit-pill kit-tone-needs">');
    expect(out).toContain('>Needs you</span>');
    expect(out).toContain('<span class="tile__activity" dir="auto">Asks: Edit src/auth.rs</span>');
  });
  it('says failed with the red pill (ADR-0102), never the needs-you or finished look', () => {
    const out = html(<AgentTile agent={agent('a', { state: 'finished', closed: true, exitCode: 3, endedMs: NOW - 60_000 })} context={ctx} />);
    expect(out).toMatch(/class="tile tile--compact tile--failed tile--closed"/);
    expect(out).toContain('<span class="kit-pill kit-tone-danger">');
    expect(out).toContain('>Failed</span>');
    expect(out).toContain('aria-label="Open claude · a, Failed"');
    expect(out).not.toMatch(/kit-tone-(needs|finished)/);
  });
  it('shows acting without asking as the bolt alone, in the foot, and says it in the tile\'s name', () => {
    const out = html(<AgentTile agent={agent('a', { skipsPermissions: true })} context={ctx} />);
    expect(out).toMatch(/<span class="tile__foot"><span role="img" aria-label="Runs without asking[^"]*" data-no-ask="started"/);
    expect(out).toContain(`aria-label="Open claude · a, Working, ${t('agent-does-not-ask')}"`);
  });

  it('makes node-supplied tile identity, location, and activity text visible', () => {
    const out = html(<AgentTile agent={agent('a', {
      name: 'clau\u3164de\u202e', machine: 'lap\u034ftop', project: 'lo\u2066gin', branch: 'fi\ue000x', preview: 'run\u0007ning',
    })} context={{ group: 'kind', now: NOW }} />);
    expect(out).toContain('clau\\u{3164}de\\u{202e}');
    expect(out).toContain('lap\\u{34f}top');
    expect(out).toContain('lo\\u{2066}gin');
    expect(out).toContain('fi\\u{e000}x');
    expect(out).toContain('run\\u{7}ning');
  });

  it('verifies tile labels across format, control, and supplementary private-use characters', () => {
    for (const [char, escaped] of [['\u200d', '\\u{200d}'], ['\u0085', '\\u{85}'], ['\u{f0000}', '\\u{f0000}']] as const) {
      const out = html(<AgentTile agent={agent('edge', { name: `co${char}dex`, machine: `ma${char}chine`, project: `pro${char}ject`, branch: `br${char}anch`, preview: `wor${char}king` })} context={{ group: 'kind', now: NOW }} />);
      for (const [start, end] of [['co', 'dex'], ['ma', 'chine'], ['pro', 'ject'], ['br', 'anch'], ['wor', 'king']] as const) expect(out).toContain(`${start}${escaped}${end}`);
    }
  });
});

describe('the CSS draws no stripe on Home', () => {
  const css = readFileSync('src/design/app.css', 'utf8');
  // Every rule that styles a tile or a Home request card or count.
  const rules = css.split('\n').filter((l) => /\.(tile|fleet-ask|fleet-attention|fleet-summary|fleet-stat|fleet-queue)\b/.test(l));
  it('has no side border, no inset side bar, no glow and no moving foot line', () => {
    expect(rules.length).toBeGreaterThan(10);
    for (const r of rules) {
      expect(r, r).not.toMatch(/border-(inline-start|inline-end|left|right|s|e)(-width|-color)?\s*:|border-[se]-\d|border-inline-start-/);
      expect(r, r).not.toMatch(/inset\s+-?\d+(px|rem)\s+0\s+0/);
      expect(r, r).not.toMatch(/--tile-accent|shadow-card|nudge|sweep|::after/);
    }
    expect(css).not.toMatch(/@keyframes (nudge|sweep)/);
  });
});

describe('the counts are stat tiles', () => {
  const counts: FleetCounts = { needs: 2, working: 3, finished: 0, failed: 1, agents: 6, machinesOnline: 2, machinesTotal: 3 };
  it('shows each count as a big number with its name, only Needs you tinted, and a zero not pressable', () => {
    act(() => root.render(<FleetSummary counts={counts} onFilter={() => {}} />));
    const tiles = [...host.querySelectorAll('.kit-stat')];
    expect(tiles.map((x) => x.querySelector('.kit-stat__n')?.textContent)).toEqual(['2', '3', '0', '1', '2']);
    expect(tiles.map((x) => x.querySelector('.kit-stat__label')?.textContent)).toEqual(['Needs you', 'Working', 'Finished', 'Failed', 'of 3 machines online']);
    expect(host.querySelectorAll('.kit-stat--tinted')).toHaveLength(1);
    expect(tiles[0]?.className).toContain('kit-tone-needs');
    expect(tiles[2]?.tagName).toBe('DIV'); // zero: not a button
    expect(tiles[3]?.querySelector('.kit-mark')?.className).toContain('kit-tone-danger'); // failed is red (ADR-0102)
    const machines = tiles[4] as HTMLAnchorElement;
    expect(machines.getAttribute('href')).toBe('#/machines');
    expect(machines.getAttribute('aria-label')).toBe('2 of 3 machines online');
  });
  it('filters on a press, clears on a second, and a zero needs-you count is not tinted', () => {
    const seen: (string | undefined)[] = [];
    act(() => root.render(<FleetSummary counts={counts} filter="working" onFilter={(f) => seen.push(f)} />));
    const working = host.querySelectorAll<HTMLButtonElement>('button.kit-stat')[1] as HTMLButtonElement;
    expect(working.getAttribute('aria-pressed')).toBe('true');
    act(() => working.click());
    act(() => (host.querySelector('button.kit-stat') as HTMLButtonElement).click());
    expect(seen).toEqual([undefined, 'needs-you']);
    act(() => root.render(<FleetSummary counts={{ ...counts, needs: 0 }} onFilter={() => {}} />));
    expect(host.querySelector('.kit-stat--tinted')).toBeNull();
  });
});

describe('request cards keep the safety rules', () => {
  const fleet = [agent('a', { state: 'needs-you' }), agent('b', { state: 'needs-you' })];
  const queue = (approvals: ApprovalModel[], over: Partial<Parameters<typeof AttentionQueue>[0]> = {}) => (
    <AttentionQueue items={attentionList(approvals, fleet)} machines={[]} now={NOW} onOpen={over.onOpen ?? (() => {})} onResolve={over.onResolve} {...over} />);
  const button = (name: string) => [...host.querySelectorAll('button')].find((b) => b.textContent === name);

  it('is a tinted card with the agent\'s avatar, never a bar: the Callout in the needs tone', () => {
    act(() => root.render(queue([ask('r1', 'a')], { onResolve: () => {} })));
    const card = host.querySelector('li.fleet-ask > section.kit-callout.kit-tone-needs');
    expect(card).not.toBeNull();
    expect(card?.querySelector('.kit-callout__title .kit-avatar')?.getAttribute('style')).toContain('--agent-teal');
    expect(card?.querySelector('.fleet-ask__cmd')?.textContent).toBe('Edit src/auth.rs · +12 −3');
  });
  it('waits out the dwell before Allow or Deny does anything, and a press during it sends nothing', () => {
    vi.useFakeTimers();
    const sent: [string, boolean][] = [];
    act(() => root.render(queue([ask('r1', 'a')], { onResolve: (x, ok) => { sent.push([x.id, ok]); } })));
    expect(button('Allow')?.getAttribute('aria-disabled')).toBe('true');
    act(() => button('Allow')?.click());
    act(() => button('Deny')?.click());
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    expect(button('Allow')?.hasAttribute('aria-disabled')).toBe(false);
    act(() => button('Allow')?.click());
    expect(sent).toEqual([['r1', true]]);
  });
  it('starts the dwell again when a card comes in above (a ready Allow must not move under a click), and when the person comes back', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); };
    act(() => root.render(queue([ask('r1', 'a')], { onResolve })));
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => root.render(queue([ask('r2', 'b', { expiresAtMs: NOW + 10_000 }), ask('r1', 'a')], { onResolve })));
    for (const b of host.querySelectorAll('button')) if (b.textContent === 'Allow') act(() => b.click());
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => { window.dispatchEvent(new Event('focus')); });
    act(() => button('Allow')?.click());
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow')?.click());
    expect(sent).toHaveLength(1);
  });
  it('waits while the link is down, says why, and starts the dwell again when it is back', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    act(() => root.render(queue([ask('r1', 'a')], { onResolve: (x) => { sent.push(x.id); }, stale: true })));
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS * 2); });
    expect(button('Allow')?.disabled).toBe(true);
    expect(host.textContent).toContain(t('fleet-stale-no-answer'));
    act(() => root.render(queue([ask('r1', 'a')], { onResolve: (x) => { sent.push(x.id); }, stale: false })));
    act(() => button('Allow')?.click());
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow')?.click());
    expect(sent).toEqual(['r1']);
  });
  it('reads first: a start request, a no-ask start, a switch to acting freely and a long request have Review only (#1070: the first three open Review over Home, the long one opens the agent)', () => {
    const start = { ...demoStartApproval, agentId: 'a' } as unknown as ApprovalModel;
    const noAsk = { ...demoStartNoAskApproval, agentId: 'a' } as unknown as ApprovalModel;
    const skip = { ...demoSkipApproval, agentId: 'a' } as unknown as ApprovalModel;
    const long = ask('r9', 'a', { detail: 'x'.repeat(200) });
    for (const x of [start, noAsk, skip, long]) {
      const opened: string[] = [];
      act(() => root.render(queue([x], { onResolve: () => {}, onOpen: (id) => opened.push(id) })));
      expect(button('Allow'), x.id).toBeUndefined();
      expect(button('Deny'), x.id).toBeUndefined();
      expect(host.querySelector('.fleet-ask__what--clamp'), x.id).not.toBeNull(); // cut text is never answered here
      act(() => button(t('fleet-review'))?.click());
      const review = !!(x.start || x.skip);
      expect(opened, x.id).toEqual(review ? [] : ['a']);
      expect(!!document.querySelector('[data-review-dialog]'), x.id).toBe(review);
      act(() => root.render(<></>));
    }
  });
});

describe('request cards: the dwell after Codex review of #1028 (T44)', () => {
  const button = (name: string) => [...host.querySelectorAll('button')].find((b) => b.textContent === name) as HTMLButtonElement;
  const render = (fleet: AgentTileModel[], approvals: ApprovalModel[], onResolve: (a: ApprovalModel, ok: boolean) => void) =>
    act(() => root.render(<AttentionQueue items={attentionList(approvals, fleet)} machines={[]} now={NOW} onOpen={() => {}} onResolve={onResolve} />));

  it('starts again when the machine or project the card shows changes, even when the request itself does not', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); };
    const x = ask('r1', 'a');
    render([agent('a', { state: 'needs-you', machine: 'laptop', project: 'web' })], [x], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    render([agent('a', { state: 'needs-you', machine: 'server', project: 'web' })], [x], onResolve);
    expect(host.textContent).toContain('server · web');
    act(() => button('Allow').click());
    render([agent('a', { state: 'needs-you', machine: 'server', project: 'api' })], [x], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS - 1); });
    act(() => button('Allow').click());
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(1); });
    act(() => button('Allow').click());
    expect(sent).toEqual(['r1']);
  });
  it('answers a request once: a second press, on either button, sends nothing while the card stays', () => {
    vi.useFakeTimers();
    const sent: [string, boolean][] = [];
    render([agent('a', { state: 'needs-you' })], [ask('r1', 'a')], (x, ok) => { sent.push([x.id, ok]); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow').click());
    act(() => button('Allow').click());
    act(() => button('Deny').click());
    expect(sent).toEqual([['r1', true]]);
    expect(button('Allow').getAttribute('aria-disabled')).toBe('true');
    expect(button('Deny').getAttribute('aria-disabled')).toBe('true');
    // Two presses in one batch (before any re-render) are still one answer.
    render([agent('b', { state: 'needs-you' })], [ask('r2', 'b')], (x, ok) => { sent.push([x.id, ok]); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => { button('Deny').click(); button('Allow').click(); });
    expect(sent).toEqual([['r1', true], ['r2', false]]);
  });
  it('is not ready the moment the person comes back: focus or the tab shown again and a press in the same batch send nothing', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    render([agent('a', { state: 'needs-you' })], [ask('r1', 'a')], (x) => { sent.push(x.id); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => { window.dispatchEvent(new Event('focus')); button('Allow').click(); });
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => { document.dispatchEvent(new Event('visibilitychange')); button('Allow').click(); });
    expect(sent).toEqual([]);
    // A timer started before the return does not make the card ready early.
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS - 100); });
    act(() => { window.dispatchEvent(new Event('focus')); });
    act(() => { vi.advanceTimersByTime(100); });
    act(() => button('Allow').click());
    expect(sent).toEqual([]);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow').click());
    expect(sent).toEqual(['r1']);
  });
});

describe('request cards: re-arming and the clock (merger review of #1028)', () => {
  const button = (name: string) => [...host.querySelectorAll('button')].find((b) => b.textContent === name) as HTMLButtonElement;
  type Resolve = (a: ApprovalModel, ok: boolean) => unknown;
  const render = (fleet: AgentTileModel[], approvals: ApprovalModel[], onResolve: Resolve, now = NOW) =>
    act(() => root.render(<AttentionQueue items={attentionList(approvals, fleet)} machines={[]} now={now} onOpen={() => {}} onResolve={onResolve as never} />));
  const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

  it('re-arms with a fresh dwell when the answer fails (false, or a rejected promise), never before the dwell', async () => {
    vi.useFakeTimers();
    for (const fail of [() => Promise.resolve(false), () => Promise.reject(new Error('refused')), () => false]) {
      const sent: string[] = [];
      render([agent('a', { state: 'needs-you' })], [ask('r1', 'a')], (x) => { sent.push(x.id); return fail(); });
      act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
      act(() => button('Allow').click());
      await flush();
      act(() => button('Allow').click()); // the fresh dwell has not passed
      expect(sent).toEqual(['r1']);
      act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
      expect(button('Allow').hasAttribute('aria-disabled')).toBe(false);
      act(() => button('Allow').click());
      expect(sent).toEqual(['r1', 'r1']);
      act(() => root.render(<></>));
    }
  });
  it('keeps an answer that went the only one for as long as its card is there: "Answer sent", Allow and Deny off, a press sends nothing', async () => {
    vi.useFakeTimers();
    // An explicit true, a promise of true, and a callback that returns nothing all mean the answer went.
    for (const went of [() => true, () => Promise.resolve(true), () => undefined]) {
      const sent: string[] = [];
      render([agent('a', { state: 'needs-you' })], [ask('r1', 'a')], (x) => { sent.push(x.id); return went(); });
      act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
      act(() => button('Allow').click());
      await flush();
      act(() => { vi.advanceTimersByTime(60_000); }); // far past any retry interval, the card still there
      act(() => { window.dispatchEvent(new Event('focus')); });
      act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
      expect(button('Allow').getAttribute('aria-disabled')).toBe('true');
      expect(button('Deny').getAttribute('aria-disabled')).toBe('true');
      act(() => button('Allow').click());
      act(() => button('Deny').click());
      expect(sent).toEqual(['r1']);
      expect(host.textContent).toContain(t('attention-sent'));
      expect(button(t('attention-review'))).toBeDefined();
      act(() => root.render(<></>));
    }
    // An answer still on its way (its promise has not settled) is never joined by a second one, however long it takes; once it went, the
    // card stays answered.
    const sent: string[] = [];
    let settle: (v: boolean) => void = () => {};
    render([agent('b', { state: 'needs-you' })], [ask('r2', 'b')], (x) => { sent.push(x.id); return new Promise<boolean>((r) => { settle = r; }); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow').click());
    act(() => { vi.advanceTimersByTime(30_000); });
    act(() => button('Deny').click());
    expect(host.textContent).not.toContain(t('attention-sent'));
    settle(true);
    await flush();
    act(() => { vi.advanceTimersByTime(30_000); });
    act(() => button('Deny').click());
    expect(sent).toEqual(['r2']);
    expect(host.textContent).toContain(t('attention-sent'));
    // When the request leaves the list it is forgotten; another request is answerable as usual.
    render([agent('b', { state: 'needs-you' })], [ask('r3', 'b')], (x) => { sent.push(x.id); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => button('Allow').click());
    expect(sent).toEqual(['r2', 'r3']);
  });
  it('does not start again when only the "Asked …" time changes, and does when the machine, project or ask does', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); };
    const waiting = (over: Partial<AgentTileModel> = {}) => [agent('a', { state: 'needs-you', since: NOW - 15_000, ...over })];
    const x = ask('r1', 'a');
    render(waiting(), [x], onResolve);
    expect(host.querySelector('.fleet-ask__time')?.textContent).toBe('Asked 15 seconds ago');
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    render(waiting(), [x], onResolve, NOW + 30_000);
    expect(host.querySelector('.fleet-ask__time')?.textContent).toBe('Asked 45 seconds ago');
    act(() => button('Allow').click());
    expect(sent).toEqual(['r1']);
    // Each of these does restart it: a press right after the change sends nothing, a press after a fresh dwell does.
    const changes: [string, AgentTileModel[], ApprovalModel][] = [['machine', waiting({ machine: 'server' }), x], ['project', waiting({ project: 'api' }), x], ['ask', waiting(), { ...x, detail: '+13 −3' }]];
    for (const [what, fleet, changed] of changes) {
      act(() => root.render(<></>));
      sent.length = 0;
      render(waiting(), [x], onResolve);
      act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
      render(fleet, [changed], onResolve);
      act(() => button('Allow').click());
      expect(sent, what).toEqual([]);
      act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
      act(() => button('Allow').click());
      expect(sent, what).toEqual(['r1']);
    }
  });
});

describe('request cards: an answer in flight, by request (Codex re-review of #1028)', () => {
  type Resolve = (a: ApprovalModel, ok: boolean) => unknown;
  const render = (fleet: AgentTileModel[], approvals: ApprovalModel[], onResolve: Resolve) =>
    act(() => root.render(<AttentionQueue items={attentionList(approvals, fleet)} machines={[]} now={NOW} onOpen={() => {}} onResolve={onResolve as never} />));
  const allowFor = (name: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="Allow: ${name}"]`) as HTMLButtonElement;
  const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

  it('never answers the same request again after the person comes back and the dwell passes', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    render([agent('a', { state: 'needs-you' })], [ask('r1', 'a')], (x) => { sent.push(x.id); return new Promise(() => {}); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => allowFor('claude · a').click());
    act(() => { window.dispatchEvent(new Event('focus')); });
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    expect(allowFor('claude · a').getAttribute('aria-disabled')).toBe('true');
    act(() => allowFor('claude · a').click());
    act(() => { vi.advanceTimersByTime(60_000); });
    act(() => allowFor('claude · a').click());
    expect(sent).toEqual(['r1']);
  });
  it('never answers it again when another request arrives while its answer is on the way, and the other one can be answered', async () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    let settle: (v: boolean) => void = () => {};
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); return x.id === 'r1' ? new Promise<boolean>((r) => { settle = r; }) : undefined; };
    const fleet = [agent('a', { state: 'needs-you' }), agent('b', { state: 'needs-you' })];
    render(fleet, [ask('r1', 'a')], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => allowFor('claude · a').click());
    render(fleet, [ask('r1', 'a'), ask('r2', 'b')], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    expect(allowFor('claude · a').getAttribute('aria-disabled')).toBe('true');
    act(() => allowFor('claude · a').click());
    expect(sent).toEqual(['r1']);
    // B is not held up by A's answer.
    expect(allowFor('claude · b').hasAttribute('aria-disabled')).toBe(false);
    act(() => allowFor('claude · b').click());
    expect(sent).toEqual(['r1', 'r2']);
    // A fails: it can be answered again, after a fresh dwell.
    settle(false);
    await flush();
    act(() => allowFor('claude · a').click());
    expect(sent).toEqual(['r1', 'r2']);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => allowFor('claude · a').click());
    expect(sent).toEqual(['r1', 'r2', 'r1']);
  });
});

describe('request cards: holds across a refreshed list, and an acknowledgement restarts the dwell (Codex on d170a6df)', () => {
  type Resolve = (a: ApprovalModel, ok: boolean) => unknown;
  const fleet = [agent('a', { state: 'needs-you' }), agent('b', { state: 'needs-you' })];
  const render = (approvals: ApprovalModel[], onResolve: Resolve) =>
    act(() => root.render(<AttentionQueue items={attentionList(approvals, fleet)} machines={[]} now={NOW} onOpen={() => {}} onResolve={onResolve as never} />));
  const allowFor = (name: string) => host.querySelector<HTMLButtonElement>(`button[aria-label="Allow: ${name}"]`) as HTMLButtonElement;
  const flush = async () => { await act(async () => { await Promise.resolve(); await Promise.resolve(); }); };

  it('keeps an answer on its way held when the list drops its request and brings it back', () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); return new Promise(() => {}); };
    const a = ask('r1', 'a');
    const b = ask('r2', 'b');
    render([a, b], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => allowFor('claude · a').click());
    render([b], onResolve);
    render([a, b], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    expect(allowFor('claude · a').getAttribute('aria-disabled')).toBe('true');
    act(() => allowFor('claude · a').click());
    expect(sent).toEqual(['r1']);
  });
  it('starts the dwell again when an acknowledgement adds "Answer sent" and Review to a card, so B is not pressed by accident', async () => {
    vi.useFakeTimers();
    const sent: string[] = [];
    let settle: (v: boolean) => void = () => {};
    const onResolve = (x: ApprovalModel) => { sent.push(x.id); return x.id === 'r1' ? new Promise<boolean>((r) => { settle = r; }) : undefined; };
    render([ask('r1', 'a'), ask('r2', 'b')], onResolve);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => allowFor('claude · a').click());
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    expect(allowFor('claude · b').hasAttribute('aria-disabled')).toBe(false); // B is armed while A's answer is on its way
    settle(true);
    await flush();
    expect(host.textContent).toContain(t('attention-sent'));
    act(() => allowFor('claude · b').click());
    expect(sent).toEqual(['r1']);
    act(() => { vi.advanceTimersByTime(ATTENTION_DWELL_MS); });
    act(() => allowFor('claude · b').click());
    expect(sent).toEqual(['r1', 'r2']);
  });
});

describe('Group by is the shared segmented control', () => {
  it('is a radio group named Group by, chosen with a click or the arrow keys', () => {
    const seen: string[] = [];
    act(() => root.render(<GroupByControl by="machine" onChange={(g) => seen.push(g)} />));
    const group = host.querySelector('[role="radiogroup"]') as HTMLElement;
    expect(group.getAttribute('aria-label')).toBe('Group by');
    expect(group.className).toBe('kit-seg');
    expect([...group.querySelectorAll('[role="radio"]')].map((r) => `${r.textContent}:${r.getAttribute('aria-checked')}`)).toEqual(['Machine:true', 'Project:false', 'Agent:false', 'State:false']);
    act(() => { group.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })); });
    act(() => (group.querySelectorAll('[role="radio"]')[3] as HTMLButtonElement).click());
    expect(seen).toEqual(['project', 'state']);
  });
});
