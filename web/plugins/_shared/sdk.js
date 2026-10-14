// OpenSanctuary plugin SDK, served to every sandboxed plugin at `__os/sdk.js` (alongside
// `__os/theme.css`). It wraps the postMessage bridge and keeps the page themed like the console.
//
//   OS.call(method, params)  -> Promise   a bridge request (see docs/PLUGIN_PERMISSIONS.md)
//   OS.on(event, callback)                events the console pushes, e.g. 'state' (needs show.observe)
//   OS.slot                               where the page is shown: 'dock', 'import-tab', 'library-source'
//   OS.resizeToContent()                  for a 'dock' page: fit the panel's height to the content
(function (root) {
  'use strict';
  var pending = {};
  var listeners = {};
  var nextId = 0;

  function call(method, params) {
    return new Promise(function (resolve, reject) {
      var id = ++nextId;
      pending[id] = { resolve: resolve, reject: reject };
      parent.postMessage({ type: 'os.request', id: id, method: method, params: params }, '*');
    });
  }

  function applyTheme(vars) {
    if (!vars) return;
    var style = document.documentElement.style;
    Object.keys(vars).forEach(function (name) {
      if (name.indexOf('--') === 0) style.setProperty(name, vars[name]);
    });
  }

  window.addEventListener('message', function (ev) {
    if (ev.source !== parent) return;
    var m = ev.data;
    if (!m) return;
    if (m.type === 'os.response' && pending[m.id]) {
      var p = pending[m.id];
      delete pending[m.id];
      if (m.ok) p.resolve(m.result); else p.reject(new Error(m.error));
    } else if (m.type === 'os.event') {
      (listeners[m.event] || []).forEach(function (cb) { try { cb(m.data); } catch (e) { console.error(e); } });
    } else if (m.type === 'os.theme') {
      applyTheme(m.vars);
    }
  });

  var params = new URLSearchParams(location.search);
  var api = {
    call: call,
    on: function (event, cb) { (listeners[event] = listeners[event] || []).push(cb); },
    slot: params.get('slot') || 'dock',
    resizeToContent: function () {
      var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
      return call('ui.resize', { height: h }).catch(function () {});
    }
  };
  root.OS = api;

  // The console also pushes theme changes; ask once up front so the first paint matches.
  call('ui.theme').then(function (r) { applyTheme(r && r.vars); }).catch(function () {});
})(window);
