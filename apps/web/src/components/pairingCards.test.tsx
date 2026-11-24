import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { setLanguage, t } from '../i18n';
import { attentionList, fleetCounts } from '../net/fleet';
import type { PairCard } from '../net/pairing';
import { AttentionQueue, FleetConsole, FleetEmpty } from './FleetConsole';
import { PairCardView, PairRequest } from './PairRequest';
import { Details } from './Inspector';
import { PairCards, PairRecentList, PairWindowControl, type PairingUi } from './Pairing';
import { initialPairCard, pairCardReducer, type PairCardState } from './pairChoice';

const NOW = 1_000_000_000;
const card = (over: Partial<PairCard> = {}): PairCard => ({ id: 'aa', pairId: new Uint8Array([1]), kind: 'machine', name: 'laptop', os: 'windows', address: '192.168.1.50', number: '482193', expiresMs: NOW + 272_000, ...over });
const ui = (over: Partial<PairingUi> = {}): PairingUi => ({ cards: [card()], recent: [], offline: false, onAnswer: async () => 'ok', onOpen: async () => true, onClose: async () => true, ...over });
const noop = () => undefined;
const view = (state: Partial<PairCardState> = {}, over: Partial<Parameters<typeof PairCardView>[0]> = {}) => renderToStaticMarkup(
  <PairCardView request={card()} state={{ ...initialPairCard, ...state }} now={NOW} onApprove={noop} onBack={noop} onSend={noop} onReplace={noop} {...over} />);

beforeEach(() => setLanguage('en'));

describe('the card an owner answers (ADR-0065)', () => {
  it('shows the name, the system and the address in rows of their own, and the number in two big groups of three', () => {
    const html = view();
    expect(html).toContain('<dt class="text-muted">Name</dt><dd class="m-0 min-w-0 break-words text-ink"><bdi>laptop</bdi></dd>');
    expect(html).toContain('<dt class="text-muted">System</dt><dd class="m-0 min-w-0 break-words text-ink"><bdi>Windows</bdi></dd>');
    expect(html).toContain('<dt class="text-muted">Address</dt>');
    expect(html).toContain('<bdi dir="ltr">192.168.1.50</bdi>');
    expect(html).toContain('class="pair-number"><span>482</span><span>193</span>');
    expect(html).toContain('Compare with the number on the new machine. Approve only if they are the same.');
  });

  it('isolates text that runs the other way, and escapes markup, in the name and the system', () => {
    const html = view({}, { request: card({ name: '‮evil<b>x</b>', os: '<i>' }) });
    expect(html).toContain('<bdi>');
    expect(html).not.toContain('<b>x</b>');
    expect(html).toContain('&lt;b&gt;x&lt;/b&gt;');
  });

  it('asks about Replace only when the hub said the name is already enrolled', () => {
    expect(view()).not.toContain('Replace the existing machine');
    expect(view()).not.toContain('type="checkbox"');
    const taken = view({}, { request: card({ nameTaken: true }) });
    expect(taken).toContain('Replace the existing machine');
    expect(taken).toContain('type="checkbox"');
    expect(taken).toContain('the old one loses its access');
    // It starts unticked.
    expect(taken).not.toContain('checked=""');
    expect(view({ replace: true }, { request: card({ nameTaken: true }) })).toContain('checked=""');
  });

  it('is disabled while the link is down, with the reason, and when the request has run out', () => {
    const off = view({}, { offline: true });
    expect(off).toContain('Not connected right now.');
    expect((off.match(/disabled=""/g) ?? []).length).toBe(2);
    const gone = view({}, { now: NOW + 300_000 });
    expect(gone).toContain('This request has run out.');
    expect((gone.match(/disabled=""/g) ?? []).length).toBe(2);
    // Live and in time: both buttons work.
    expect(view()).not.toContain('disabled=""');
  });

  it('counts down to the request\'s expiry', () => {
    expect(view()).toContain('Runs out in 4 min');
    expect(view({}, { now: NOW + 250_000 })).toContain('Runs out in 22 s');
    // A card with no expiry (the gallery) has no countdown.
    expect(view({}, { request: { ...card(), expiresMs: undefined } })).not.toContain('Runs out');
  });

  it('takes two presses to approve: Approve asks "do the numbers match?", only the second answers', () => {
    let s = initialPairCard;
    // The first press does not send anything; it shows the question.
    s = pairCardReducer(s, { type: 'approve' });
    expect(s.step).toBe('confirm');
    const asking = view(s);
    expect(asking).toContain('Do the two numbers match exactly?');
    expect(asking).toContain('Yes, the numbers match');
    expect(asking).toContain('Go back');
    // A press of Approve does not exist on that screen: the only way on is the explicit yes.
    expect(asking).not.toContain('>Numbers match…<');
    // "Go back" returns to the start with nothing sent.
    expect(pairCardReducer(s, { type: 'back' }).step).toBe('idle');
    // The yes does nothing until the question has been up for MIN_THINK_MS (`armed`).
    expect(pairCardReducer(s, { type: 'send', approve: true })).toBe(s);
    expect(pairCardReducer(pairCardReducer(s, { type: 'armed' }), { type: 'send', approve: true }).step).toBe('busy');
    // And no approval goes out from the card itself.
    expect(pairCardReducer(initialPairCard, { type: 'send', approve: true })).toBe(initialPairCard);
  });

  it('keeps Approve out of reach of a stray send and a double answer', () => {
    // A send needs a step the person went through; a second one while the first is out does nothing.
    const busy = pairCardReducer(initialPairCard, { type: 'send', approve: false });
    expect(pairCardReducer(busy, { type: 'send', approve: false })).toBe(busy);
    expect(pairCardReducer(busy, { type: 'approve' })).toBe(busy);
    const sent = pairCardReducer(busy, { type: 'answered', answer: 'ok' });
    expect(sent.step).toBe('sent');
    expect(pairCardReducer(sent, { type: 'send', approve: false })).toBe(sent);
    expect(view(sent)).toContain('Answer sent.');
  });

  it('after "name taken" the card starts over: the owner chooses about Replace and goes through both presses again', () => {
    let s = [{ type: 'approve' }, { type: 'armed' }, { type: 'send', approve: true }].reduce((x, a) => pairCardReducer(x, a as never), initialPairCard);
    s = pairCardReducer(s, { type: 'answered', answer: 'taken' });
    expect(s).toMatchObject({ step: 'idle', failed: false, replace: false });
    // Ticking Replace is itself a choice that sends nothing and restarts the two presses.
    s = pairCardReducer(pairCardReducer(s, { type: 'approve' }), { type: 'replace', on: true });
    expect(s).toMatchObject({ step: 'idle', replace: true });
  });

  it('says so when the answer could not be sent, and leaves the card ready to try again', () => {
    let s = pairCardReducer(initialPairCard, { type: 'send', approve: false });
    s = pairCardReducer(s, { type: 'answered', answer: 'failed' });
    expect(s).toMatchObject({ step: 'idle', failed: true });
    expect(view(s)).toContain('Your answer could not be sent. Try again.');
  });

  it('speaks Turkish', () => {
    setLanguage('tr');
    const html = view({ step: 'confirm' });
    expect(html).toContain('İki numara tamamen aynı mı?');
    expect(view()).toContain('Yeni makinedeki numarayla karşılaştırın.');
    expect(view({}, { offline: true })).toContain('Şu anda bağlı değil.');
    setLanguage('en');
  });

  it('uses no internal word', () => {
    const html = [view(), view({ step: 'confirm' }), view({}, { request: card({ nameTaken: true }), offline: true }), renderToStaticMarkup(<PairWindowControl ui={ui()} intro />), renderToStaticMarkup(<PairWindowControl ui={ui({ window: { open: true, closesAtMs: NOW + 540_000, pending: 0 } })} now={NOW} />)].join(' ');
    for (const word of [/\bnode\b/i, /\bhub\b/i, /\benrol/i, /\bpairing\b/i,]) expect(html).not.toMatch(word);
  });

  it('is a dialog that floats unless shown in place', () => {
    expect(renderToStaticMarkup(<PairRequest request={card()} onAnswer={async () => 'ok'} />)).toContain('fixed bottom-4');
    expect(renderToStaticMarkup(<PairRequest inline request={card()} onAnswer={async () => 'ok'} />)).not.toContain('fixed bottom-4');
    expect(renderToStaticMarkup(<PairCards ui={ui({ cards: [card(), card({ id: 'bb', name: 'server' })] })} />).match(/role="dialog"/g)).toHaveLength(2);
  });
});

describe('a request to join in the fleet console', () => {
  const console_ = (p: PairingUi, over: Partial<Parameters<typeof FleetConsole>[0]> = {}) => renderToStaticMarkup(
    <FleetConsole fleet={[]} machines={[]} approvals={[]} link="live" now={NOW} onOpen={noop} onStart={noop} onResolve={noop} pairing={p} {...over} />);

  it('is a "needs you" item, counted in the header, and is only ever reviewed: no Approve in its row', () => {
    const items = attentionList([], [], [card()]);
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ kind: 'pair', key: 'pair:aa' });
    expect(fleetCounts([], [], [], 1).needs).toBe(1);
    const html = renderToStaticMarkup(<AttentionQueue items={items} machines={[]} now={NOW} onOpen={noop} onResolve={noop} pairing={ui()} />);
    expect(html).toContain('<bdi class="fleet-ask__name">laptop</bdi>');
    expect(html).toContain('Review the request from \u2068laptop\u2069');
    expect(html).toContain('Compare its number with the one on its screen before you approve.');
    expect(html).not.toContain('Approve');
    expect(html).not.toContain('Numbers match');
    expect(html).not.toContain('Allow');
    // The row does not show the number: the owner sees it in the card, next to the machine's own screen.
    expect(html).not.toContain('482');
  });

  it('has its full card in the right-hand needs-you list, so that list does not say "all calm" while someone asks to join', () => {
    const details = (pairing?: PairingUi) => renderToStaticMarkup(<Details tab="needs" attention={[]} approvals={[]} changed={[]} boards={[]} messages={[]} projects={[]} onOpen={noop} onResolve={noop} pairing={pairing} />);
    const html = details(ui());
    expect(html).toContain('role="dialog"');
    expect(html).toContain('<span>482</span><span>193</span>');
    expect(html).not.toContain('All calm');
    expect(details(undefined)).toContain('All calm');
  });

  it('counts in "needs you" with the agents, and sorts by when it runs out', () => {
    const items = attentionList([], [], [card({ id: 'late', expiresMs: NOW + 200_000 }), card({ id: 'soon', expiresMs: NOW + 50_000 })]);
    expect(items.map((i) => i.key)).toEqual(['pair:soon', 'pair:late']);
    const html = console_(ui({ cards: [card()] }), { machines: [{ id: 'n1', name: 'laptop', os: 'linux', online: true, agentsRunning: 0, lastSeenMs: NOW }], fleet: [{ id: 'n1/a', name: 'claude · a', accent: '#fff', machine: 'laptop', machineId: 'n1', workspace: 'w', project: 'w', state: 'working', preview: '' }] });
    expect(html).toMatch(/kit-stat--tinted kit-tone-needs"[^>]*><span class="kit-stat__n">1<\/span>/);
    expect(html).toContain('Review and decide');
  });

  it('shows a request on a hub that has no machine yet, above the empty state, and offers to let a machine join', () => {
    const html = console_(ui());
    expect(html).toContain('fleet-attention');
    expect(html).toContain('Allow a new machine to join');
    expect(html.indexOf('fleet-attention')).toBeLessThan(html.indexOf('fleet-empty'));
  });

  it('offers "Allow a new machine to join" on a fresh hub, and shows the open window with a Close button', () => {
    const closed = renderToStaticMarkup(<FleetEmpty kind="empty" machines={[]} pairing={ui({ cards: [] })} />);
    expect(closed).toContain('Allow a new machine to join');
    expect(closed).toContain('agora join');
    const open = renderToStaticMarkup(<PairWindowControl ui={ui({ window: { open: true, closesAtMs: NOW + 540_000, pending: 0 } })} now={NOW} />);
    expect(open).toContain('Open for new machines. It closes in 9 min.');
    expect(open).toContain('>Close<');
    expect(open).not.toContain('Allow a new machine to join');
    // Without a link it cannot be changed, and says why.
    const off = renderToStaticMarkup(<PairWindowControl ui={ui({ offline: true })} />);
    expect(off).toContain('disabled=""');
    expect(off).toContain('Not connected right now.');
  });

  it('shows no pairing at all without an owner\'s session', () => {
    const html = renderToStaticMarkup(<FleetEmpty kind="empty" machines={[]} />);
    expect(html).not.toContain('Allow a new machine to join');
    expect(console_(undefined as unknown as PairingUi)).not.toContain('wants to join');
  });

  it('puts the end of a request in the recent list, one line each, in both languages', () => {
    const recent = (['approved', 'denied', 'expired', 'closed'] as const).map((outcome, i) => ({ id: `r${i}`, kind: 'machine' as const, name: 'laptop', outcome, at: 1 }));
    const html = renderToStaticMarkup(<PairRecentList recent={recent} />);
    expect(html).toContain('\u2068laptop\u2069 was approved and is joining.');
    expect(html).toContain('\u2068laptop\u2069 was denied.');
    expect(html).toContain('The request from \u2068laptop\u2069 ran out.');
    expect(html).toContain('The request from \u2068laptop\u2069 was cancelled.');
    setLanguage('tr');
    expect(renderToStaticMarkup(<PairRecentList recent={recent} />)).toContain('\u2068laptop\u2069 onaylandı, katılıyor.');
    setLanguage('en');
    expect(renderToStaticMarkup(<PairRecentList recent={[]} />)).toBe('');
    expect(t('pair-recent')).toBe('Recent requests to join');
  });

  it('writes nothing to storage or the console while rendering', () => {
    const store = { setItem: vi.fn(), getItem: vi.fn(() => null), removeItem: vi.fn() };
    vi.stubGlobal('localStorage', store);
    vi.stubGlobal('sessionStorage', store);
    const spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) => vi.spyOn(console, m));
    console_(ui());
    view({ step: 'confirm' });
    expect(store.setItem).not.toHaveBeenCalled();
    for (const spy of spies) expect(spy).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });
});
