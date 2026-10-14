// Video Downloader: a sandboxed OpenSanctuary plugin. It never runs anything itself: the host runs the
// one command line this plugin declared in its manifest (shown at approval), with the parameters
// validated, inside a throwaway job folder; this page polls the job's output, then asks the host to move
// the finished file into the Media Library. See docs/PLUGIN_PERMISSIONS.md.
(function () {
  'use strict';

  var $ = function (id) { return document.getElementById(id); };
  var statusEl = $('status');
  var running = null; // { job, cancelled }
  var toolReady = false;

  function setStatus(text, kind) {
    statusEl.textContent = text;
    statusEl.className = 'status' + (kind ? ' ' + kind : '');
  }
  function setBusy(busy) {
    $('download').disabled = busy;
    $('download-live').disabled = busy;
    $('cancel').classList.toggle('hidden', !busy);
    if (!busy) $('progress-wrap').classList.add('hidden');
  }
  function setProgress(percent, speed, eta) {
    $('progress-wrap').classList.remove('hidden');
    $('progress-bar').style.width = Math.max(0, Math.min(100, percent)) + '%';
    $('speed').textContent = speed ? 'Speed: ' + speed : '';
    $('eta').textContent = eta ? 'ETA: ' + eta : '';
  }
  function sleep(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }

  // ---- the tool (yt-dlp) ---------------------------------------------------------------------------
  function describeTool(t) {
    if (t.updating) return 'Updating yt-dlp…';
    if (!t.installed) return 'yt-dlp is not installed yet' + (t.last_update && t.last_update.error ? ': ' + t.last_update.message : '.');
    return 'yt-dlp ' + (t.version || '') + ' (' + (t.source === 'managed' ? 'installed and kept current by OpenSanctuary' : 'found on this computer') + ')';
  }

  async function refreshTool() {
    var t = await OS.call('tool.status', { tool: 'yt-dlp' });
    toolReady = t.installed;
    $('tool-status').textContent = describeTool(t);
    return t;
  }

  async function updateTool() {
    $('tool-update').disabled = true;
    try {
      var t = await OS.call('tool.update', { tool: 'yt-dlp' });
      for (var i = 0; i < 600 && t.updating; i++) {
        $('tool-status').textContent = describeTool(t);
        await sleep(1000);
        t = await OS.call('tool.status', { tool: 'yt-dlp' });
      }
      toolReady = t.installed;
      var last = t.last_update;
      $('tool-status').textContent = describeTool(t) + (last && last.updated ? ' — updated.' : last && last.error ? ' — update failed: ' + last.message : '');
    } catch (e) {
      $('tool-status').textContent = 'Could not check for updates: ' + e.message;
    } finally {
      $('tool-update').disabled = false;
    }
  }

  // ---- a download ----------------------------------------------------------------------------------
  /** Runs the command once; resolves with { status, exit_code, files, info, lines }. */
  async function runOnce(params, onProgress) {
    var started = await OS.call('process.start', { command: 'download', params: params });
    running = { job: started.job, cancelled: false };
    var since = 0;
    var info = null;
    var tail = [];
    for (;;) {
      var p = await OS.call('process.poll', { job: started.job, since: since });
      since = p.next;
      p.lines.forEach(function (line) {
        var parsed = YtDlp.parseInfoLine(line);
        if (parsed) { info = parsed; return; }
        tail.push(line);
        if (tail.length > 8) tail.shift();
        var prog = YtDlp.parseProgressLine(line);
        if (prog) onProgress(prog);
      });
      if (p.status !== 'running') return { job: started.job, status: p.status, exit_code: p.exit_code, files: p.files, info: info, tail: tail };
      await sleep(700);
    }
  }

  async function download(goLive) {
    var url = $('url').value.trim();
    if (!url) { setStatus('Enter a video URL first.', 'error'); return; }
    if (!/^https?:\/\//i.test(url)) { setStatus('The URL must start with http:// or https://', 'error'); return; }
    var audioOnly = $('audio-only').checked;
    var base = { url: url, audio_only: audioOnly, sponsorblock: $('sponsorblock').checked };
    var browser = YtDlp.cookieBrowser($('browser').value);
    setBusy(true);
    setProgress(0);
    setStatus('Connecting to video source…');
    try {
      if (!toolReady) {
        setStatus('Installing yt-dlp first…');
        await updateTool();
        if (!toolReady) throw new Error('yt-dlp could not be installed');
      }
      var onProgress = function (p) {
        if (p.percent !== undefined) setProgress(p.percent, p.speed, p.eta);
        else if (p.speed || p.eta) setProgress(parseFloat($('progress-bar').style.width) || 0, p.speed, p.eta);
        if (p.status) setStatus(p.status);
      };
      var result = await runOnce(browser ? Object.assign({ browser: browser }, base) : base, onProgress);
      // Restricted videos need cookies, but a browser we can't read them from shouldn't block a public one.
      if (!running.cancelled && result.status === 'exited' && result.exit_code !== 0 && browser) {
        await OS.call('process.release', { job: result.job }).catch(function () {});
        setStatus('Retrying direct download without cookies…');
        result = await runOnce(base, onProgress);
      }
      if (running.cancelled || result.status === 'cancelled') {
        await OS.call('process.release', { job: result.job }).catch(function () {});
        setStatus('Cancelled.');
        return;
      }
      if (result.status !== 'exited' || result.exit_code !== 0) {
        var why = result.tail.filter(function (l) { return /error/i.test(l); }).pop() || result.tail.pop() || 'yt-dlp exited with an error';
        await OS.call('process.release', { job: result.job }).catch(function () {});
        throw new Error(why.replace(/^ERROR:\s*/i, ''));
      }
      var file = YtDlp.chooseFile(result.files, result.info, audioOnly);
      if (!file) throw new Error('yt-dlp finished but no ' + (audioOnly ? 'audio' : 'video') + ' file was produced');
      var title = (result.info && result.info.title) || file.name.replace(/\.[^.]+$/, '');
      setStatus('Adding to your Media Library…');
      var added = await OS.call('library.addMedia', {
        job: result.job, file: file.name, media_type: audioOnly ? 'audio' : 'video', title: title,
        id: 'ytdlp_' + ((result.info && result.info.id) || ''), duration: result.info && result.info.duration, thumbnail: result.info && result.info.thumbnail
      });
      await OS.call('process.release', { job: result.job }).catch(function () {});
      var msg = 'Completed: ' + title + '.';
      if (goLive) {
        try {
          await OS.call('sendCommand', { command: { AddToSchedule: { item_type: 'media', item_id: added.id } } });
          await OS.call('sendCommand', { command: { GoLive: { item_index: null, slide_index: 0 } } });
        } catch (e) {
          msg += ' (Could not go live: ' + e.message + ')';
        }
      }
      setProgress(100);
      setStatus(msg, 'ok');
      OS.notify('Download finished', title, { level: 'success' }).catch(function () {});
    } catch (e) {
      setStatus('Failed: ' + e.message, 'error');
      OS.notify('Download failed', e.message, { level: 'error', sticky: true }).catch(function () {});
    } finally {
      running = null;
      setBusy(false);
    }
  }

  $('download').addEventListener('click', function () { download(false); });
  $('download-live').addEventListener('click', function () { download(true); });
  $('cancel').addEventListener('click', function () {
    if (!running) return;
    running.cancelled = true;
    OS.call('process.cancel', { job: running.job }).catch(function () {});
    setStatus('Cancelling…');
  });
  $('paste').addEventListener('click', async function () {
    try {
      var r = await OS.call('ui.pasteText', {});
      if (r && r.text) { $('url').value = r.text.trim(); $('url').focus(); }
      else setStatus('Nothing to paste: copy a link first, or press Ctrl+V in the box.');
    } catch (e) {
      setStatus('Press Ctrl+V in the box to paste.');
      $('url').focus();
    }
  });
  $('url').addEventListener('keydown', function (ev) { if (ev.key === 'Enter') download(false); });
  $('tool-update').addEventListener('click', updateTool);

  if (OS.slot === 'dock') OS.call('ui.resize', { height: 480 }).catch(function () {});

  // Show what's installed, and look for a newer yt-dlp in the background.
  refreshTool().then(function (t) {
    if (!t.installed || t.source === 'managed') updateTool();
  }).catch(function (e) {
    $('tool-status').textContent = 'yt-dlp status unavailable: ' + e.message;
  });
})();
