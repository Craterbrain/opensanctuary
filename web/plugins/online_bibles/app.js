// Online Bibles: a sandboxed OpenSanctuary plugin. It has no network or console access of its own:
// the host fetches pages for it (net.fetch, only the hosts it was granted), stores chapters
// (library.addScriptures) and runs the schedule command it may have been granted. See
// docs/PLUGIN_PERMISSIONS.md.
(function () {
  'use strict';

  var call = OS.call;

  var $ = function (id) { return document.getElementById(id); };
  var statusEl = $('status');
  var progressWrap = $('progress-wrap');
  var progressBar = $('progress-bar');
  var busy = false;

  function setStatus(text, kind) {
    statusEl.textContent = text;
    statusEl.className = 'status' + (kind ? ' ' + kind : '');
  }
  function setProgress(done, total) {
    if (total == null) { progressWrap.classList.add('hidden'); return; }
    progressWrap.classList.remove('hidden');
    progressBar.style.width = Math.max(0, Math.min(100, (done / total) * 100)) + '%';
  }

  async function fetchText(url) {
    var res = await call('net.fetch', { url: url });
    if (res.status < 200 || res.status >= 300) throw new Error('the server answered ' + res.status);
    return res.body;
  }

  // ---- tabs ---------------------------------------------------------------------------------------
  var tabs = ['catalog', 'github', 'passage'];
  function showTab(name) {
    tabs.forEach(function (t) {
      $('tab-' + t).classList.toggle('active', t === name);
      $('view-' + t).classList.toggle('hidden', t !== name);
    });
  }
  tabs.forEach(function (t) { $('tab-' + t).addEventListener('click', function () { showTab(t); }); });

  // ---- catalog ------------------------------------------------------------------------------------
  var catalog = [];

  function renderEntries(listEl, entries, limit) {
    listEl.textContent = '';
    entries.slice(0, limit || 200).forEach(function (e) {
      var li = document.createElement('li');
      var meta = document.createElement('div');
      meta.className = 'meta';
      var name = document.createElement('div');
      name.className = 'title';
      var abbr = document.createElement('span');
      abbr.className = 'abbr';
      abbr.textContent = e.abbreviation;
      name.appendChild(abbr);
      name.appendChild(document.createTextNode(e.name));
      var sub = document.createElement('div');
      sub.className = 'sub';
      sub.textContent = e.language + ' \u00b7 ' + e.source;
      meta.appendChild(name);
      meta.appendChild(sub);
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'btn btn-sm btn-primary';
      btn.textContent = 'Install / Download';
      btn.addEventListener('click', function () { downloadBible(e, btn); });
      li.appendChild(meta);
      li.appendChild(btn);
      listEl.appendChild(li);
    });
    if (entries.length > (limit || 200)) {
      var more = document.createElement('li');
      more.className = 'hint';
      more.textContent = 'Showing the first ' + (limit || 200) + ' of ' + entries.length + ' \u2014 search to narrow it down.';
      listEl.appendChild(more);
    }
  }

  function filterCatalog() {
    var q = $('catalog-q').value.trim().toLowerCase();
    var lang = $('catalog-lang').value;
    var shown = catalog.filter(function (e) {
      if (lang !== 'all' && e.language.toLowerCase() !== lang) return false;
      return !q || e.name.toLowerCase().indexOf(q) !== -1 || e.abbreviation.toLowerCase().indexOf(q) !== -1;
    });
    renderEntries($('catalog-list'), shown);
    $('catalog-count').textContent = 'Showing ' + shown.length + ' of ' + catalog.length + ' translations';
    setStatus('');
  }

  async function loadCatalog() {
    setStatus('Loading the Bible catalog…');
    var bolls = [];
    var churchApps = [];
    // Each source is optional: the curated list always works, and a source that is down just
    // contributes nothing.
    await Promise.all([
      fetchText(Bibles.BOLLS_LANGUAGES_URL).then(function (t) { bolls = JSON.parse(t); }).catch(function () {}),
      fetchText(Bibles.CHURCHAPPS_URL).then(function (t) { churchApps = JSON.parse(t); }).catch(function () {})
    ]);
    catalog = Bibles.buildCatalog(bolls, churchApps);
    var languages = {};
    catalog.forEach(function (e) { languages[e.language] = true; });
    var select = $('catalog-lang');
    Object.keys(languages).sort(function (a, b) { return a === 'English' ? -1 : b === 'English' ? 1 : a < b ? -1 : 1; }).forEach(function (l) {
      var o = document.createElement('option');
      o.value = l.toLowerCase();
      o.textContent = l;
      select.appendChild(o);
    });
    filterCatalog();
  }

  $('catalog-q').addEventListener('input', filterCatalog);
  $('catalog-lang').addEventListener('change', filterCatalog);

  // ---- downloads ----------------------------------------------------------------------------------
  async function storeChapters(chapters) {
    var batches = Bibles.chunk(chapters, 100);
    var stored = 0;
    for (var i = 0; i < batches.length; i++) {
      var res = await call('library.addScriptures', { items: batches[i] });
      stored += res.chapters;
      setProgress(stored, chapters.length);
      setStatus('Saving… ' + stored + ' of ' + chapters.length + ' chapters');
    }
    return stored;
  }

  async function downloadBible(entry, btn) {
    if (busy) { setStatus('Another download is already running.'); return; }
    busy = true;
    btn.disabled = true;
    try {
      var url, chapters;
      if (entry.provider === 'github') {
        if (!Bibles.isRawGithubUrl(entry.sourceKey)) throw new Error('not a GitHub raw-file address');
        url = entry.sourceKey;
      } else {
        url = Bibles.bollsUrl(entry.abbreviation);
      }
      setStatus('Downloading ' + entry.name + '… (a whole Bible can take a minute)');
      setProgress(0, 100);
      var body = await fetchText(url);
      chapters = entry.provider === 'github' ? Bibles.parseGithubBible(body, url) : Bibles.parseBollsTranslation(body, entry.abbreviation);
      if (!chapters.length) throw new Error('that file had no chapters in a format this plugin understands');
      await storeChapters(chapters);
      var verses = chapters.reduce(function (n, c) { return n + c.verses.length; }, 0);
      setStatus('Installed ' + entry.abbreviation + ': ' + chapters.length + ' chapters, ' + verses + ' verses.', 'ok');
    } catch (e) {
      setStatus('Download failed: ' + e.message, 'error');
    } finally {
      busy = false;
      btn.disabled = false;
      setProgress(0, null);
    }
  }

  // ---- GitHub repositories -------------------------------------------------------------------------
  async function browseGithub() {
    var repo = Bibles.parseRepo($('gh-repo').value);
    if (!repo) { setStatus('Enter a repository as owner/repo.'); return; }
    var path = $('gh-path').value.trim() || 'versions';
    setStatus('Reading ' + repo.owner + '/' + repo.repo + '…');
    $('gh-list').textContent = '';
    try {
      var langDirs = JSON.parse(await fetchText(Bibles.githubContentsUrl(repo, path)));
      var entries = [];
      for (var i = 0; i < langDirs.length; i++) {
        var dir = langDirs[i];
        if (!dir || dir.type !== 'dir') continue;
        try {
          var files = JSON.parse(await fetchText(Bibles.githubContentsUrl(repo, dir.path)));
          entries = entries.concat(Bibles.githubCatalogEntries(repo, dir.name, files));
        } catch (e) { /* skip a folder we can't read */ }
      }
      renderEntries($('gh-list'), entries);
      setStatus(entries.length ? entries.length + ' Bibles in ' + repo.owner + '/' + repo.repo : 'No Bible files found in that folder.');
    } catch (e) {
      setStatus('Could not read the repository: ' + e.message, 'error');
    }
  }
  $('gh-browse').addEventListener('click', browseGithub);

  // ---- passages -----------------------------------------------------------------------------------
  async function fetchPassage() {
    var query = $('passage-q').value.trim();
    var trans = $('passage-trans').value;
    if (!query) return;
    if (busy) { setStatus('Another download is already running.'); return; }
    busy = true;
    try {
      var items = [];
      var book = Bibles.exactBook(query);
      if (book) {
        // A whole book: its chapters, a few at a time.
        setStatus('Fetching all ' + book[1] + ' chapters of ' + book[0] + '…');
        var chapterNumbers = [];
        for (var c = 1; c <= book[1]; c++) chapterNumbers.push(c);
        var done = 0;
        var results = new Array(chapterNumbers.length);
        var next = 0;
        async function worker() {
          while (next < chapterNumbers.length) {
            var idx = next++;
            try {
              results[idx] = Bibles.parseBibleApi(await fetchText(Bibles.chapterUrl(book[0], chapterNumbers[idx], trans)), trans);
            } catch (e) { results[idx] = null; }
            setProgress(++done, chapterNumbers.length);
          }
        }
        await Promise.all([worker(), worker(), worker(), worker(), worker()]);
        items = results.filter(Boolean);
      } else {
        setStatus('Fetching ' + query + '…');
        var one = Bibles.parseBibleApi(await fetchText(Bibles.passageUrl(query, trans)), trans);
        if (one) items = [one];
      }
      if (!items.length) throw new Error('nothing found for “' + query + '”');
      var stored = await call('library.addScriptures', { items: items });
      var msg = 'Added ' + stored.chapters + ' ' + (stored.chapters === 1 ? 'passage' : 'chapters') + ' (' + items[0].version + ').';
      try {
        await call('sendCommand', { command: { AddToSchedule: { item_type: 'scripture', item_id: stored.ids[0] } } });
        msg += ' Added to the schedule.';
      } catch (e) {
        msg += ' (Not added to the schedule: ' + e.message + ')';
      }
      setStatus(msg, 'ok');
    } catch (e) {
      setStatus('Could not fetch that passage: ' + e.message, 'error');
    } finally {
      busy = false;
      setProgress(0, null);
    }
  }
  $('passage-go').addEventListener('click', fetchPassage);
  $('passage-q').addEventListener('keydown', function (ev) { if (ev.key === 'Enter') fetchPassage(); });

  if (OS.slot === 'dock') call('ui.resize', { height: 460 }).catch(function () {});
  loadCatalog();
})();
