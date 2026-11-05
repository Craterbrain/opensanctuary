// Parsing for Genius search results and lyrics pages. Pure functions (no DOM, no network) so they
// can be tested on their own; the page code in app.js does the fetching through the host.
(function (root) {
  'use strict';

  /** The search term sent to Genius: what the user typed. (Adding "christian worship" to bias the
   *  results used to push the actual song out of them -- "Lead On Good Shepherd christian worship"
   *  finds nothing, the plain title finds it. Non-Christian results are now filtered by genre
   *  instead; see classifyHits.) */
  function searchTerms(query) {
    return String(query).trim();
  }

  function searchUrl(query) {
    var terms = searchTerms(query);
    return terms ? 'https://genius.com/api/search/multi?q=' + encodeURIComponent(terms) : '';
  }

  var STOP_WORDS = ['the', 'a', 'an', 'of', 'by', 'and', 'in', 'to', 'for', 'with'];

  function words(text) {
    return String(text).toLowerCase().replace(/[^a-z0-9\u00c0-\uffff]+/g, ' ').split(' ').filter(Boolean);
  }

  /**
   * Does a result actually answer the search? At least 60% of the words the user typed (ignoring "the",
   * "of" ...) have to appear in the result's title, artist or address. Genius always returns *something*,
   * and listing five unrelated songs for a song it doesn't have is worse than saying nothing was found.
   */
  function answersQuery(query, title, artist, url) {
    var wanted = words(query).filter(function (w) { return STOP_WORDS.indexOf(w) === -1; });
    if (!wanted.length) return true;
    var haystack = ' ' + words(title + ' ' + artist + ' ' + url).join(' ') + ' ';
    var found = wanted.filter(function (w) { return haystack.indexOf(w) !== -1; }).length;
    return found >= Math.max(1, Math.ceil(wanted.length * 0.6));
  }

  /** Genius multi-search JSON (text) -> [{ id, title, artist, url, thumbnail }]. */
  function parseSearchResponse(jsonText, query) {
    var data;
    try { data = JSON.parse(jsonText); } catch (e) { return []; }
    var sections = (data && data.response && data.response.sections) || [];
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
        if (answersQuery(query, title, artist, url)) {
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
  /** Removes every `<div data-exclude-from-selection="true">...</div>` subtree: Genius puts its
   *  LyricsHeader (contributor count, title, and a song-bio preview) inside the first lyrics
   *  container under that marker, and the bio otherwise lands on the first slide as a "verse". */
  function dropExcluded(chunk) {
    var out = '';
    var rest = chunk;
    var re = /<div\b[^>]*data-exclude-from-selection="true"[^>]*>/i;
    for (;;) {
      var m = re.exec(rest);
      if (!m) return out + rest;
      out += rest.slice(0, m.index);
      var inside = rest.slice(m.index + m[0].length);
      var close = matchingDivEnd(inside);
      if (close === -1) return out; // unbalanced: drop the remainder rather than leak page furniture
      rest = inside.slice(close + 6);
    }
  }

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
      chunks.push(dropExcluded(inside.slice(0, close)));
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


  // ---- LRCLIB (https://lrclib.net): a free, community-built lyrics database with an open API ----------------

  function lrclibSearchUrl(query) {
    var q = String(query).trim();
    return q ? 'https://lrclib.net/api/search?q=' + encodeURIComponent(q) : '';
  }

  /** "3:25" from seconds, or '' when unknown. */
  function clock(seconds) {
    var n = Math.round(Number(seconds));
    if (!isFinite(n) || n <= 0) return '';
    var s = n % 60;
    return Math.floor(n / 60) + ':' + (s < 10 ? '0' : '') + s;
  }

  /**
   * LRCLIB search JSON (text) -> [{ source:'lrclib', id, title, artist, album, length, plain }]. Only entries
   * with words (not instrumentals) that answer the search, and one per title/artist/album: LRCLIB often
   * holds the same recording several times.
   */
  function parseLrclibResponse(jsonText, query) {
    var data;
    try { data = JSON.parse(jsonText); } catch (e) { return []; }
    if (!Array.isArray(data)) return [];
    var hits = [];
    var seen = {};
    data.forEach(function (r) {
      if (!r || r.instrumental) return;
      var plain = typeof r.plainLyrics === 'string' ? r.plainLyrics.trim() : '';
      if (!plain) return;
      var title = String(r.trackName || r.name || '').replace(/\s+/g, ' ').trim();
      var artist = String(r.artistName || '').replace(/\s+/g, ' ').trim();
      if (!title || !answersQuery(query, title, artist, '')) return;
      var album = String(r.albumName || '').replace(/\s+/g, ' ').trim();
      var key = (title + '|' + artist + '|' + album).toLowerCase();
      if (seen[key]) return;
      seen[key] = true;
      hits.push({ source: 'lrclib', id: 'lrclib_' + r.id, title: title, artist: artist || 'Unknown artist',
                  album: album, length: clock(r.duration), plain: plain });
    });
    return hits;
  }

  function sameLines(a, b) { return a.replace(/\s+/g, ' ').toLowerCase() === b.replace(/\s+/g, ' ').toLowerCase(); }

  /**
   * Plain lyrics (stanzas separated by blank lines, no section names) -> a Song. A stanza that comes back
   * word for word is the chorus; the others are numbered verses. Slides stay in singing order.
   */
  function plainLyricsToSong(title, artist, album, plain) {
    var stanzas = String(plain).replace(/\r/g, '').split(/\n\s*\n/)
      .map(function (t) { return t.split('\n').map(function (l) { return l.trim(); }).filter(Boolean).join('\n'); })
      .filter(Boolean);
    var kinds = stanzas.map(function (t, i) {
      return stanzas.some(function (u, j) { return j !== i && sameLines(t, u); });
    });
    var verse = 0;
    var song = {
      title: title, author: artist, copyright: 'Lyrics from LRCLIB (community-contributed)' + (album ? ' \u2014 ' + album : ''),
      ccli_number: null, slides: [], theme_name: null, alternate_title: null
    };
    stanzas.forEach(function (t, i) {
      var label, tag;
      if (kinds[i]) {
        // every copy of the same repeated stanza shares one name; different repeated stanzas get their own
        var base = stanzas.findIndex(function (u, j) { return kinds[j] && sameLines(u, t); });
        var n = stanzas.slice(0, base + 1).filter(function (u, j) { return kinds[j] && stanzas.findIndex(function (v, k) { return kinds[k] && sameLines(v, u); }) === j; }).length;
        label = 'Chorus ' + n; tag = 'C' + n;
      } else {
        verse++;
        label = 'Verse ' + verse; tag = 'V' + verse;
      }
      song.slides.push({ text: t, header: tag, label: label, tag: label });
    });
    return song;
  }

  // ---- Christian-music filter: genres from Apple's iTunes Search API ------------------------------------------
  // Neither LRCLIB nor Genius says what kind of music a song is (Genius tags a worship song like "Lead On Good
  // Shepherd" plain "Pop"). The iTunes store files every commercial release under a genre, and Christian music
  // has its own ("Christian", "Gospel", "CCM", "Southern Gospel", ...). One song search per query covers most
  // results; artists it doesn't cover are looked up on their own (an artist's primary genre).

  var CHRISTIAN_GENRE = /christian|gospel|\bccm\b|worship|inspirational/i;
  var CHRISTIAN_ARTIST_WORDS = /\b(worship|church|gospel|praise|christian|hymns?|ministr(y|ies)|tabernacle)\b/i;

  function isChristianGenre(genre) { return CHRISTIAN_GENRE.test(String(genre || '')); }

  function itunesSongSearchUrl(query) {
    var q = String(query).trim();
    return q ? 'https://itunes.apple.com/search?media=music&entity=song&limit=200&term=' + encodeURIComponent(q) : '';
  }

  function itunesArtistSearchUrl(artist) {
    var a = String(artist).trim();
    return a ? 'https://itunes.apple.com/search?media=music&entity=musicArtist&limit=5&term=' + encodeURIComponent(a) : '';
  }

  /** A name compared loosely: lower case, accents and punctuation dropped, "the" dropped. */
  function normName(text) {
    return String(text || '').toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '')
      .replace(/[^a-z0-9À-￿]+/g, ' ').replace(/^the /, '').trim();
  }

  /** Every artist credited in a credit line, the primary one first: "Patrick Mayberry & Crowder",
   *  "Bethel Music (Ft. Brian Johnson)". */
  function artistNames(credit) {
    return String(credit || '')
      // Only the separators credit lines actually use: splitting on "and" would turn "Hillsong Young and
      // Free" into an artist called "Free" (a rock band). "feat"/"ft" only as whole words (not "Feather
      // Rose"), "/" and "+" only between spaces (not "AC/DC").
      .split(/\s*(?:,|&|•|\(|\)|\[|\]|\bfeat\b\.?|\bft\b\.?|\bfeaturing\b)\s*|\s+[\/+]\s+/i)
      .map(normName)
      .filter(function (n) { return n && n !== 'recommended for you'; });
  }

  /** A title without its "(Live)", "[feat. ...]", " - Radio Edit" decorations. */
  function coreTitle(title) {
    return normName(String(title || '').replace(/\([^)]*\)|\[[^\]]*\]/g, ' ').replace(/\s[-–—]\s.*$/, ''));
  }

  /** iTunes song search JSON (text) -> [{ title, artists, genre }]. */
  function parseItunesSongs(jsonText) {
    var data;
    try { data = JSON.parse(jsonText); } catch (e) { return []; }
    return ((data && data.results) || []).filter(function (r) { return r && r.trackName && r.primaryGenreName; })
      .map(function (r) { return { title: coreTitle(r.trackName), artists: artistNames(r.artistName), genre: String(r.primaryGenreName) }; });
  }

  /** iTunes artist search JSON (text) -> the primary genre of the artist named exactly `artist`, or null. */
  function parseItunesArtistGenre(jsonText, artist) {
    var data;
    try { data = JSON.parse(jsonText); } catch (e) { return null; }
    var want = normName(artist);
    var hit = ((data && data.results) || []).find(function (r) { return r && r.primaryGenreName && normName(r.artistName) === want; });
    return hit ? String(hit.primaryGenreName) : null;
  }

  function overlaps(a, b) { return a.some(function (x) { return b.indexOf(x) !== -1; }); }

  /** Whether two core titles name the same song: equal, or one the other's start at a word break
   *  ("way maker" / "way maker live session") -- never an empty title, nor a start shorter than four
   *  letters ("holy" matches "holy forever", "go" matches nothing it merely begins). */
  function sameTitle(a, b) {
    if (!a || !b) return false;
    if (a === b) return true;
    var shorter = a.length < b.length ? a : b, longer = a.length < b.length ? b : a;
    return shorter.length >= 4 && longer.indexOf(shorter) === 0 && longer.charAt(shorter.length) === ' ';
  }

  /** Genres known for each artist from song records: { "patrick mayberry": ["Christian"], ... }. */
  function genresByArtist(records) {
    // No prototype: an artist called "constructor" or "toString" is a key like any other.
    var map = Object.create(null);
    records.forEach(function (r) {
      r.artists.forEach(function (a) { (map[a] = map[a] || []).push(r.genre); });
    });
    return map;
  }

  /** The verdict for one artist from the genres known for them: 'christian', 'other' or null (no idea). */
  function verdict(genres) {
    if (!genres || !genres.length) return null;
    return genres.some(isChristianGenre) ? 'christian' : 'other';
  }

  /**
   * Labels each hit `faith: 'christian' | 'other' | 'unknown'` (and `genre` when known), in place:
   *   1. the same song by the same artist in the iTunes results decides;
   *   2. otherwise the artist's genres (their other songs there, or `artistGenres` from an artist lookup);
   *   3. otherwise a worship word in the artist's name ("Generations Worship") counts as Christian -- not in
   *      the title: plenty of songs called "Hallelujah" or "Grace" aren't;
   *   4. otherwise it's unknown -- shown, but after the songs known to be Christian.
   * Artist names must match exactly (after normalizing), so "Chris Brown" never vouches for "Dennis Brown".
   */
  function classifyHits(hits, songRecords, artistGenres) {
    var byArtist = genresByArtist(songRecords);
    artistGenres = artistGenres || Object.create(null);
    var genreOf = function (a) { return Object.prototype.hasOwnProperty.call(artistGenres, a) ? artistGenres[a] : null; };
    hits.forEach(function (hit) {
      var artists = artistNames(hit.artist);
      var title = coreTitle(hit.title);
      var same = songRecords.filter(function (r) {
        return overlaps(r.artists, artists) && sameTitle(r.title, title);
      });
      var v = verdict(same.map(function (r) { return r.genre; }));
      var genre = same.length ? (same.find(function (r) { return isChristianGenre(r.genre); }) || same[0]).genre : null;
      // The primary artist's genres decide before a featured artist's.
      for (var i = 0; !v && i < artists.length; i++) {
        var known = (byArtist[artists[i]] || []).concat(genreOf(artists[i]) ? [genreOf(artists[i])] : []);
        v = verdict(known);
        if (v) genre = known.find(isChristianGenre) || known[0];
      }
      if (!v && CHRISTIAN_ARTIST_WORDS.test(hit.artist)) v = 'christian';
      hit.faith = v || 'unknown';
      hit.genre = genre || null;
    });
    return hits;
  }

  /** Artists of hits the song records say nothing about, other than the ones in `skip` (looked up
   *  already): the ones worth an artist lookup, the first `max`. */
  function artistsToLookUp(hits, songRecords, max, skip) {
    var byArtist = genresByArtist(songRecords);
    var out = [];
    hits.forEach(function (hit) {
      var artists = artistNames(hit.artist);
      if (!artists.length || artists.some(function (a) { return byArtist[a]; })) return;
      if (skip && Object.prototype.hasOwnProperty.call(skip, artists[0])) return;
      if (out.indexOf(artists[0]) === -1) out.push(artists[0]);
    });
    return out.slice(0, max);
  }

  var api = {
    isChristianGenre: isChristianGenre, itunesSongSearchUrl: itunesSongSearchUrl, itunesArtistSearchUrl: itunesArtistSearchUrl,
    artistNames: artistNames, coreTitle: coreTitle, parseItunesSongs: parseItunesSongs, parseItunesArtistGenre: parseItunesArtistGenre,
    classifyHits: classifyHits, artistsToLookUp: artistsToLookUp, sameTitle: sameTitle,
    searchTerms: searchTerms, answersQuery: answersQuery, searchUrl: searchUrl, parseSearchResponse: parseSearchResponse, isGeniusUrl: isGeniusUrl,
    lrclibSearchUrl: lrclibSearchUrl, parseLrclibResponse: parseLrclibResponse, plainLyricsToSong: plainLyricsToSong,
    extractCcli: extractCcli, stripHtml: stripHtml, extractLyrics: extractLyrics, lyricsToSong: lyricsToSong, pageToSong: pageToSong
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else { root.Lyrics = api; root.Genius = api; } // Genius: the name older copies of this page used
})(typeof self !== 'undefined' ? self : this);
