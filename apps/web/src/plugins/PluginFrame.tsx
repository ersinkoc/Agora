// A plugin's pane body (ADR-0114 decisions 2 and 3; docs/ux/plugins.md): its sandboxed frame and the bridge that answers it. The window's
// pane bar above it (Workspace.tsx) carries the plugin's name and the "plugin" marker; the frame draws only inside the pane body, and
// nothing it does can reach outside that box. The bridge answers only this frame, only the capabilities the person granted, and only
// from the plugin view (projection.ts): never the built-in panels' context, state or actions.
import { useEffect, useRef, useState } from 'react';
import { directionOf, getLanguage, t } from '../i18n';
import { mergeLines, type ChatLine } from '../net/chat';
import { themeStore } from '../themes/store';
import { COLOR_TOKENS } from '../themes/tokens';
import type { Capability } from './capabilities';
import { FRAME_SANDBOX, frameUrl } from './frameDoc';
import { type CheckedPlugin, type PluginPanelDecl, typeOf } from './package';
import { type PluginView, type ThemeFacts, project, projectChanges, projectChat } from './projection';
import { type Inbound, type MessageRouter, type Outbound, MAX_WATCHES, frameFilter, gate, messageRouter } from './protocol';

// One listener for every plugin frame of this window (protocol.ts `messageRouter`): a message is taken only from a frame's own window.
let shared: MessageRouter | undefined;
function router(): MessageRouter {
  if (!shared) {
    const r = messageRouter();
    window.addEventListener('message', (e) => { r.receive(e); });
    shared = r;
  }
  return shared;
}

/** The theme's colours as the window draws them now. */
export function themeFacts(): ThemeFacts {
  const css = getComputedStyle(document.documentElement);
  const tokens: Record<string, string> = {};
  for (const k of COLOR_TOKENS) { const v = css.getPropertyValue(`--${k}`).trim(); if (v) tokens[k] = v; }
  return { scheme: themeStore.scheme(), tokens };
}

const FLUSH_MS = 250;
const READ_TIMEOUT_MS = 5000;

interface Watch { cap: Capability; last?: string; close?: () => void; lines?: ChatLine[]; key?: string }

export function PluginFrame({ plugin, panel, granted, view, dev }: { plugin: CheckedPlugin; panel: PluginPanelDecl; granted: readonly Capability[]; view: PluginView; dev: boolean }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const latest = useRef(view);
  latest.current = view;
  const [dropped, setDropped] = useState(0);
  const flushNow = useRef<() => void>(() => {});
  const m = plugin.manifest;
  useEffect(() => {
    const win = frame.current?.contentWindow;
    if (!win) return;
    const filter = frameFilter();
    const watches = new Map<number, Watch>();
    let booted = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let alive = true;
    const post = (msg: Outbound) => { if (alive) frame.current?.contentWindow?.postMessage(msg, '*'); };
    const answer = (id: number, data: unknown, first: boolean) => post(first ? { agora: 1, type: 'result', id, ok: true, data } : { agora: 1, type: 'update', id, data });
    const count = () => { const c = filter.counts; setDropped(c.type + c.size + c.rate); };

    // Sends what changed since the last answer of each watch. Changes come from the window's state (every render) and the theme.
    const flush = () => {
      timer = undefined;
      const v = latest.current;
      for (const [id, w] of watches) {
        if (w.cap === 'chat.read') continue;
        if (w.cap === 'changes.read') { if (v.changesKey !== w.key) void readChanges(id, w, false); continue; }
        const data = project(w.cap, v, themeFacts);
        const s = JSON.stringify(data);
        if (s !== w.last) { w.last = s; answer(id, data, false); }
      }
    };
    flushNow.current = () => { if (!timer && watches.size > 0) timer = setTimeout(flush, FLUSH_MS); };

    const readChanges = async (id: number, w: Watch | undefined, first: boolean) => {
      const v = latest.current;
      const agent = v.pane.agent;
      if (w) w.key = v.changesKey;
      if (!v.diff || !agent) { if (first) post({ agora: 1, type: 'result', id, ok: false, error: 'unavailable' }); return; }
      try {
        const data = projectChanges(agent.id, await v.diff());
        if (w && !watches.has(id)) return;
        const s = JSON.stringify(data);
        if (w) { if (s === w.last) return; w.last = s; }
        answer(id, data, first);
      } catch { if (first) post({ agora: 1, type: 'result', id, ok: false, error: 'unavailable' }); }
    };
    // The bound agent's conversation: a read takes the first batch and closes; a watch keeps the stream and sends each change.
    const readChat = (id: number, w: Watch | undefined) => {
      const v = latest.current;
      const agent = v.pane.agent;
      if (!v.chat || !agent) { post({ agora: 1, type: 'result', id, ok: false, error: 'unavailable' }); return; }
      let first = true;
      let lines: ChatLine[] = [];
      const give = () => { const data = projectChat(agent.id, lines); answer(id, data, first); first = false; };
      const timeout = setTimeout(() => { if (first) { conn.close(); post({ agora: 1, type: 'result', id, ok: false, error: 'unavailable' }); first = false; } }, READ_TIMEOUT_MS);
      const conn = v.chat.connect((batch, replace) => {
        lines = replace ? [...batch] : mergeLines(lines, batch);
        if (!w) { if (first) { clearTimeout(timeout); give(); queueMicrotask(() => conn.close()); } return; }
        clearTimeout(timeout);
        give();
      });
      if (w) w.close = () => { clearTimeout(timeout); conn.close(); };
    };

    const on = (msg: Inbound) => {
      if (msg.type === 'boot') {
        if (booted) return;
        booted = true;
        const files = [...plugin.files].map(([name, bytes]) => ({ name, type: typeOf(name) ?? 'application/octet-stream', bytes }));
        post({ agora: 1, type: 'load', plugin: { id: m.id, name: m.name, version: m.version }, panel: panel.id, granted: [...granted], entry: m.entry, scheme: themeStore.scheme(), lang: getLanguage(), dir: directionOf(getLanguage()), files });
        return;
      }
      if (msg.type === 'unwatch') { watches.get(msg.id)?.close?.(); watches.delete(msg.id); return; }
      const g = gate(msg.cap, granted);
      if (!g.ok) { post({ agora: 1, type: 'result', id: msg.id, ok: false, error: g.error }); return; }
      if (msg.type === 'watch' && (watches.size >= MAX_WATCHES || watches.has(msg.id))) { post({ agora: 1, type: 'result', id: msg.id, ok: false, error: 'too-many' }); return; }
      const w: Watch | undefined = msg.type === 'watch' ? { cap: g.cap } : undefined;
      if (w) watches.set(msg.id, w);
      if (g.cap === 'chat.read') { readChat(msg.id, w); return; }
      if (g.cap === 'changes.read') { void readChanges(msg.id, w, true); return; }
      const data = project(g.cap, latest.current, themeFacts);
      if (w) w.last = JSON.stringify(data);
      answer(msg.id, data, true);
    };
    const off = router().add(win, filter, (msg) => on(msg));
    const counter = setInterval(count, 1000);
    const theme = themeStore.subscribe(() => flushNow.current());
    return () => {
      alive = false;
      off();
      theme();
      clearInterval(counter);
      if (timer) clearTimeout(timer);
      for (const w of watches.values()) w.close?.();
      watches.clear();
      flushNow.current = () => {};
    };
  }, [plugin, panel, granted, m]);

  // Every new view (the window's state moved) is compared with what each watch last sent.
  useEffect(() => { flushNow.current(); });

  return (
    <iframe ref={frame} src={frameUrl(import.meta.env.BASE_URL)} sandbox={FRAME_SANDBOX} referrerPolicy="no-referrer" title={t('plugin-frame-title', { name: panel.name ?? m.name })}
      className="block min-h-0 w-full flex-1 border-0 bg-transparent" data-plugin-frame={m.id} data-plugin-dev={dev ? '' : undefined} data-dropped={dropped} />
  );
}
