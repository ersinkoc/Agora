// @ts-check
/// <reference path="agora-plugin.d.ts" />
// A new Agora plugin: one panel that counts the agents by state, in the window's colours. Change anything here; the manifest
// (agora-plugin.json) says what the plugin asks to see, and README.md says how to try it in the window.
(function () {
  'use strict';
  var title = document.createElement('h1');
  title.textContent = agora.plugin.name;
  var list = document.createElement('ul');
  var note = document.createElement('p');
  note.className = 'note';
  document.body.append(title, list, note);

  /** @param {AgoraPlugin.Theme} theme */
  function paint(theme) {
    var style = document.documentElement.style;
    Object.entries(theme.tokens).forEach(function (t) { style.setProperty('--' + t[0], t[1]); });
    style.colorScheme = theme.scheme;
  }

  /** @param {AgoraPlugin.Answers['agents.read']} data */
  function render(data) {
    /** @type {Record<string, number>} */
    var counts = {};
    data.agents.forEach(function (a) { if (!a.closed) counts[a.state] = (counts[a.state] || 0) + 1; });
    list.replaceChildren();
    Object.keys(counts).sort().forEach(function (state) {
      var li = document.createElement('li');
      // Draw other people's text with textContent, never innerHTML.
      li.textContent = state + ': ' + counts[state];
      list.append(li);
    });
    note.textContent = list.children.length === 0 ? 'No agents are running.' : '';
  }

  /** @param {Error} e */
  function refused(e) { note.textContent = 'Cannot show the agents: ' + e.message; }

  agora.watch('theme.read', paint);
  agora.watch('agents.read', render, refused);
})();
