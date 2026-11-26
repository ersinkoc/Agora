// The window's loader inside a plugin's sandboxed frame (ADR-0114 and its Amendment A; docs/ux/plugins.md, "The frame"). It is the
// window's own code, the one script of the frame page the hub serves (`plugin-frame.html`, under its own header policy); the plugin's
// files arrive from the window by message and run from blobs made here, inside the frame's opaque origin (a blob the window made is not
// loadable in an opaque frame).
//
// It runs only in an opaque frame inside another page, takes only the window's one `load` message, and then gives the plugin `agora`:
//   agora.read(cap)                 a Promise of the answer; rejected with an Error whose message is the code ("not-granted", …)
//   agora.watch(cap, onData, onErr) answers now and on every change; returns a function that stops it
//   agora.file(name)                a blob: address, made in the frame, of one of the plugin's own images or fonts, or undefined
//   agora.granted, agora.plugin, agora.panel, agora.scheme
(function () {
  'use strict';
  // WebRTC is the one way out a page policy does not close in every engine (`connect-src` does not cover it, and `webrtc 'block'` is
  // not enforced everywhere): its constructors are taken away for good before anything of the plugin's runs, in this frame and in any
  // frame the plugin makes that runs this loader too. A frame the plugin makes without it has an opaque origin of its own (the
  // sandbox), so this one cannot reach into it, and no plugin script can run there (`script-src` allows no inline code, and a blob
  // made here does not load there).
  Object.getOwnPropertyNames(window).forEach(function (k) {
    if (!/^(webkit|moz)?RTC/.test(k)) return;
    try { Object.defineProperty(window, k, { value: undefined, writable: false, configurable: false, enumerable: false }); } catch (e) { /* already fixed */ }
  });
  if (self.origin !== 'null' || window.parent === window) return;
  var host = window.parent;
  var loaded = false;
  var next = 1;
  var pending = new Map();
  var watches = new Map();
  var files = new Map();
  function send(m) { m.agora = 1; host.postMessage(m, '*'); }
  function text(bytes) { return new TextDecoder().decode(bytes); }
  var urls = new Map();
  function blobUrl(f) {
    if (!urls.has(f.name)) urls.set(f.name, URL.createObjectURL(new Blob([f.bytes], { type: f.type })));
    return urls.get(f.name);
  }
  function start(m) {
    for (var i = 0; i < m.files.length; i++) files.set(m.files[i].name, m.files[i]);
    document.documentElement.style.colorScheme = m.scheme === 'light' ? 'light' : 'dark';
    if (typeof m.lang === 'string' && /^[A-Za-z]{2,3}(-[A-Za-z0-9]{1,8})*$/.test(m.lang)) document.documentElement.lang = m.lang;
    document.documentElement.dir = m.dir === 'rtl' ? 'rtl' : 'ltr';
    m.files.forEach(function (f) {
      if (f.type !== 'text/css') return;
      var s = document.createElement('style');
      s.textContent = text(f.bytes);
      document.head.appendChild(s);
    });
    var api = {
      version: 1,
      plugin: Object.freeze({ id: m.plugin.id, name: m.plugin.name, version: m.plugin.version }),
      panel: m.panel,
      scheme: m.scheme,
      granted: Object.freeze(m.granted.slice()),
      read: function (cap) {
        var id = next++;
        return new Promise(function (resolve, reject) {
          pending.set(id, { resolve: resolve, reject: function (code) { reject(new Error(code)); } });
          send({ type: 'read', id: id, cap: cap });
        });
      },
      watch: function (cap, onData, onError) {
        var id = next++;
        watches.set(id, { data: onData, error: onError || function () {} });
        send({ type: 'watch', id: id, cap: cap });
        return function () { if (watches.delete(id)) send({ type: 'unwatch', id: id }); };
      },
      file: function (name) {
        var f = files.get(name);
        return f && /^(image|font)\//.test(f.type) ? blobUrl(f) : undefined;
      },
    };
    Object.defineProperty(window, 'agora', { value: Object.freeze(api), enumerable: true });
    var entry = files.get(m.entry);
    if (!entry) return;
    var s = document.createElement('script');
    s.src = URL.createObjectURL(new Blob([entry.bytes], { type: 'text/javascript' }));
    document.head.appendChild(s);
  }
  addEventListener('message', function (e) {
    if (e.source !== host) return;
    var m = e.data;
    if (!m || typeof m !== 'object' || m.agora !== 1) return;
    if (m.type === 'load') { if (!loaded) { loaded = true; start(m); } return; }
    if (m.type === 'result') {
      var p = pending.get(m.id);
      if (p) { pending.delete(m.id); if (m.ok) p.resolve(m.data); else p.reject(m.error); return; }
      var w = watches.get(m.id);
      if (w) { if (m.ok) w.data(m.data); else { watches.delete(m.id); w.error(new Error(m.error)); } }
      return;
    }
    if (m.type === 'update') { var u = watches.get(m.id); if (u) u.data(m.data); }
  });
  send({ type: 'boot' });
})();
