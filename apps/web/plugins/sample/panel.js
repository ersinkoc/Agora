// A sample plugin for sandboxed panels (ADR-0114; docs/ux/plugins.md): the agents' names and states, in the window's colours.
// It runs in its own frame with no network and sees only what the person allowed: `agents.read` and `theme.read`.
(function () {
  'use strict';
  var root = document.createElement('main');
  var title = document.createElement('h1');
  title.textContent = 'Agents';
  var list = document.createElement('ul');
  var note = document.createElement('p');
  note.className = 'note';
  root.append(title, list, note);
  document.body.append(root);

  function paint(theme) {
    var s = document.documentElement.style;
    Object.keys(theme.tokens).forEach(function (k) { s.setProperty('--' + k, theme.tokens[k]); });
    s.colorScheme = theme.scheme;
  }
  function render(data) {
    list.replaceChildren();
    data.agents.filter(function (a) { return !a.closed; }).forEach(function (a) {
      var li = document.createElement('li');
      var dot = document.createElement('span');
      dot.className = 'dot dot--' + a.state;
      var name = document.createElement('span');
      name.className = 'name';
      name.textContent = a.name;
      var where = document.createElement('span');
      where.className = 'where';
      where.textContent = a.machine;
      var state = document.createElement('span');
      state.className = 'state';
      state.textContent = a.state.replace('-', ' ');
      li.append(dot, name, where, state);
      list.append(li);
    });
    note.textContent = list.children.length === 0 ? 'No agents are running.' : '';
    document.body.dataset.ready = '';
  }
  function refused(e) { note.textContent = 'Cannot show agents: ' + e.message; document.body.dataset.ready = ''; }

  agora.watch('theme.read', paint, function () {});
  agora.watch('agents.read', render, refused);
})();
