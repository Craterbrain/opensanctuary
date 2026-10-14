// Parsing for yt-dlp's output. Pure functions (no DOM, no network, no process) so they can be tested
// on their own; app.js runs the download through the host and feeds the lines in here.
(function (root) {
  'use strict';

  /**
   * One line of yt-dlp output -> what it says about progress, or null if it says nothing.
   *   "[download]  45.2% of 12.34MiB at 3.45MiB/s ETA 00:02"  -> { percent, speed, eta, status }
   *   "[SponsorBlock] ..." / "[ExtractAudio]" / "[Merger]" / "[Fixup..." -> a status message
   */
  function parseProgressLine(line) {
    var text = String(line).trim();
    if (!text) return null;
    if (text.indexOf('[download]') !== -1) {
      var out = {};
      var pct = /(\d+(?:\.\d+)?)%/.exec(text);
      if (pct) out.percent = parseFloat(pct[1]);
      var speed = / at (\S+)/.exec(text);
      if (speed && speed[1] !== 'Unknown') out.speed = speed[1];
      var eta = / ETA (\S+)/.exec(text);
      if (eta && eta[1] !== 'Unknown') out.eta = eta[1];
      if (out.percent === undefined && !out.speed && !out.eta) return null;
      if (out.percent !== undefined) {
        out.status = out.percent >= 99.5 ? 'Merging video & audio format…' : 'Downloading: ' + out.percent.toFixed(1) + '%';
      }
      return out;
    }
    if (text.indexOf('[SponsorBlock]') !== -1) return { status: '✂️ SponsorBlock: Stripping sponsor segments…' };
    if (text.indexOf('[ExtractAudio]') !== -1 || text.indexOf('[Merger]') !== -1 || text.indexOf('[Fixup') !== -1) {
      return { status: 'Processing and finalizing media file…' };
    }
    return null;
  }

  /** A `--print-json` line -> { id, title, duration, thumbnail, filename, ext }, or null. */
  function parseInfoLine(line) {
    var t = String(line).trim();
    if (t.charAt(0) !== '{' || t.charAt(t.length - 1) !== '}') return null;
    var d;
    try { d = JSON.parse(t); } catch (e) { return null; }
    if (!d || typeof d !== 'object') return null;
    return {
      id: d.id != null ? String(d.id) : null,
      title: typeof d.title === 'string' ? d.title : null,
      duration: typeof d.duration === 'number' ? d.duration : null,
      thumbnail: typeof d.thumbnail === 'string' ? d.thumbnail : null,
      filename: typeof d._filename === 'string' ? d._filename : (typeof d.filename === 'string' ? d.filename : null),
      ext: typeof d.ext === 'string' ? d.ext : null
    };
  }

  var VIDEO = ['mp4', 'webm', 'mkv', 'mov', 'm4v', 'avi'];
  var AUDIO = ['mp3', 'm4a', 'opus', 'ogg', 'wav', 'flac', 'aac'];

  function extOf(name) {
    var m = /\.([A-Za-z0-9]+)$/.exec(name);
    return m ? m[1].toLowerCase() : '';
  }

  /**
   * Which of the job folder's files is the result. Prefer the one yt-dlp named (after any audio
   * conversion changed its extension), else the largest media file. Leftovers (.part, .ytdl, a
   * video-only intermediate such as name.f137.mp4) lose to the final file.
   */
  function chooseFile(files, info, audioOnly) {
    var allowed = audioOnly ? AUDIO : VIDEO;
    var media = (files || []).filter(function (f) { return allowed.indexOf(extOf(f.name)) !== -1; });
    if (!media.length) return null;
    if (info && info.filename) {
      var base = info.filename.split(/[\\/]/).pop();
      var stem = base.replace(/\.[^.]+$/, '');
      var same = media.filter(function (f) { return f.name.replace(/\.[^.]+$/, '') === stem; });
      if (same.length) media = same;
    }
    var finals = media.filter(function (f) { return !/\.f\d+\.[A-Za-z0-9]+$/.test(f.name); });
    if (finals.length) media = finals;
    media.sort(function (a, b) { return b.size - a.size; });
    return media[0];
  }

  var BROWSERS = ['chrome', 'firefox', 'brave', 'edge', 'chromium', 'opera', 'vivaldi', 'safari'];

  /** The browser for --cookies-from-browser: "auto" means Chrome, "none"/"" means don't. */
  function cookieBrowser(choice) {
    var c = String(choice || '').trim().toLowerCase();
    if (!c || c === 'none') return null;
    if (c === 'auto') return 'chrome';
    return BROWSERS.indexOf(c) !== -1 ? c : null;
  }

  var api = { parseProgressLine: parseProgressLine, parseInfoLine: parseInfoLine, chooseFile: chooseFile, cookieBrowser: cookieBrowser, BROWSERS: BROWSERS };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.YtDlp = api;
})(typeof self !== 'undefined' ? self : this);
