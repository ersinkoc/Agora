// ADR-0113 step 4: the person's themes, packs and theme choice kept the same on their devices through the hub's opaque settings value.
import { describe, expect, it } from 'vitest';
import { CommandError } from '../net/live';
import { writeTheme } from './file';
import { createPackStore } from './kept';
import { writePack } from './pack';
import { CHOICE_AT_KEY, CHOICE_KEY, MAX_USER_THEMES, createThemeStore } from './store';
import { CLOCK_SLACK_MS, EMPTY, MAX_SYNC_BYTES, SETTING_KEY, type SyncAsk, createThemeSync, merge, readSync, serialiseSync } from './sync';
import { BUILT_IN, serialiseTheme } from './tokens';

function memory(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => { m.set(k, v); }, removeItem: (k: string) => { m.delete(k); }, m };
}
const night = (name = 'Night', accent = '#ff9f43') => serialiseTheme({ 'agora-theme': 1, name, scheme: 'dark', tokens: { accent, 'on-accent': '#1a1000' } });
const sand = BUILT_IN.find((b) => b.id === 'sand')!;
const uid = (n: number) => n.toString(16).padStart(24, '0');
const file = (name: string) => writeTheme(sand, name);
const decode = (b: Uint8Array) => new TextDecoder().decode(b);

// A hub that keeps one person's value with its version and the person's tag, as `user_settings.rs` does.
let people = 0;
function fakeHub(init?: Record<string, unknown>, tag = (++people).toString(16).padStart(32, '0')) {
  const h: { value: Uint8Array; version: number; calls: string[] } = { value: init ? new TextEncoder().encode(JSON.stringify(init)) : new Uint8Array(), version: init ? 1 : 0, calls: [] as string[] };
  const ask: SyncAsk = async (body) => {
    if ('userSettingsGet' in body) { h.calls.push('get'); return { userSettings: { key: 'themes', value: h.value, version: h.version, tag } }; }
    h.calls.push('set');
    const s = body.userSettingsSet;
    if (s.value.length > MAX_SYNC_BYTES) throw new CommandError('settings.too_large', { max: String(MAX_SYNC_BYTES) });
    if (s.baseVersion !== h.version) throw new CommandError('settings.stale', { version: String(h.version) });
    h.value = s.value;
    h.version += 1;
    return { userSettings: { key: 'themes', value: s.value, version: h.version, tag } };
  };
  return { h, ask, json: () => JSON.parse(decode(h.value)) as Record<string, unknown> };
}
// A device: its own browser storage, theme store, pack store and sync engine, with timers it runs by hand.
function device(opts: { kv?: ReturnType<typeof memory>; safe?: boolean; maxBytes?: number; now?: () => number } = {}) {
  const kv = opts.kv ?? memory();
  const themes = createThemeStore({ kv });
  const packs = createPackStore(kv);
  const pending: (() => void)[] = [];
  const sync = createThemeSync({ themes, packs, kv, safe: opts.safe, maxBytes: opts.maxBytes, now: opts.now, debounceMs: 1000, setTimer: (f) => { pending.push(f); return f; }, clearTimer: (t) => { const i = pending.indexOf(t as () => void); if (i >= 0) pending.splice(i, 1); } });
  const tick = async () => { while (pending.length) pending.shift()?.(); await sync.settled(); await sync.settled(); };
  return { kv, themes, packs, sync, pending, tick };
}
const local = (d: ReturnType<typeof device>) => ({ themes: d.themes.syncSnapshot(), packs: d.packs.syncSnapshot() });

describe('reading the hub\'s value', () => {
  it('reads nothing kept from an empty value or one of no known format, and refuses a newer one', () => {
    for (const text of ['', '{', '[]', '{"themes":[]}', '{"agora-theme-sync":"1"}']) expect(readSync(text)).toEqual({ ok: true, remote: EMPTY });
    expect(readSync('{"agora-theme-sync":2,"themes":[]}')).toEqual({ ok: false, newer: true });
  });

  it('checks every theme and pack with the file readers: one it cannot read is kept aside with its problems', () => {
    const bad = { uid: uid(2), edited: 5, file: { 'agora-theme': 2, name: 'Neon', dark: { tokens: { accent: 'red' } } } };
    const r = readSync(JSON.stringify({
      'agora-theme-sync': 1,
      themes: [{ uid: uid(1), edited: 4, file: file('Harbour') }, bad, { uid: 'not-a-uid', file: file('x') }, { uid: uid(1), file: file('again') }],
      packs: [{ uid: uid(3), added: 1, file: { 'agora-pack': 1, name: 'Empty' } }],
      choice: { dark: `u:${uid(1)}`, light: 'sand', at: 9 },
    }));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.remote.themes.map((t) => [t.uid, t.data.name])).toEqual([[uid(1), 'Harbour']]);
    expect(r.remote.invalid.map((i) => [i.kind, i.uid, i.name])).toEqual([['theme', uid(2), 'Neon'], ['pack', uid(3), 'Empty']]);
    expect(r.remote.invalid[0]?.issues.length).toBeGreaterThan(0);
    expect(r.remote.invalid[0]?.raw).toEqual(bad);
    expect(r.remote.choice).toEqual({ dark: `u:${uid(1)}`, light: 'sand', at: 9 });
  });
});

describe('merging', () => {
  const remoteOf = (v: Record<string, unknown>) => { const r = readSync(JSON.stringify({ 'agora-theme-sync': 1, ...v })); if (!r.ok) throw new Error('newer'); return r.remote; };

  it('is the union by lasting id, and the same themes give the same value whatever the order on each device', () => {
    const a = device();
    a.themes.import(night('Mine'));
    const m = merge(local(a), remoteOf({ themes: [{ uid: uid(1), edited: 3, file: file('Theirs') }] }));
    expect(m.themes.themes.map((t) => t.data.name)).toEqual(['Mine', 'Theirs']);
    const b = device();
    b.themes.applySync({ themes: [...m.themes.themes].reverse(), removed: [], choiceAt: 0 });
    const again = merge(local(b), remoteOf({}));
    expect((again.value.themes as { uid: string }[]).map((t) => t.uid)).toEqual((m.value.themes as { uid: string }[]).map((t) => t.uid));
  });

  it('takes the newer edit of a theme both sides have, this browser\'s on a tie', () => {
    const a = device();
    a.themes.import(night('Mine'));
    const { uid: id, edited } = a.themes.syncSnapshot().themes[0]!;
    const names = (e: number) => merge(local(a), remoteOf({ themes: [{ uid: id, edited: e, file: file('Edited elsewhere') }] })).themes.themes.map((t) => t.data.name);
    expect(names(edited + 1)).toEqual(['Edited elsewhere']);
    expect(names(edited)).toEqual(['Mine']);
    expect(names(edited - 1)).toEqual(['Mine']);
  });

  it('never drops a theme of this browser silently: a removal mark wins only when it is newer than the theme\'s last edit', () => {
    const a = device();
    a.themes.import(night('Mine'));
    const { uid: id, edited } = a.themes.syncSnapshot().themes[0]!;
    expect(merge(local(a), remoteOf({ themesRemoved: [{ uid: id, at: edited - 1 }] })).themes.themes).toHaveLength(1);
    expect(merge(local(a), remoteOf({})).themes.themes).toHaveLength(1); // a theme only this browser has is kept, and written
    expect(merge(local(a), remoteOf({})).value.themes).toHaveLength(1);
    expect(merge(local(a), remoteOf({ themesRemoved: [{ uid: id, at: edited + 1 }] })).themes.themes).toHaveLength(0);
    // And one removed here does not come back from another device.
    a.themes.remove(a.themes.state().user[0]!.id);
    const m = merge(local(a), remoteOf({ themes: [{ uid: id, edited, file: file('Mine') }] }));
    expect(m.themes.themes).toHaveLength(0);
    expect(m.value.themes).toEqual([]);
  });

  it('lets themes that do not fit wait, kept in the value and shown', () => {
    const a = device();
    for (let i = 0; i < MAX_USER_THEMES; i++) a.themes.import(night(`Mine ${i}`));
    const m = merge(local(a), remoteOf({ themes: [{ uid: uid(1), edited: 1, file: file('Late') }, { uid: uid(2), edited: 2, file: file('Later') }] }));
    expect(m.themes.themes).toHaveLength(MAX_USER_THEMES);
    expect(m.waiting.map((w) => w.name)).toEqual(['Later', 'Late']);
    expect(m.value.themes).toHaveLength(MAX_USER_THEMES + 2);
  });

  it('never applies what it cannot read, and writes it back as it was', () => {
    const bad = { uid: uid(7), edited: 1, file: { 'agora-theme': 9, name: 'From a newer window' }, extra: true };
    const m = merge(local(device()), remoteOf({ themes: [bad] }));
    expect(m.themes.themes).toEqual([]);
    expect(m.invalid.map((i) => i.uid)).toEqual([uid(7)]);
    expect(m.value.themes).toEqual([bad]);
  });

  it('takes the newer choice', () => {
    const a = device();
    a.themes.choose('slate');
    const at = a.themes.syncSnapshot().choiceAt;
    expect(at).toBeGreaterThan(0);
    expect(merge(local(a), remoteOf({ choice: { dark: 'large-text', light: 'sand', at: at + 1 } })).themes.choice).toEqual({ dark: 'large-text', light: 'sand' });
    expect(merge(local(a), remoteOf({ choice: { dark: 'large-text', light: 'sand', at } })).themes.choice).toEqual({ dark: 'large-text', light: 'sand' }); // a tie: the hub's
    expect(merge(local(a), remoteOf({ choice: { dark: 'large-text', light: 'sand', at: at - 1 } })).themes.choice).toBeUndefined();
    expect(merge(local(a), remoteOf({ choice: { dark: 'large-text', light: 'sand', at: at - 1 } })).value.choice).toMatchObject({ dark: 'slate', at });
  });
});

describe('the engine', () => {
  it('reads on sign-in, merges with this browser and writes the union; another device sees the same themes and choice', async () => {
    const hub = fakeHub();
    const laptop = device();
    laptop.themes.import(night('Harbour'));
    laptop.sync.attach(hub.ask);
    await laptop.tick();
    expect(hub.h.calls).toEqual(['get', 'set']);
    expect(laptop.sync.view().status).toBe('synced');
    const phone = device();
    phone.sync.attach(hub.ask);
    await phone.tick();
    expect(phone.themes.state().user.map((u) => u.name)).toEqual(['Harbour']);
    expect(phone.themes.active().name).toBe('Harbour'); // the choice came too
    expect(hub.h.calls).toEqual(['get', 'set', 'get']); // nothing new to write
    // A change on the phone reaches the laptop when it reads again (the window came back into view).
    phone.themes.choose('slate');
    await phone.tick();
    await laptop.sync.refresh();
    expect(laptop.themes.active().id).toBe('slate');
  });

  it('writes once after a burst of changes (debounced), and only what differs', async () => {
    const hub = fakeHub();
    const d = device();
    d.sync.attach(hub.ask);
    await d.tick();
    const writes = () => hub.h.calls.filter((c) => c === 'set').length;
    const before = writes();
    d.themes.import(night('One'));
    d.themes.import(night('Two'));
    d.themes.choose('sand');
    expect(d.pending).toHaveLength(1);
    await d.tick();
    expect(writes()).toBe(before + 1);
    d.themes.setMode('light'); // the scheme stays per device: nothing to write
    await d.tick();
    expect(writes()).toBe(before + 1);
  });

  it('refused as stale, reads again, merges and writes both devices\' themes', async () => {
    const hub = fakeHub();
    const a = device();
    const b = device();
    a.sync.attach(hub.ask);
    b.sync.attach(hub.ask);
    await a.tick();
    await b.tick();
    a.themes.import(night('From A'));
    await a.tick();
    b.themes.import(night('From B')); // b still holds the older version
    await b.tick();
    expect(hub.h.calls.filter((c) => c === 'set').length).toBeGreaterThanOrEqual(2);
    const names = (hub.json().themes as { file: { name: string } }[]).map((t) => t.file.name).sort();
    expect(names).toEqual(['From A', 'From B']);
    expect(b.themes.state().user.map((u) => u.name).sort()).toEqual(['From A', 'From B']);
    expect(b.sync.view().status).toBe('synced');
  });

  it('shows an invalid synced item with its problems and does not apply it', async () => {
    const hub = fakeHub({ 'agora-theme-sync': 1, themes: [{ uid: uid(5), edited: 1, file: { 'agora-theme': 2, name: 'Broken', dark: { tokens: { accent: 'red' } } } }] });
    const d = device();
    d.themes.import(night('Mine'));
    d.sync.attach(hub.ask);
    await d.tick();
    expect(d.themes.state().user.map((u) => u.name)).toEqual(['Mine']);
    expect(d.sync.view().invalid.map((i) => i.name)).toEqual(['Broken']);
    expect((hub.json().themes as { uid: string }[]).map((t) => t.uid)).toContain(uid(5)); // written back for the device that wrote it
    // "Remove from my devices" marks it removed, and the next write leaves it out.
    d.themes.forget(uid(5));
    await d.tick();
    expect((hub.json().themes as { uid: string }[]).map((t) => t.uid)).not.toContain(uid(5));
    expect(d.sync.view().invalid).toEqual([]);
  });

  it('agrees on the choice: one made before this version wins over a device that never chose, and a tie writes nothing', async () => {
    const hub = fakeHub();
    // The laptop chose Slate before choices had a time; the phone never chose.
    const laptop = device({ kv: memory({ [CHOICE_KEY]: JSON.stringify({ dark: 'slate', light: 'sand' }) }) });
    const phone = device();
    phone.sync.attach(hub.ask);
    await phone.tick();
    laptop.sync.attach(hub.ask);
    await laptop.tick();
    await phone.sync.refresh();
    expect(phone.themes.state().choice).toEqual({ dark: 'slate', light: 'sand' });
    expect(laptop.themes.state().choice).toEqual({ dark: 'slate', light: 'sand' });
    // Two devices whose choices have the same time take the hub's, and reading again writes nothing.
    const at = 1_795_600_000_000;
    const a = device({ kv: memory({ [CHOICE_KEY]: JSON.stringify({ dark: 'slate', light: 'light' }), [CHOICE_AT_KEY]: String(at) }) });
    const b = device({ kv: memory({ [CHOICE_KEY]: JSON.stringify({ dark: 'large-text', light: 'light' }), [CHOICE_AT_KEY]: String(at) }) });
    const tie = fakeHub();
    a.sync.attach(tie.ask);
    await a.tick();
    b.sync.attach(tie.ask);
    await b.tick();
    const writes = () => tie.h.calls.filter((c) => c === 'set').length;
    const before = writes();
    await a.sync.refresh();
    await b.sync.refresh();
    await a.sync.refresh();
    expect(writes()).toBe(before);
    expect(b.themes.state().choice.dark).toBe(a.themes.state().choice.dark);
  });

  it('keeps a newer choice it cannot show here for the devices that can, and its own choice on screen', async () => {
    const bad = { uid: uid(5), edited: 1, file: { 'agora-theme': 9, name: 'Newer' } };
    const hub = fakeHub({ 'agora-theme-sync': 1, themes: [bad], choice: { dark: `u:${uid(5)}`, light: 'sand', at: Date.now() + 60_000 } });
    const d = device();
    d.themes.choose('slate');
    d.sync.attach(hub.ask);
    await d.tick();
    expect(d.themes.state().choice).toEqual({ dark: 'slate', light: 'sand' });
    expect(hub.json().choice).toMatchObject({ dark: `u:${uid(5)}`, light: 'sand' });
  });

  it('never syncs in safe mode, when turned off, or with no hub; an older hub means local only', async () => {
    const hub = fakeHub();
    const safe = device({ safe: true });
    safe.sync.attach(hub.ask);
    safe.themes.import(night());
    await safe.tick();
    expect(safe.sync.view().status).toBe('safe');
    expect(hub.h.calls).toEqual([]);

    const off = device({ kv: memory({ [SETTING_KEY]: 'off' }) });
    off.sync.attach(hub.ask);
    off.themes.import(night());
    await off.tick();
    expect(off.sync.view().status).toBe('off');
    expect(hub.h.calls).toEqual([]);
    off.sync.setEnabled(true);
    await off.tick();
    expect(hub.h.calls).toEqual(['get', 'set']);
    expect(off.kv.m.has(SETTING_KEY)).toBe(false);

    const demo = device();
    demo.sync.attach(undefined, false);
    expect(demo.sync.view().status).toBe('local');

    // An older hub refuses a command it cannot name; the window keeps its themes here and asks no more on this connection.
    let asked = 0;
    const older: SyncAsk = async () => { asked++; throw new CommandError('auth.forbidden', { action: 'None' }); };
    const d = device();
    d.sync.attach(older);
    await d.tick();
    expect(d.sync.view().status).toBe('older');
    d.themes.import(night());
    await d.tick();
    expect(asked).toBe(1);
    expect(d.themes.state().user).toHaveLength(1);
    // A hub that knows the commands but could not answer is not an older hub: it is tried again at the next change.
    let down = true;
    const flaky = fakeHub();
    const e = device();
    e.sync.attach(async (body) => { if (down) throw new CommandError('node.timeout'); return flaky.ask(body); });
    await e.tick();
    expect(e.sync.view().status).toBe('error');
    down = false;
    e.themes.import(night('After'));
    await e.tick();
    expect(e.sync.view().status).toBe('synced');
    expect(flaky.h.calls).toEqual(['get', 'set']);
  });

  it('writes nothing over the cap, and never writes over a value from a newer window', async () => {
    // The cap is the hub's 256 KiB; a smaller one here keeps the test small.
    const cap = 40 * 1024;
    expect(MAX_SYNC_BYTES).toBe(256 * 1024);
    const hub = fakeHub();
    const d = device({ maxBytes: cap });
    d.sync.attach(hub.ask);
    await d.tick();
    const huge = { 'agora-pack': 1, name: 'Huge', themes: Array.from({ length: 20 }, (_, i) => writeTheme(sand, `T${i}`)) };
    for (let i = 0; i < 4; i++) d.packs.keep({ ...huge, name: `Huge ${i}` });
    expect(serialiseSync({ packs: d.packs.list().map((p) => p.file) }).length).toBeGreaterThan(cap);
    const before = hub.h.calls.length;
    await d.tick();
    expect(d.sync.view().status).toBe('too-large');
    expect(hub.h.calls.length).toBe(before);
    expect(d.packs.list()).toHaveLength(4); // this browser keeps everything

    const newer = fakeHub({ 'agora-theme-sync': 2, themes: [] });
    const n = device();
    n.themes.import(night());
    n.sync.attach(newer.ask);
    await n.tick();
    expect(n.sync.view().status).toBe('newer');
    expect(newer.h.calls).toEqual(['get']);
  });

  it('carries packs to another device, where nothing of them is applied until picked', async () => {
    const hub = fakeHub();
    const a = device();
    a.sync.attach(hub.ask);
    await a.tick();
    a.packs.keep(writePack({ name: 'Harbour set', themes: [{ theme: sand, name: 'Harbour' }], layouts: [] }));
    await a.tick();
    const b = device();
    b.sync.attach(hub.ask);
    await b.tick();
    expect(b.packs.list().map((p) => p.pack.name)).toEqual(['Harbour set']);
    expect(b.themes.state().user).toEqual([]);
    // Forgotten on b, it goes from a at a's next read.
    b.packs.forget(b.packs.list()[0]!.uid);
    await b.tick();
    await a.sync.refresh();
    expect(a.packs.list()).toEqual([]);
  });
});

describe('one browser, two people (security review F1)', () => {
  const names = (hub: ReturnType<typeof fakeHub>) => (hub.h.value.length ? (hub.json().themes as { file: { name: string } }[]).map((t) => t.file.name) : []);
  // One browser: the same storage for whoever signs in.
  function browser() {
    const d = device();
    const signIn = async (hub: ReturnType<typeof fakeHub>) => { d.sync.attach(undefined, false); d.sync.attach(hub.ask); await d.tick(); };
    return { ...d, signIn };
  }

  it('the second person does not get the first person\'s themes, nor the first the second\'s', async () => {
    const a = fakeHub();
    const b = fakeHub();
    const br = browser();
    await br.signIn(a);
    br.themes.import(night('Eda - job interview at Rival Corp'));
    await br.tick();
    expect(names(a)).toEqual(['Eda - job interview at Rival Corp']);
    await br.signIn(b);
    expect(names(b)).toEqual([]);
    expect(br.themes.state().user).toEqual([]); // this browser now shows B's own themes (none)
    br.themes.import(night('Bo private'));
    await br.tick();
    expect(names(b)).toEqual(['Bo private']);
    await br.signIn(a);
    expect(names(a)).toEqual(['Eda - job interview at Rival Corp']);
    expect(br.themes.state().user.map((u) => u.name)).toEqual(['Eda - job interview at Rival Corp']);
  });

  it('keeps the second person\'s choice, and the first person\'s removal marks and packs out of the second\'s value', async () => {
    const b = fakeHub({ 'agora-theme-sync': 1, themes: [], themesRemoved: [], packs: [], packsRemoved: [], choice: { dark: 'slate', light: 'light', at: Date.now() - 86_400_000 } });
    const a = fakeHub();
    const br = browser();
    await br.signIn(a);
    br.themes.import(night('A work theme'));
    br.themes.remove(br.themes.state().user[0]!.id);
    br.themes.import(night('A kept theme'));
    br.packs.keep(writePack({ name: 'A pack', themes: [{ theme: sand, name: 'Harbour' }], layouts: [] }));
    await br.tick();
    await br.signIn(b);
    const v = b.json();
    expect(v.choice).toMatchObject({ dark: 'slate', light: 'light' });
    expect(br.themes.state().choice.dark).toBe('slate');
    expect(v.themes).toEqual([]);
    expect(v.themesRemoved).toEqual([]);
    expect(v.packs).toEqual([]);
    expect(br.packs.list()).toEqual([]);
  });

  it('nothing the second person removes reaches the first person\'s value', async () => {
    const a = fakeHub();
    const b = fakeHub();
    const br = browser();
    await br.signIn(a);
    br.themes.import(night('A work theme'));
    await br.tick();
    await br.signIn(b);
    // B never sees A's theme, so nothing of it can be removed in B's name; B removes one of their own.
    expect(br.themes.state().user).toEqual([]);
    br.themes.import(night('B temp'));
    br.themes.remove(br.themes.state().user[0]!.id);
    await br.tick();
    await br.signIn(a);
    expect(names(a)).toEqual(['A work theme']);
    expect(a.json().themesRemoved).toEqual([]);
  });

  it('a value with no tag after one with a tag is someone else\'s too; a browser\'s first sync still merges what it had', async () => {
    const br = browser();
    br.themes.import(night('Mine before syncing'));
    const first = fakeHub();
    await br.signIn(first);
    expect(names(first)).toEqual(['Mine before syncing']); // the first sync of this browser
    const untagged = fakeHub(undefined, '');
    await br.signIn(untagged);
    expect(names(untagged)).toEqual([]);
    expect(br.themes.state().user).toEqual([]);
  });
});

describe('times from a clock that is ahead (security review F2)', () => {
  it('a choice or an edit dated in the future is cut back to the hub\'s now, so a later pick wins once the slack has passed', async () => {
    let clock = 1_795_600_000_000;
    const future = clock + 365 * 86_400_000;
    const hub = fakeHub({ 'agora-theme-sync': 1, themes: [{ uid: uid(9), edited: future, file: file('From the future') }], themesRemoved: [], packs: [], packsRemoved: [], choice: { dark: 'large-text', light: 'light', at: future } });
    const d = device({ now: () => clock });
    d.sync.attach(hub.ask);
    await d.tick();
    const cut = clock + CLOCK_SLACK_MS;
    expect(hub.json().choice).toMatchObject({ at: cut });
    expect((hub.json().themes as { edited: number }[])[0]!.edited).toBe(cut);
    clock += CLOCK_SLACK_MS + 1000;
    // The device's own clock is the hub's here: a pick made now is newer than the cut-back one.
    const realNow = Date.now;
    Date.now = () => clock;
    try {
      d.themes.choose('slate');
      await d.tick();
      await d.sync.refresh();
    } finally { Date.now = realNow; }
    expect(d.themes.state().choice.dark).toBe('slate');
    expect(hub.json().choice).toMatchObject({ dark: 'slate' });
  });
});

describe('fields of a newer window (security review F4)', () => {
  it('writes unknown top-level fields back as they were', async () => {
    const hub = fakeHub({ 'agora-theme-sync': 1, themes: [], layouts: [{ name: 'a newer window\'s' }], zz: 7 });
    const d = device();
    d.themes.import(night('Mine'));
    d.sync.attach(hub.ask);
    await d.tick();
    expect(hub.json().layouts).toEqual([{ name: 'a newer window\'s' }]);
    expect(hub.json().zz).toBe(7);
    expect((hub.json().themes as unknown[]).length).toBe(1);
  });
});
