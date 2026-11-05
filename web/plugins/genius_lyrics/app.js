// Song Lyrics: a sandboxed OpenSanctuary plugin. It has no network or console access of its own;
// the host fetches for it (net.fetch, only lrclib.net, genius.com and itunes.apple.com), stores the song (library.addSong) and
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
    hits.forEach(function (hit) { listEl.appendChild(hitRow(hit)); });
  }

  /** Christian songs first, then ones whose genre couldn't be told; the rest behind a "show" row
   *  (`onShowAll` runs when it's pressed). */
  function renderFiltered(hits, onShowAll) {
    var shown = hits.filter(function (h) { return h.faith === 'christian'; })
      .concat(hits.filter(function (h) { return h.faith !== 'christian' && h.faith !== 'other'; }));
    var others = hits.filter(function (h) { return h.faith === 'other'; });
    renderHits(shown);
    if (others.length) {
      var li = document.createElement('li');
      li.className = 'more-row';
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'btn btn-sm';
      btn.textContent = 'Show ' + others.length + ' other result' + (others.length === 1 ? '' : 's') + ' (not listed as Christian music)';
      btn.addEventListener('click', function () {
        li.remove();
        others.forEach(function (hit) { listEl.appendChild(hitRow(hit)); });
        if (onShowAll) onShowAll();
      });
      li.appendChild(btn);
      listEl.appendChild(li);
    }
    return { shown: shown.length, hidden: others.length };
  }

  function hitRow(hit) {
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
    sub.textContent = [hit.artist, hit.album, hit.length, hit.genre || (hit.faith === 'unknown' ? 'genre unknown' : '')]
      .filter(Boolean).join(' \u00b7 ');
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
    return li;
  }

  function pause(ms) { return new Promise(function (resolve) { setTimeout(resolve, ms); }); }

  /** net.fetch, tried again (twice, after a short pause) when the server says it's busy: LRCLIB often
   *  answers 503 "The server is busy, please retry in a moment" -- about 4 searches in 10 at times. */
  async function fetchPatiently(url, stillWanted) {
    var res;
    for (var attempt = 0; attempt < 3; attempt++) {
      if (attempt) await pause(attempt * 700);
      // A newer search has taken over during the pause: don't keep a busy server busier for this one.
      if (attempt && stillWanted && !stillWanted()) return res;
      res = await OS.call('net.fetch', { url: url });
      if (res.status !== 429 && res.status < 500) return res;
    }
    return res;
  }

  function sourceError(label, status) {
    return new Error(status === 429 || status === 503 ? label + ' is busy right now' : label + ' answered ' + status);
  }

  async function fetchHits(label, url, parse, query, stillWanted) {
    var res = stillWanted ? await fetchPatiently(url, stillWanted) : await OS.call('net.fetch', { url: url });
    if (res.status !== 200) throw sourceError(label, res.status);
    return parse(res.body, query);
  }

  // Artist lookups already made while this panel is open (genre, or '' for "iTunes doesn't know them"):
  // iTunes allows about 20 requests a minute, so a second search never spends one twice. No prototype,
  // so an artist called "constructor" is never mistaken for one already looked up.
  var artistGenreCache = Object.create(null);

  /** Genre records for the query (one iTunes song search) plus a lookup for each artist that leaves
   *  unexplained (up to 5 new ones). null when iTunes can't be reached: then nothing is filtered. */
  async function genreRecords(songsPromise, hits, stillWanted) {
    var records = await songsPromise;
    if (!records || !stillWanted()) return null;
    var pending = Lyrics.artistsToLookUp(hits, records, 5, artistGenreCache);
    await Promise.all(pending.map(async function (artist) {
      try {
        var res = await OS.call('net.fetch', { url: Lyrics.itunesArtistSearchUrl(artist) });
        if (res.status === 200) artistGenreCache[artist] = Lyrics.parseItunesArtistGenre(res.body, artist) || '';
      } catch (e) { /* unknown stays unknown, and is asked again next time */ }
    }));
    // The cache itself: an artist iTunes doesn't know is '' there, which counts as unknown.
    return { records: records, artistGenres: artistGenreCache };
  }

  // Bumped by each search; an older one still in flight never paints over a newer one's results.
  var searchSeq = 0;

  async function search(query) {
    var q = String(query).trim();
    if (!q) return;
    var mine = ++searchSeq;
    setStatus('\u2728 Searching LRCLIB and Genius for \u201c' + q + '\u201d\u2026');
    listEl.textContent = '';
    var stillWanted = function () { return mine === searchSeq; };
    // Genres come from iTunes, its song search sent alongside the lyrics searches (so sorting doesn't
    // wait for them); the artist lookups after it only if there are results.
    var songsPromise = OS.call('net.fetch', { url: Lyrics.itunesSongSearchUrl(q) })
      .then(function (res) { return res.status === 200 ? Lyrics.parseItunesSongs(res.body) : null; })
      .catch(function () { return null; });
    // LRCLIB is an open lyrics database; Genius sits behind a bot filter that often refuses us, so it is
    // a bonus: its failure never hides LRCLIB's results.
    var results = await Promise.all([
      fetchHits('LRCLIB', Lyrics.lrclibSearchUrl(q), Lyrics.parseLrclibResponse, q, stillWanted).catch(function (e) { return e; }),
      fetchHits('Genius', Lyrics.searchUrl(q), Lyrics.parseSearchResponse, q).catch(function (e) { return e; })
    ]);
    if (mine !== searchSeq) return;
    var hits = [], problems = [];
    results.forEach(function (r) { if (r instanceof Error) problems.push(r.message); else hits = hits.concat(r); });
    var note = problems.length ? ' (' + problems.join('; ') + ')' : '';
    if (!hits.length) {
      if (problems.length === results.length) setStatus('Search failed: ' + problems.join('; ') + '. Try again in a moment.', 'error');
      else setStatus('No song matching \u201c' + q + '\u201d' + note + '. Check the spelling, or try the title and artist.');
      return;
    }
    setStatus('\u2728 Checking which of ' + hits.length + ' result' + (hits.length === 1 ? ' is' : 's are') + ' Christian music\u2026');
    var unfiltered = function (why) {
      // Whatever a sort that failed half-way marked is dropped: nothing is filtered, nothing is labelled.
      hits.forEach(function (h) { delete h.faith; delete h.genre; });
      renderHits(hits);
      setStatus(hits.length + ' result' + (hits.length === 1 ? '' : 's') + note + ' \u2014 ' + why + ', so nothing is filtered.');
    };
    // Any failure in sorting still shows every result: the lyrics were found.
    var counts;
    try {
      var genres = await genreRecords(songsPromise, hits, stillWanted);
      if (mine !== searchSeq) return;
      if (!genres) return unfiltered('couldn\u2019t check genres');
      Lyrics.classifyHits(hits, genres.records, genres.artistGenres);
      counts = renderFiltered(hits, function () {
        setStatus(hits.length + ' result' + (hits.length === 1 ? '' : 's') + note + ', ' + counts.hidden + ' not listed as Christian music');
      });
    } catch (e) {
      if (mine !== searchSeq) return;
      return unfiltered('genres couldn\u2019t be sorted (' + e.message + ')');
    }
    if (counts.shown) {
      setStatus(counts.shown + ' result' + (counts.shown === 1 ? '' : 's') + note +
        (counts.hidden ? ' \u00b7 ' + counts.hidden + ' other' + (counts.hidden === 1 ? '' : 's') + ' hidden' : ''));
    } else {
      setStatus('Found ' + counts.hidden + ' song' + (counts.hidden === 1 ? '' : 's') + ' called that, none listed as Christian music' + note + '.');
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
