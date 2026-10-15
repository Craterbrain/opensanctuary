// OpenSanctuary plugin SDK, served to every sandboxed plugin at `__os/sdk.js` (alongside
// `__os/theme.css`). It wraps the postMessage bridge and keeps the page themed like the console.
//
//   OS.call(method, params)  -> Promise   a bridge request (see docs/PLUGIN_PERMISSIONS.md)
//   OS.on(event, callback)                events the console pushes, e.g. 'state' (needs show.observe)
//   OS.slot                               where the page is shown: 'dock', 'import-tab', 'library-source'
//   OS.notify(title, message, opts)       post to the console's notification center (needs notifications.post)
//                                         opts: level 'info'|'success'|'warning'|'error', key (replaces an earlier
//                                         one with the same key), sticky (warning/error stay until dismissed)
//   OS.resizeToContent()                  for a 'dock' page: fit the panel's height to the content
//   OS.icon(name)                         HTML for one of the console's icons (themed like the console's)
//   OS.iconify(root)                      swap emoji / {icon:name} tokens under root for those icons
//
// Icons: load `__os/icons.js` before this file and emoji are swapped for the console's icons
// automatically (and `{icon:name}` renders that icon), exactly as in the console, following its icon
// set. Text that can't hold an image (placeholder=, title=, <option>) should stay plain text.
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

  // ---- icons ------------------------------------------------------------------------------------
  var EMOJI = root.__OS_EMOJI || {};
  var iconSet = 'orange';
  var VS16 = '\uFE0F';
  var escapeRe = function (s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); };
  var glyphs = Object.keys(EMOJI).sort(function (a, b) { return b.length - a.length; }).map(escapeRe);
  var matcher = new RegExp((glyphs.length ? '(?:' + glyphs.join('|') + ')' + VS16 + '?|' : '') + '\\{icon:([a-z0-9-]+)\\}', 'gu');
  var SKIP_TAGS = { SCRIPT: 1, STYLE: 1, NOSCRIPT: 1, TEXTAREA: 1, INPUT: 1, SELECT: 1, OPTION: 1, TITLE: 1 };

  function iconUrl(name) { return '__os/icons/' + iconSet + '/' + name + '.svg'; }
  function icon(name) {
    return '<img class="ui-icon" data-icon="' + name + '" src="' + iconUrl(name) + '" alt="" draggable="false">';
  }
  function iconElement(name) {
    var img = document.createElement('img');
    img.className = 'ui-icon';
    img.dataset.icon = name;
    img.src = iconUrl(name);
    img.alt = '';
    img.draggable = false;
    return img;
  }
  function skip(el) {
    return !el || SKIP_TAGS[el.tagName] || (el.closest && el.closest('[contenteditable], [data-no-iconify]'));
  }
  function iconifyText(node) {
    var text = node.nodeValue;
    if (!text || skip(node.parentElement)) return;
    var frag = null;
    var last = 0;
    text.replace(matcher, function (m, token, offset) {
      var name = token || EMOJI[m.split(VS16).join('')];
      if (!name) return m;
      frag = frag || document.createDocumentFragment();
      if (offset > last) frag.appendChild(document.createTextNode(text.slice(last, offset)));
      frag.appendChild(iconElement(name));
      last = offset + m.length;
      return m;
    });
    if (!frag) return;
    if (last < text.length) frag.appendChild(document.createTextNode(text.slice(last)));
    node.parentNode.replaceChild(frag, node);
  }
  function iconify(rootNode) {
    if (!rootNode) return;
    if (rootNode.nodeType === 3) { iconifyText(rootNode); return; }
    if (rootNode.nodeType !== 1 || skip(rootNode)) return;
    var walker = document.createTreeWalker(rootNode, NodeFilter.SHOW_TEXT);
    var found = [];
    for (var n = walker.nextNode(); n; n = walker.nextNode()) found.push(n);
    found.forEach(iconifyText);
  }
  function setIconSet(id) {
    if (!id || id === iconSet) return;
    iconSet = id;
    document.querySelectorAll('img[data-icon]').forEach(function (img) {
      img.setAttribute('src', iconUrl(img.dataset.icon));
    });
  }
  function startIcons() {
    iconify(document.body);
    var observer = new MutationObserver(function (records) {
      records.forEach(function (r) {
        if (r.type === 'characterData') iconify(r.target);
        else r.addedNodes.forEach(function (n) { iconify(n); });
      });
      observer.takeRecords(); // our own replacements aren't news
    });
    observer.observe(document.body, { childList: true, subtree: true, characterData: true });
  }
  if (document.body) startIcons(); else document.addEventListener('DOMContentLoaded', startIcons);

  function applyTheme(vars, set) {
    var style = document.documentElement.style;
    Object.keys(vars || {}).forEach(function (name) {
      if (name.indexOf('--') === 0) style.setProperty(name, vars[name]);
    });
    setIconSet(set);
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
      applyTheme(m.vars, m.iconSet);
    } else if (m.type === 'os.ping') {
      // The console's watchdog: a page that stops answering is removed for the session.
      parent.postMessage({ type: 'os.pong', n: m.n }, '*');
    }
  });

  var params = new URLSearchParams(location.search);
  var api = {
    call: call,
    on: function (event, cb) { (listeners[event] = listeners[event] || []).push(cb); },
    slot: params.get('slot') || 'dock',
    icon: icon,
    iconify: iconify,
    notify: function (title, message, opts) {
      opts = opts || {};
      return call('notify', { title: title, message: message, level: opts.level, key: opts.key, sticky: opts.sticky });
    },
    resizeToContent: function () {
      var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
      return call('ui.resize', { height: h }).catch(function () {});
    }
  };
  root.OS = api;

  // The console also pushes theme changes; ask once up front so the first paint matches.
  call('ui.theme').then(function (r) { applyTheme(r && r.vars, r && r.iconSet); }).catch(function () {});
})(window);
