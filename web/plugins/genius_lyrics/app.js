// Song Lyrics: a sandboxed OpenSanctuary plugin. It has no network or console access of its own;
// the host fetches for it (net.fetch, only lrclib.net and genius.com), stores the song (library.addSong) and
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
      sub.textContent = [hit.artist, hit.album, hit.length].filter(Boolean).join(' \u00b7 ');
      meta.appendChild(title);
      meta.appendChild(sub);
      var badge = document.createElement('span');
      badge.className = 'os-badge os-badge--accent';
      badge.textContent = hit.source === 'lrclib' ? 'LRCLIB' : 'Genius';
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

  async function fetchHits(label, url, parse, query) {
    var res = await OS.call('net.fetch', { url: url });
    if (res.status !== 200) throw new Error(label + ' answered ' + res.status);
    return parse(res.body, query);
  }

  async function search(query) {
    var q = String(query).trim();
    if (!q) return;
    setStatus('\u2728 Searching LRCLIB and Genius for \u201c' + q + '\u201d\u2026');
    listEl.textContent = '';
    // LRCLIB is an open lyrics database; Genius sits behind a bot filter that often refuses us, so it is
    // a bonus: its failure never hides LRCLIB's results.
    var results = await Promise.all([
      fetchHits('LRCLIB', Lyrics.lrclibSearchUrl(q), Lyrics.parseLrclibResponse, q).catch(function (e) { return e; }),
      fetchHits('Genius', Lyrics.searchUrl(q), Lyrics.parseSearchResponse, q).catch(function (e) { return e; })
    ]);
    var hits = [], problems = [];
    results.forEach(function (r) { if (r instanceof Error) problems.push(r.message); else hits = hits.concat(r); });
    renderHits(hits);
    if (hits.length) {
      setStatus(hits.length + ' result' + (hits.length === 1 ? '' : 's') + (problems.length ? ' (' + problems.join('; ') + ')' : ''));
    } else if (problems.length === results.length) {
      setStatus('Search failed: ' + problems.join('; '), 'error');
    } else {
      setStatus('No song matching \u201c' + q + '\u201d' + (problems.length ? ' (' + problems.join('; ') + ')' : '') + '. Try the title and artist.');
    }
  }

  async function importHit(hit, goLive) {
    if (busy) return;
    busy = true;
    setStatus('Importing “' + hit.title + '”…');
    try {
      var song;
      if (hit.source === 'lrclib') {
        song = Lyrics.plainLyricsToSong(hit.title, hit.artist, hit.album, hit.plain);
      } else {
        if (!Lyrics.isGeniusUrl(hit.url)) throw new Error('not a Genius page');
        var page = await OS.call('net.fetch', { url: hit.url });
        if (page.status !== 200) throw new Error('Genius answered ' + page.status);
        song = Lyrics.pageToSong(page.body, hit.title, hit.artist);
      }
      if (!song.slides.length) throw new Error('no lyrics found');
      var added = await OS.call('library.addSong', { song: song });
      var msg = 'Imported “' + song.title + '” with ' + song.slides.length + ' slides' +
        (song.ccli_number ? ' (CCLI #' + song.ccli_number + ')' : '') + '.';
      try {
        var res = await OS.call('sendCommand', { command: { AddToSchedule: { item_type: 'song', item_id: added.id } } });
        var ev = ((res && res.events) || []).find(function (e) { return e.event && e.event.event_type === 'ScheduleItemAdded'; });
        if (!res || !res.success || !ev) throw new Error((res && res.error) || 'the console refused it');
        if (goLive) {
          // Go live on the item just added, by index: `item_index: null` means "whatever is
          // staged/selected", which is the previous item, not this one.
          await OS.call('sendCommand', { command: { GoLive: { item_index: ev.event.data.index, slide_index: 0 } } });
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
