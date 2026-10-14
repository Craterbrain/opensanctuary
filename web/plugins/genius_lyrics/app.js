// Genius Lyrics: a sandboxed OpenSanctuary plugin. It has no network or console access of its own;
// the host fetches pages for it (net.fetch, only genius.com), stores the song (library.addSong) and
// runs the schedule commands it was granted. See docs/PLUGIN_PERMISSIONS.md.
(function () {
  'use strict';

  var pending = {};
  var nextId = 0;

  function call(method, params) {
    return new Promise(function (resolve, reject) {
      var id = ++nextId;
      pending[id] = { resolve: resolve, reject: reject };
      parent.postMessage({ type: 'os.request', id: id, method: method, params: params }, '*');
    });
  }

  window.addEventListener('message', function (ev) {
    if (ev.source !== parent) return;
    var m = ev.data;
    if (!m || m.type !== 'os.response' || !pending[m.id]) return;
    var p = pending[m.id];
    delete pending[m.id];
    if (m.ok) p.resolve(m.result); else p.reject(new Error(m.error));
  });

  var statusEl = document.getElementById('status');
  var listEl = document.getElementById('hits');
  var inputEl = document.getElementById('q');

  function setStatus(text) { statusEl.textContent = text; }

  function renderHits(hits) {
    listEl.textContent = '';
    hits.forEach(function (hit) {
      var li = document.createElement('li');
      var title = document.createElement('div');
      title.className = 'title';
      title.textContent = hit.title;
      var artist = document.createElement('div');
      artist.className = 'artist';
      artist.textContent = hit.artist;
      var row = document.createElement('div');
      row.className = 'row';
      var add = document.createElement('button');
      add.type = 'button';
      add.textContent = 'Import & add to schedule';
      add.addEventListener('click', function () { importHit(hit, false); });
      var live = document.createElement('button');
      live.type = 'button';
      live.textContent = 'Import & go live';
      live.addEventListener('click', function () { importHit(hit, true); });
      row.appendChild(add);
      row.appendChild(live);
      li.appendChild(title);
      li.appendChild(artist);
      li.appendChild(row);
      listEl.appendChild(li);
    });
  }

  async function search(query) {
    var url = Genius.searchUrl(query);
    if (!url) return;
    setStatus('Searching Genius…');
    listEl.textContent = '';
    try {
      var res = await call('net.fetch', { url: url });
      if (res.status !== 200) throw new Error('Genius answered ' + res.status);
      var hits = Genius.parseSearchResponse(res.body, query);
      renderHits(hits);
      setStatus(hits.length ? hits.length + ' result' + (hits.length === 1 ? '' : 's') : 'No Christian lyrics found for “' + query + '”. Try another title or artist.');
    } catch (e) {
      setStatus('Search failed: ' + e.message);
    }
  }

  async function importHit(hit, goLive) {
    setStatus('Importing “' + hit.title + '”…');
    try {
      if (!Genius.isGeniusUrl(hit.url)) throw new Error('not a Genius page');
      var page = await call('net.fetch', { url: hit.url });
      if (page.status !== 200) throw new Error('Genius answered ' + page.status);
      var song = Genius.pageToSong(page.body, hit.title, hit.artist);
      if (!song.slides.length) throw new Error('no lyrics found on that page');
      var added = await call('library.addSong', { song: song });
      var msg = 'Imported “' + song.title + '” with ' + song.slides.length + ' slides' +
        (song.ccli_number ? ' (CCLI #' + song.ccli_number + ')' : '') + '.';
      try {
        await call('sendCommand', { command: { AddToSchedule: { item_type: 'song', item_id: added.id } } });
        if (goLive) {
          await call('sendCommand', { command: { GoLive: { item_index: null, slide_index: 0 } } });
        }
      } catch (e) {
        msg += ' (Not added to the schedule: ' + e.message + ')';
      }
      setStatus(msg);
    } catch (e) {
      setStatus('Import failed: ' + e.message);
    }
  }

  document.getElementById('search-btn').addEventListener('click', function () { search(inputEl.value); });
  inputEl.addEventListener('keydown', function (ev) {
    if (ev.key === 'Enter') search(inputEl.value);
  });

  call('ui.resize', { height: 360 }).catch(function () {});
})();
