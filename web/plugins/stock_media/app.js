// Stock Images: a sandboxed OpenSanctuary plugin. It has no network access of its own: the host
// searches Pexels and Pixabay for it (net.fetch, only those hosts), downloads the picture you pick
// into the Media Library (library.addMediaUrl) and keeps the API keys (keyring.*). See
// docs/PLUGIN_PERMISSIONS.md.
(function () {
  'use strict';

  var PROVIDERS = {
    pexels: { name: 'Pexels', request: Stock.pexelsRequest, parse: Stock.parsePexels },
    pixabay: { name: 'Pixabay', request: Stock.pixabayRequest, parse: Stock.parsePixabay }
  };
  var statusEl = document.getElementById('status');
  var gridEl = document.getElementById('grid');
  var inputEl = document.getElementById('q');
  var keys = { pexels: '', pixabay: '' };
  var busy = false;

  function setStatus(text, kind) {
    statusEl.textContent = text;
    statusEl.className = 'status' + (kind ? ' ' + kind : '');
  }

  async function loadKeys() {
    for (var id in PROVIDERS) {
      try {
        var r = await OS.call('keyring.get', { account: id });
        keys[id] = (r && r.secret) || '';
      } catch (e) { keys[id] = ''; }
      var box = document.getElementById('use-' + id);
      box.disabled = !keys[id];
      box.checked = !!keys[id];
    }
    if (!keys.pexels && !keys.pixabay) {
      document.getElementById('keys').open = true;
      setStatus('Add a free API key from Pexels or Pixabay below to start searching.');
    }
  }

  async function saveKey(id) {
    var el = document.getElementById('key-' + id);
    var value = Stock.clean(el.value, 200);
    if (!value) return;
    try {
      await OS.call('keyring.set', { account: id, secret: value });
      el.value = '';
      await loadKeys();
      setStatus(PROVIDERS[id].name + ' key saved.', 'ok');
    } catch (e) {
      setStatus('Could not save the key: ' + e.message, 'error');
    }
  }

  async function removeKey(id) {
    try {
      await OS.call('keyring.delete', { account: id });
      await loadKeys();
      setStatus(PROVIDERS[id].name + ' key removed.', 'ok');
    } catch (e) {
      setStatus('Could not remove the key: ' + e.message, 'error');
    }
  }

  function card(hit) {
    var el = document.createElement('div');
    el.className = 'card';
    el.title = (hit.tags || hit.provider + ' photo') + '\n' + hit.width + ' × ' + hit.height + (hit.author ? ' · ' + hit.author : '');
    var img = document.createElement('img');
    img.className = 'photo';
    img.loading = 'lazy';
    img.alt = hit.tags || '';
    img.src = hit.thumb;
    var badge = document.createElement('span');
    badge.className = 'os-badge os-badge--accent badge';
    badge.textContent = hit.provider;
    var add = document.createElement('button');
    add.type = 'button';
    add.className = 'btn btn-sm btn-primary';
    add.textContent = '📥 Add';
    var cap = document.createElement('div');
    cap.className = 'cap';
    cap.textContent = hit.author ? 'by ' + hit.author : hit.provider;
    el.append(img, badge, add, cap);
    var go = function () { addHit(hit, el); };
    add.addEventListener('click', function (ev) { ev.stopPropagation(); go(); });
    el.addEventListener('click', go);
    return el;
  }

  async function searchOne(id, query) {
    var p = PROVIDERS[id];
    var req = p.request(query, keys[id]);
    if (!req) return { hits: [], note: null };
    var res = await OS.call('net.fetch', { url: req.url, headers: req.headers });
    if (res.status !== 200) throw new Error(Stock.explainStatus(p.name, res.status));
    return { hits: p.parse(res.body) };
  }

  async function search() {
    var query = Stock.clean(inputEl.value, 200);
    if (!query || busy) return;
    var ids = Object.keys(PROVIDERS).filter(function (id) { return keys[id] && document.getElementById('use-' + id).checked; });
    if (!ids.length) { setStatus('Turn on a source with an API key (see “API keys” below).', 'error'); return; }
    busy = true;
    gridEl.textContent = '';
    setStatus('Searching for “' + query + '”…');
    var lists = [], problems = [];
    await Promise.all(ids.map(async function (id) {
      try { lists.push({ id: id, hits: (await searchOne(id, query)).hits }); }
      catch (e) { problems.push(e.message); }
    }));
    lists.sort(function (a, b) { return ids.indexOf(a.id) - ids.indexOf(b.id); });
    var hits = Stock.interleave(lists.map(function (l) { return l.hits; }));
    hits.forEach(function (h) { gridEl.appendChild(card(h)); });
    if (hits.length) setStatus(hits.length + ' photo' + (hits.length === 1 ? '' : 's') + (problems.length ? ' (' + problems.join(' ') + ')' : '') + '. Click one to add it.', problems.length ? 'error' : '');
    else setStatus(problems.length ? problems.join(' ') : 'No photos found for “' + query + '”. Try another word.', problems.length ? 'error' : '');
    busy = false;
  }

  async function addHit(hit, el) {
    if (el.classList.contains('busy') || el.classList.contains('added')) return;
    el.classList.add('busy');
    setStatus('Adding the photo to your Media Library…');
    try {
      var added = await OS.call('library.addMediaUrl', { url: hit.full, title: Stock.libraryName(hit), id: hit.id });
      el.classList.add('added');
      setStatus('Added to your Media Library (' + hit.provider + (hit.author ? ', photo by ' + hit.author : '') + ').', 'ok');
      OS.notify('Photo added', Stock.libraryName(hit), { level: 'success' }).catch(function () {});
      return added;
    } catch (e) {
      setStatus('Could not add that photo: ' + e.message, 'error');
    } finally {
      el.classList.remove('busy');
    }
  }

  document.getElementById('search-btn').addEventListener('click', search);
  inputEl.addEventListener('keydown', function (ev) { if (ev.key === 'Enter') search(); });
  Object.keys(PROVIDERS).forEach(function (id) {
    document.getElementById('save-' + id).addEventListener('click', function () { saveKey(id); });
    document.getElementById('clear-' + id).addEventListener('click', function () { removeKey(id); });
  });
  loadKeys().then(function () { inputEl.focus(); });
})();
