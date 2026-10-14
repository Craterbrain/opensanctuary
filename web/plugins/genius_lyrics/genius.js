// Parsing for Genius search results and lyrics pages. Pure functions (no DOM, no network) so they
// can be tested on their own; the page code in app.js does the fetching through the host.
(function (root) {
  'use strict';

  var CHRISTIAN_KEYWORDS = [
    'worship', 'christian', 'gospel', 'praise', 'hymn', 'church', 'jesus', 'god', 'lord', 'holy',
    'faith', 'grace', 'savior', 'christ', 'cross', 'hallelu'
  ];

  function hasChristianKeyword(text) {
    var t = String(text).toLowerCase();
    return CHRISTIAN_KEYWORDS.some(function (k) { return t.indexOf(k) !== -1; });
  }

  /** The search term sent to Genius: bias toward worship music unless the user already did. */
  function searchTerms(query) {
    var q = String(query).trim();
    if (!q) return '';
    return hasChristianKeyword(q) ? q : q + ' christian worship';
  }

  function searchUrl(query) {
    var terms = searchTerms(query);
    return terms ? 'https://genius.com/api/search/multi?q=' + encodeURIComponent(terms) : '';
  }

  /** Genius multi-search JSON (text) -> [{ id, title, artist, url, thumbnail }]. */
  function parseSearchResponse(jsonText, query) {
    var data;
    try { data = JSON.parse(jsonText); } catch (e) { return []; }
    var sections = (data && data.response && data.response.sections) || [];
    var alreadyChristian = hasChristianKeyword(query);
    var hits = [];
    var seen = {};
    sections.forEach(function (section) {
      if (!section || (section.type !== 'song' && section.type !== 'top_hit')) return;
      (section.hits || []).forEach(function (hit) {
        var res = hit && hit.result;
        if (!res) return;
        var url = res.url || '';
        if (url.indexOf('lyrics') === -1 && url.indexOf('annotated') === -1) return;
        if (seen[url]) return;
        var title = res.title || 'Unknown Title';
        var artist = res.artist_names || 'Christian Artist';
        if (title.indexOf('Release Calendar') !== -1 || title.indexOf('Tracklist') !== -1) return;
        var lowerUrl = url.toLowerCase();
        var relevant = alreadyChristian || hasChristianKeyword(title + ' ' + artist) ||
          lowerUrl.indexOf('worship') !== -1 || lowerUrl.indexOf('christian') !== -1;
        if (relevant || hits.length < 5) {
          seen[url] = true;
          hits.push({
            id: res.id != null ? 'genius_' + res.id : 'genius_' + Math.random().toString(16).slice(2),
            title: title,
            artist: artist,
            url: url,
            thumbnail: res.song_art_image_thumbnail_url || res.header_image_thumbnail_url || null
          });
        }
      });
    });
    return hits;
  }

  function isGeniusUrl(url) {
    var m = /^https:\/\/([^\/?#]*)/.exec(String(url).trim());
    return !!m && (m[1] === 'genius.com' || m[1] === 'www.genius.com');
  }

  /** CCLI song number mentioned in free text ("CCLI #1234567"), 4-10 digits. */
  function extractCcli(text) {
    var lower = String(text).toLowerCase();
    var from = 0;
    for (;;) {
      var idx = lower.indexOf('ccli', from);
      if (idx === -1) return null;
      var rest = lower.slice(idx + 4);
      var digits = '';
      var skipped = 0;
      for (var i = 0; i < rest.length; i++) {
        var c = rest.charAt(i);
        if (c >= '0' && c <= '9') {
          while (i < rest.length && rest.charAt(i) >= '0' && rest.charAt(i) <= '9') { digits += rest.charAt(i); i++; }
          break;
        }
        skipped++;
        if (skipped > 25) break;
      }
      if (digits.length >= 4 && digits.length <= 10) return digits;
      from = idx + 4;
    }
  }

  function stripHtml(html) {
    var withBreaks = String(html)
      .split('<br>').join('\n').split('<br/>').join('\n').split('<br />').join('\n')
      .split('</p>').join('\n\n').split('</div>').join('\n');
    var out = '';
    var inTag = false;
    for (var i = 0; i < withBreaks.length; i++) {
      var ch = withBreaks.charAt(i);
      if (ch === '<') inTag = true;
      else if (ch === '>') inTag = false;
      else if (!inTag) out += ch;
    }
    return out
      .split('&quot;').join('"').split('&apos;').join("'").split('&#39;').join("'")
      .split('&amp;').join('&').split('&lt;').join('<').split('&gt;').join('>')
      .split('&#x27;').join("'").split('&nbsp;').join(' ');
  }

  /** Index (in `s`, which starts just after an opening <div ...>) of the </div> that closes it,
   *  counting nested divs, or -1 if the markup is unbalanced. */
  function matchingDivEnd(s) {
    var re = /<(\/?)div\b[^>]*>/gi;
    var depth = 1;
    var m;
    while ((m = re.exec(s)) !== null) {
      depth += m[1] ? -1 : 1;
      if (depth === 0) return m.index;
    }
    return -1;
  }

  /** The lyrics text of a Genius song page. A page's lyrics are split over several
   *  data-lyrics-container divs, and each may hold nested divs (ads, headers), so each one is
   *  cut at its *matching* close tag rather than the first </div>. */
  function extractLyrics(html) {
    var marker = 'data-lyrics-container="true"';
    var chunks = [];
    var rest = String(html);
    for (;;) {
      var pos = rest.indexOf(marker);
      if (pos === -1) break;
      var after = rest.slice(pos + marker.length);
      var gt = after.indexOf('>');
      if (gt === -1) break;
      var inside = after.slice(gt + 1);
      var close = matchingDivEnd(inside);
      if (close === -1) break;
      chunks.push(inside.slice(0, close));
      rest = inside.slice(close + 6);
    }
    var raw = chunks.join('\n');
    if (!raw) {
      var legacy = String(html).indexOf('class="lyrics"');
      if (legacy !== -1) {
        var after2 = String(html).slice(legacy);
        var gt2 = after2.indexOf('>');
        if (gt2 !== -1) {
          var inside2 = after2.slice(gt2 + 1);
          var close2 = matchingDivEnd(inside2);
          if (close2 !== -1) raw = inside2.slice(0, close2);
        }
      }
    }
    return stripHtml(raw);
  }

  /** Page furniture that gets mixed into the lyrics text: "12 Contributors", "Translations", "Embed". */
  function isFurniture(trimmed) {
    return /^\d+\s*Contributors?/.test(trimmed) || trimmed.indexOf('Contributors') !== -1 ||
      trimmed.indexOf('Translations') !== -1 || trimmed.indexOf('Embed') !== -1;
  }

  /** "[Verse 1]" / "[Chorus]" ... sectioned lyrics -> a Song (the shape the library stores). */
  function lyricsToSong(title, artist, rawLyrics) {
    var song = {
      title: title, author: artist, copyright: 'Genius Christian Lyrics',
      ccli_number: null, slides: [], theme_name: null, alternate_title: null
    };
    var label = 'Verse 1';
    var tag = 'V1';
    var current = [];
    var verse = 1, chorus = 1, bridge = 1;

    function addSlide(lbl, tg, text) {
      song.slides.push({ text: text, header: tg, label: lbl, tag: lbl });
    }
    function flush() {
      var text = current.join('\n').trim();
      if (text) addSlide(label, tag, text);
      current = [];
    }

    String(rawLyrics).split(/\r?\n/).forEach(function (line) {
      var trimmed = line.trim();
      if (trimmed.charAt(0) === '[' && trimmed.charAt(trimmed.length - 1) === ']') {
        flush();
        var header = trimmed.slice(1, -1);
        var h = header.toLowerCase();
        if (h.indexOf('chorus') !== -1) { label = 'Chorus ' + chorus; tag = 'C' + chorus; chorus++; }
        else if (h.indexOf('bridge') !== -1) { label = 'Bridge ' + bridge; tag = 'B' + bridge; bridge++; }
        else if (h.indexOf('outro') !== -1 || h.indexOf('ending') !== -1) { label = 'Ending'; tag = 'E1'; }
        else if (h.indexOf('verse') !== -1) { label = 'Verse ' + verse; tag = 'V' + verse; verse++; }
        else { label = header; tag = 'V' + verse; verse++; }
      } else if ((trimmed || current.length) && !isFurniture(trimmed)) {
        current.push(line);
      }
    });
    flush();

    if (!song.slides.length) {
      String(rawLyrics).split('\n\n').forEach(function (chunk, idx) {
        var t = chunk.trim();
        if (t && !isFurniture(t)) {
          addSlide('Verse ' + (idx + 1), 'V' + (idx + 1), t);
        }
      });
    }
    return song;
  }

  /** A Genius song page (HTML) -> a Song, with the CCLI number if the page mentions one. */
  function pageToSong(html, title, artist) {
    var song = lyricsToSong(title || 'Worship Song', artist || 'Christian Artist', extractLyrics(html));
    var ccli = extractCcli(html);
    if (ccli) song.ccli_number = ccli;
    return song;
  }

  var api = {
    searchTerms: searchTerms, searchUrl: searchUrl, parseSearchResponse: parseSearchResponse, isGeniusUrl: isGeniusUrl,
    extractCcli: extractCcli, stripHtml: stripHtml, extractLyrics: extractLyrics, lyricsToSong: lyricsToSong, pageToSong: pageToSong
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.Genius = api;
})(typeof self !== 'undefined' ? self : this);
