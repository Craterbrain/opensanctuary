// A deliberately hostile sandboxed plugin, for tests only (never shipped under web/plugins).
// It asks for a broad set of permissions (the operator approves them all, the worst realistic case)
// and then, on command, tries to escape the sandbox, exceed its grants, forge messages and take the
// console down. tests/e2e_hostile_plugin.test.ts runs it against a real server.

export const HOSTILE_ID = "hostile_test";

export const hostileManifest = {
  manifest_version: 1,
  id: HOSTILE_ID,
  name: "Hostile Test Plugin",
  version: "1.0.0",
  publisher: "Red Team",
  description: "Test fixture: attacks the console. Never install.",
  web: { entry: "index.html", sandbox: true },
  permissions: [
    { id: "show.observe", reason: "x" },
    { id: "ui.panel", reason: "x" },
    { id: "ui.toolbar", reason: "x" },
    { id: "notifications.post", reason: "x" },
    { id: "keyring.own", reason: "x" },
    { id: "commands.send", commands: ["NextSlide"], reason: "x" },
    { id: "network.outbound", hosts: ["127.0.0.1"], reason: "x" },
  ],
  ui: { placements: [{ slot: "dock" }], toolbar: [{ id: "go", label: "Evil", icon: "next", tooltip: "x" }] },
};

export const hostileHtml = `<!doctype html><html><head><meta charset="utf-8"><title>Hostile</title>
<link rel="stylesheet" href="__os/theme.css"></head><body><div id="status">hostile</div>
<script src="__os/icons.js"></script><script src="__os/sdk.js"></script><script src="app.js"></script></body></html>`;

// Each attack resolves to a short string describing what happened. Nothing here throws out of the page.
export const hostileJs = String.raw`
(function () {
  'use strict';
  var T = function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); };
  var call = function (m, p) { return OS.call(m, p).then(function (r) { return 'OK ' + JSON.stringify(r).slice(0, 160); }, function (e) { return 'ERR ' + String(e && e.message).slice(0, 160); }); };
  var attacks = {};

  // ---- escape attempts ----
  attacks.parentDom = function () { try { return 'READ ' + parent.document.title; } catch (e) { return 'blocked'; } };
  attacks.topNav = function () { try { top.location = 'http://127.0.0.1:1/pwn'; return 'no-throw'; } catch (e) { return 'blocked'; } };
  attacks.windowOpen = function () { var w = null; try { w = window.open('about:blank'); } catch (e) {} return w ? 'OPENED' : 'blocked'; };
  attacks.modals = function () { var r = []; try { alert('x'); r.push('alert'); } catch (e) {} try { r.push(String(prompt('x'))); } catch (e) {} return 'ret ' + r.join(','); };
  attacks.storage = function () { var r = []; try { localStorage.setItem('a', 'b'); r.push('ls'); } catch (e) {} try { document.cookie = 'a=b'; r.push(document.cookie ? 'cookie' : ''); } catch (e) {} try { indexedDB.open('x'); r.push('idb'); } catch (e) {} return 'wrote [' + r.join(',') + ']'; };
  attacks.formToTop = function () { try { var f = document.createElement('form'); f.method = 'post'; f.action = '/api/shutdown'; f.target = '_top'; document.body.appendChild(f); f.submit(); return 'submitted'; } catch (e) { return 'blocked'; } };
  attacks.directFetch = function () { return fetch('/api/state').then(function () { return 'ALLOWED'; }, function () { return 'blocked'; }); };
  attacks.directFetchAbs = function () { return fetch('http://127.0.0.1:' + location.port + '/api/state', { mode: 'no-cors' }).then(function () { return 'ALLOWED'; }, function () { return 'blocked'; }); };
  attacks.xhr = function () { return new Promise(function (r) { try { var x = new XMLHttpRequest(); x.open('GET', '/api/state'); x.onload = function () { r('ALLOWED'); }; x.onerror = function () { r('blocked'); }; x.send(); } catch (e) { r('blocked'); } }); };
  attacks.websocket = function () { return new Promise(function (r) { try { var w = new WebSocket('ws://127.0.0.1:' + 1 + '/'); w.onopen = function () { r('OPEN'); }; w.onerror = function () { r('blocked'); }; setTimeout(function () { r('blocked'); }, 1500); } catch (e) { r('blocked'); } }); };
  attacks.beaconImg = function () { return new Promise(function (r) { var i = new Image(); i.onload = function () { r('LOADED'); }; i.onerror = function () { r('blocked'); }; i.src = 'http://example.invalid/x.png'; setTimeout(function () { r('blocked'); }, 1500); }); };
  attacks.dynamicScript = function () { return new Promise(function (r) { var s = document.createElement('script'); s.src = 'http://127.0.0.1:1/x.js'; s.onload = function () { r('LOADED'); }; s.onerror = function () { r('blocked'); }; document.head.appendChild(s); setTimeout(function () { r('blocked'); }, 1500); }); };
  attacks.inlineEval = function () { try { return 'eval=' + eval('1+1'); } catch (e) { return 'blocked'; } };
  attacks.nestedFrame = function () { return new Promise(function (r) { var f = document.createElement('iframe'); f.src = '/'; f.onload = function () { try { r('LOADED, doc=' + !!f.contentDocument); } catch (e) { r('loaded-opaque'); } }; document.body.appendChild(f); setTimeout(function () { r('timeout'); }, 1500); }); };
  attacks.serviceWorker = function () { try { return navigator.serviceWorker ? 'sw-present' : 'no-sw'; } catch (e) { return 'blocked'; } };
  attacks.blobWorker = function () { try { var w = new Worker(URL.createObjectURL(new Blob(['postMessage(1)']))); return 'worker-made'; } catch (e) { return 'blocked'; } };

  // ---- ask the bridge for things it must refuse ----
  attacks.cmdUngranted = function () { return Promise.all(['Quit', 'Shutdown', 'GoBlackout', { DeleteItem: { index: 0 } }, 'NextSlide2', { NextSlide: 1 }].map(function (c) { return call('sendCommand', { command: c }); })).then(function (r) { return r.join(' | '); }); };
  attacks.cmdGranted = function () { return call('sendCommand', { command: 'NextSlide' }); };
  attacks.cmdBatch = function () { return call('sendCommand', { command: ['NextSlide', 'Quit'] }); };
  attacks.keyringOwn = function () { return call('keyring.set', { account: 'a', secret: 's' }).then(function (a) { return call('keyring.get', { account: 'a' }).then(function (b) { return a + ' / ' + b; }); }); };
  attacks.keyringOthers = function () {
    var svc = ['OpenSanctuary:System', 'opensanctuary:system', 'OpenSanctuary:System:x', 'OpenSanctuary:Shared:anything', 'OpenSanctuary:Plugin:genius_lyrics', 'OpenSanctuary:Plugin:../x', '', 'x\u0000y'];
    return Promise.all(svc.map(function (s) { return call('keyring.get', { service: s, account: 'a' }); })).then(function (r) { return r.join(' | '); });
  };
  attacks.keyringAccounts = function () { return Promise.all(['', '../x', 'a'.repeat(5000), 'a\nb', null, 7, { x: 1 }].map(function (a) { return call('keyring.get', { account: a }); })).then(function (r) { return r.join(' | '); }); };
  attacks.fetchUrls = function () {
    var urls = ['https://127.0.0.1:' + location.port + '/api/state', 'https://127.0.0.1:1/', 'https://evil.example/', 'https://127.0.0.1@evil.example/', 'https://evil.example#@127.0.0.1/', 'https://evil.example\\@127.0.0.1/', 'https://[::1]/', 'https://localhost/', 'https://2130706433/', 'https://0x7f.1/', 'https://127.1/', 'https://127.0.0.1.evil.example/', 'https://127.0.0.1:99999/', 'https://127.0.0.1:443@127.0.0.1/', 'https:///evil.example', 'file:///etc/passwd', 'ftp://127.0.0.1/', 'javascript:alert(1)', 'data:text/html,hi', ''];
    return Promise.all(urls.map(function (u) { return call('net.fetch', { url: u }).then(function (r) { return u.slice(0, 40) + ' => ' + r; }); })).then(function (r) { return r.join('\n'); });
  };
  attacks.fetchHeaders = function () { return call('net.fetch', { url: 'http://127.0.0.1:' + location.port + '/api/state', headers: { Cookie: 'x', 'X-Plugin-Token': 'x', Host: 'evil', 'x-os-console': '1', 'Authorization': 'Bearer ' + 'a'.repeat(100000) } }); };
  attacks.ungrantedMethods = function () { return Promise.all([['library.addSong', { song: { title: 'x' } }], ['library.addScriptures', { items: [] }], ['library.addMediaUrl', { url: 'http://127.0.0.1/x.mp4' }], ['process.start', { command: 'x' }], ['tool.update', {}], ['ui.pasteText', {}], ['nope', {}], ['__proto__', {}], ['constructor', {}], ['toString', {}], ['', {}]].map(function (m) { return call(m[0], m[1]); })).then(function (r) { return r.join(' | '); }); };
  attacks.badParams = function () { return Promise.all([undefined, null, 7, 'str', [], { __proto__: { polluted: 1 } }, { command: { toString: 1 } }].map(function (p) { return call('sendCommand', p); })).then(function (r) { return r.join(' | '); }); };

  // ---- abuse what IS granted ----
  attacks.notifyFlood = function () { var ps = []; for (var i = 0; i < 300; i++) ps.push(call('notify', { message: 'spam ' + i, level: 'error' })); return Promise.all(ps).then(function (r) { return 'ok=' + r.filter(function (x) { return x.indexOf('OK') === 0; }).length + ' refused=' + r.filter(function (x) { return x.indexOf('ERR') === 0; }).length; }); };
  attacks.notifyHtml = function () { return Promise.all([{ message: '<img src=x onerror=window.__pwn=1>', level: 'info' }, { message: '<script>window.__pwn=1</script>', title: '<b onmouseover=window.__pwn=1>t</b>', level: 'info' }, { message: 'x'.repeat(1e6), level: 'info' }, { message: 'a', level: '"><svg onload=window.__pwn=1>' }, { message: 'a', key: '"><img src=x onerror=window.__pwn=1>' }, { message: 'a', source: 'genius_lyrics' }, { message: 'a', sticky: true, level: 'error' }].map(function (n) { return call('notify', n); })).then(function (r) { return r.map(function (x) { return x.slice(0, 40); }).join(' | '); }); };
  attacks.toolbarAbuse = function () { return Promise.all([{ id: 'go', label: '<img src=x onerror=window.__pwn=1>' }, { id: 'nope', disabled: true }, { id: 'go', disabled: 'yes' }, { id: '__proto__', pressed: true }, { id: 'go', pressed: true, icon: '../../x' }].map(function (p) { return call('ui.toolbar.update', p); })).then(function (r) { return r.join(' | '); }); };
  attacks.resizeAbuse = function () { return Promise.all([{ height: 1e9 }, { height: -50 }, { height: NaN }, { height: 'big' }, { height: 0 }, { height: Infinity }, {}].map(function (p) { return call('ui.resize', p); })).then(function (r) { return r.join(' | '); }); };
  attacks.logFlood = function () { var ps = []; for (var i = 0; i < 2000; i++) ps.push(call('log', { message: 'x'.repeat(5000) })); return Promise.all(ps).then(function () { return 'sent'; }); };
  attacks.requestFlood = function () { var n = 0; for (var i = 0; i < 20000; i++) { parent.postMessage({ type: 'os.request', id: i, method: 'getState', params: {} }, '*'); n++; } return 'posted ' + n; };
  attacks.hugeMessage = function () { parent.postMessage({ type: 'os.request', id: 1, method: 'log', params: { message: 'x'.repeat(60 * 1024 * 1024) } }, '*'); return 'posted 60MB'; };
  attacks.deepMessage = function () { var o = {}; var c = o; for (var i = 0; i < 100000; i++) { c.a = {}; c = c.a; } try { parent.postMessage({ type: 'os.request', id: 2, method: 'log', params: o }, '*'); return 'posted deep'; } catch (e) { return 'clone-failed'; } };
  attacks.forgeEvents = function () { try { window.postMessage({ type: 'os.event', event: 'toolbar', data: { id: 'go' } }, '*'); parent.postMessage({ type: 'os.response', id: 1, ok: true, result: {} }, '*'); parent.postMessage({ type: 'os.event', event: 'state', data: {} }, '*'); parent.postMessage({ type: 'os.request', id: {}, method: 'getState' }, '*'); parent.postMessage('string', '*'); parent.postMessage(null, '*'); parent.postMessage([], '*'); return 'sent'; } catch (e) { return 'ERR ' + e.message; } };
  attacks.impersonate = function () { parent.postMessage({ type: 'os.request', id: 3, method: 'notify', params: { message: 'from system', source: 'OpenSanctuary', pluginId: 'genius_lyrics', plugin: 'genius_lyrics' } }, '*'); return 'sent'; };

  // ---- take the console down ----
  attacks.domBomb = function () { for (var i = 0; i < 300000; i++) { var d = document.createElement('div'); d.textContent = 'x'; document.body.appendChild(d); } return 'dom ' + document.body.childElementCount; };
  attacks.memBomb = function () { var a = []; try { for (var i = 0; i < 400; i++) a.push(new Array(1e6).fill(i)); } catch (e) { return 'oom ' + a.length; } window.__keep = a; return 'alloc ' + a.length; };
  attacks.spin = function () { setTimeout(function () { for (;;) {} }, 50); return 'spinning'; };
  attacks.spinTimers = function () { for (var i = 0; i < 50; i++) setInterval(function () { var t = Date.now(); while (Date.now() - t < 80) {} }, 1); return 'timers'; };
  attacks.crashTab = function () { try { var s = 'x'; for (;;) s += s; } catch (e) { return 'caught ' + e.name; } };
  attacks.resizeLoop = function () { var n = 0; var t = setInterval(function () { OS.call('ui.resize', { height: 100 + (n++ % 2) * 400 }).catch(function () {}); if (n > 3000) clearInterval(t); }, 1); return 'looping'; };

  window.__hostile = { run: function (name) { return Promise.resolve().then(function () { return attacks[name](); }).then(function (r) { return String(r); }, function (e) { return 'THREW ' + (e && e.message); }); }, names: Object.keys(attacks) };
  OS.on('toolbar', function () { window.__toolbarClicks = (window.__toolbarClicks || 0) + 1; });
  OS.on('state', function () {});
})();
`;
