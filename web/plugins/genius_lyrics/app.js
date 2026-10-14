// Genius Lyrics: a sandboxed OpenSanctuary plugin. It has no network or console access of its own;
// the host fetches pages for it (net.fetch, only genius.com), stores the song (library.addSong) and
// runs the schedule commands it was granted. See docs/PLUGIN_PERMISSIONS.md.
(function () {
  'use strict';

  var statusEl = document.getElementById('status');
  var listEl = document.getElementById('hits');
  var inputEl = document.getElementById('q');
  var busy = false;

  function setStatus(text, kind) {
    statusEl.textContent = text;
    statusEl.className = 'status' + (kind ? ' ' + kind : '');
  }

  function renderHits(hits) {
    listEl.textContent = '';
    hits.forEach(function (hit) {
      var li = document.createElement('li');
      var thumb = document.createElement('div');
      thumb.className = 'thumb';
      thumb.textContent = '✨';
      var meta = document.createElement('div');
      meta.className = 'meta';
      var title = document.createElement('div');
      title.className = 'title';
      title.textContent = hit.title;
      var sub = document.createElement('div');
      sub.className = 'sub';
      sub.textContent = hit.artist;
      meta.appendChild(title);
      meta.appendChild(sub);
      var badge = document.createElement('span');
      badge.className = 'os-badge os-badge--accent';
      badge.textContent = 'Genius';
      var actions = document.createElement('div');
      actions.className = 'actions';
      var add = document.createElement('button');
      add.type = 'button';
      add.className = 'btn btn-sm btn-primary';
      add.textContent = '📥 Import & stage';
      add.addEventListener('click', function () { importHit(hit, false); });
      var live = document.createElement('button');
      live.type = 'button';
      live.className = 'btn btn-sm';
      live.textContent = 'Import & go live';
      live.addEventListener('click', function () { importHit(hit, true); });
      actions.appendChild(add);
      actions.appendChild(live);
      li.addEventListener('dblclick', function () { importHit(hit, false); });
      li.appendChild(thumb);
      li.appendChild(meta);
      li.appendChild(badge);
      li.appendChild(actions);
      listEl.appendChild(li);
    });
  }

  async function search(query) {
    var url = Genius.searchUrl(query);
    if (!url) return;
    setStatus('✨ Searching Genius for Christian lyrics matching “' + query.trim() + '”…');
    listEl.textContent = '';
    try {
      var res = await OS.call('net.fetch', { url: url });
      if (res.status !== 200) throw new Error('Genius answered ' + res.status);
      var hits = Genius.parseSearchResponse(res.body, query);
      renderHits(hits);
      setStatus(hits.length ? hits.length + ' result' + (hits.length === 1 ? '' : 's') : 'No Christian lyrics found for “' + query.trim() + '”. Try another title or artist.');
    } catch (e) {
      setStatus('Search failed: ' + e.message, 'error');
    }
  }

  async function importHit(hit, goLive) {
    if (busy) return;
    busy = true;
    setStatus('Importing “' + hit.title + '”…');
    try {
      if (!Genius.isGeniusUrl(hit.url)) throw new Error('not a Genius page');
      var page = await OS.call('net.fetch', { url: hit.url });
      if (page.status !== 200) throw new Error('Genius answered ' + page.status);
      var song = Genius.pageToSong(page.body, hit.title, hit.artist);
      if (!song.slides.length) throw new Error('no lyrics found on that page');
      var added = await OS.call('library.addSong', { song: song });
      var msg = 'Imported “' + song.title + '” with ' + song.slides.length + ' slides' +
        (song.ccli_number ? ' (CCLI #' + song.ccli_number + ')' : '') + '.';
      try {
        await OS.call('sendCommand', { command: { AddToSchedule: { item_type: 'song', item_id: added.id } } });
        if (goLive) {
          await OS.call('sendCommand', { command: { GoLive: { item_index: null, slide_index: 0 } } });
        }
      } catch (e) {
        msg += ' (Not added to the schedule: ' + e.message + ')';
      }
      setStatus(msg, 'ok');
    } catch (e) {
      setStatus('Import failed: ' + e.message, 'error');
    } finally {
      busy = false;
    }
  }

  document.getElementById('search-btn').addEventListener('click', function () { search(inputEl.value); });
  inputEl.addEventListener('keydown', function (ev) {
    if (ev.key === 'Enter') search(inputEl.value);
  });
  if (OS.slot === 'dock') OS.call('ui.resize', { height: 380 }).catch(function () {});
  inputEl.focus();
})();
