// Parsing and catalog logic for the Online Bibles plugin. Pure functions (no DOM, no network) so they
// can be tested on their own; app.js does the fetching through the host.
(function (root) {
  'use strict';

  // [name, chapters] in canonical order; a Bolls translation numbers its books 1..66 in this order.
  var BOOKS = [
    ['Genesis', 50], ['Exodus', 40], ['Leviticus', 27], ['Numbers', 36], ['Deuteronomy', 34],
    ['Joshua', 24], ['Judges', 21], ['Ruth', 4], ['1 Samuel', 31], ['2 Samuel', 24],
    ['1 Kings', 22], ['2 Kings', 25], ['1 Chronicles', 29], ['2 Chronicles', 36], ['Ezra', 10],
    ['Nehemiah', 13], ['Esther', 10], ['Job', 42], ['Psalms', 150], ['Proverbs', 31],
    ['Ecclesiastes', 12], ['Song of Solomon', 8], ['Isaiah', 66], ['Jeremiah', 52], ['Lamentations', 5],
    ['Ezekiel', 48], ['Daniel', 12], ['Hosea', 14], ['Joel', 3], ['Amos', 9],
    ['Obadiah', 1], ['Jonah', 4], ['Micah', 7], ['Nahum', 3], ['Habakkuk', 3],
    ['Zephaniah', 3], ['Haggai', 2], ['Zechariah', 14], ['Malachi', 4],
    ['Matthew', 28], ['Mark', 16], ['Luke', 24], ['John', 21], ['Acts', 28],
    ['Romans', 16], ['1 Corinthians', 16], ['2 Corinthians', 13], ['Galatians', 6], ['Ephesians', 6],
    ['Philippians', 4], ['Colossians', 4], ['1 Thessalonians', 5], ['2 Thessalonians', 3],
    ['1 Timothy', 6], ['2 Timothy', 4], ['Titus', 3], ['Philemon', 1], ['Hebrews', 13],
    ['James', 5], ['1 Peter', 5], ['2 Peter', 3], ['1 John', 5], ['2 John', 1],
    ['3 John', 1], ['Jude', 1], ['Revelation', 22]
  ];

  var LANGUAGES = [
    [['eng'], 'English'], [['spa', 'esp'], 'Spanish'], [['fra', 'fre'], 'French'], [['deu', 'ger'], 'German'],
    [['ukr'], 'Ukrainian'], [['rus'], 'Russian'], [['por'], 'Portuguese'], [['chi', 'zho', 'cmn'], 'Chinese'],
    [['lat'], 'Latin'], [['grc', 'ell', 'greek'], 'Greek'], [['heb', 'hebrew'], 'Hebrew'], [['ita', 'italian'], 'Italian'],
    [['kor', 'korean'], 'Korean'], [['tgl', 'fil', 'tagalog'], 'Tagalog'], [['swe', 'swedish'], 'Swedish'],
    [['nor', 'norwegian'], 'Norwegian'], [['dan', 'danish'], 'Danish'], [['nld', 'dut', 'dutch'], 'Dutch'],
    [['afr', 'afrikaans'], 'Afrikaans'], [['ara', 'arabic'], 'Arabic'], [['vie', 'vietnamese'], 'Vietnamese'],
    [['hin', 'hindi'], 'Hindi'], [['ces', 'cze', 'czech'], 'Czech'], [['pol', 'polish'], 'Polish'],
    [['ron', 'rum', 'romanian'], 'Romanian'], [['hun', 'hungarian'], 'Hungarian']
  ];

  // Two-letter folder names ("en", "es") as GitHub Bible repos use them. (The old server importer
  // turned "en" into "En".)
  var ISO2 = { en: 'English', es: 'Spanish', fr: 'French', de: 'German', uk: 'Ukrainian', ru: 'Russian', pt: 'Portuguese',
    zh: 'Chinese', la: 'Latin', el: 'Greek', he: 'Hebrew', it: 'Italian', ko: 'Korean', tl: 'Tagalog', sv: 'Swedish',
    no: 'Norwegian', da: 'Danish', nl: 'Dutch', af: 'Afrikaans', ar: 'Arabic', vi: 'Vietnamese', hi: 'Hindi',
    cs: 'Czech', pl: 'Polish', ro: 'Romanian', hu: 'Hungarian' };

  /** "eng" / "en" / "Spanish" / "" -> a display language name. */
  function normalizeLanguage(lang) {
    var raw = String(lang == null ? '' : lang);
    var lower = raw.trim().toLowerCase();
    if (!lower) return 'English';
    if (ISO2[lower]) return ISO2[lower];
    for (var i = 0; i < LANGUAGES.length; i++) {
      var needles = LANGUAGES[i][0];
      for (var j = 0; j < needles.length; j++) {
        if (lower.indexOf(needles[j]) !== -1) return LANGUAGES[i][1];
      }
    }
    return raw.charAt(0).toUpperCase() + raw.slice(1);
  }

  var CURATED = [
    ['KJV', 'King James Version (Authorized 1769)', 'English'], ['NKJV', 'New King James Version (1982)', 'English'],
    ['ESV', 'English Standard Version (2001, 2016)', 'English'], ['NIV', 'New International Version (1984)', 'English'],
    ['NLT', 'New Living Translation (2015)', 'English'], ['CSB17', 'Christian Standard Bible (2017)', 'English'],
    ['NASB', 'New American Standard Bible (1995)', 'English'], ['ASV', 'American Standard Version (1901)', 'English'],
    ['BSB', 'Berean Standard Bible', 'English'], ['WEB', 'World English Bible', 'English'],
    ['YLT', "Young's Literal Translation (1898)", 'English'], ['DBY', 'Darby Translation (1890)', 'English'],
    ['DRB', 'Douay-Rheims Bible (1899)', 'English'], ['MSG', 'The Message (2002)', 'English'],
    ['AMP', 'Amplified Bible (2015)', 'English'], ['RVR1960', 'Reina-Valera 1960', 'Spanish'],
    ['LSG', 'Louis Segond 1910', 'French'], ['LUT', 'Lutherbibel 1912', 'German'],
    ['CLE', 'Biblia Sacra Vulgata (Clementine Latin)', 'Latin'],
    ['ALM', 'João Ferreira de Almeida Revista e Corrigida', 'Portuguese'],
    ['SYNOD', 'Синодальный перевод (Russian Synodal 1876)', 'Russian'],
    ['UBIO', 'Біблія в пер. Івана Огієнка (1962)', 'Ukrainian'], ['CUV', '和合本 (Chinese Union Version)', 'Chinese'],
    ['TAG', 'Ang Biblia (1905)', 'Tagalog'], ['LXXE', 'English Septuagint Bible (Brenton 1851)', 'Greek']
  ];

  /**
   * Merges the curated list, the Bolls.life language index and the ChurchApps list into one catalog,
   * de-duplicated by (abbreviation, language) with the sources combined; English first, then by
   * language and name. Entries: { id, name, language, abbreviation, source, provider, sourceKey }.
   */
  function buildCatalog(bollsLanguages, churchAppsItems) {
    var map = {};
    function add(item, label) {
      var abbr = String(item.abbreviation || item.id || '').trim().toUpperCase();
      if (!abbr) return;
      var lang = normalizeLanguage(item.language || 'English');
      var key = abbr + '\u0000' + lang.toLowerCase();
      var existing = map[key];
      if (existing) {
        if (existing.source.indexOf(label) === -1) existing.source = existing.source ? existing.source + ' • ' + label : label;
        if (String(item.name || '').length > existing.name.length) existing.name = item.name;
      } else {
        map[key] = { id: String(item.id || abbr), name: String(item.name || abbr), language: lang, abbreviation: abbr,
                     source: label, provider: 'bolls', sourceKey: item.sourceKey || item.source_key || null };
      }
    }
    CURATED.forEach(function (c) { add({ id: c[0], abbreviation: c[0], name: c[1], language: c[2] }, 'OpenLP Community'); });
    (Array.isArray(bollsLanguages) ? bollsLanguages : []).forEach(function (l) {
      var langName = (l && l.language) || 'English';
      ((l && l.translations) || []).forEach(function (t) {
        var short = t && t.short_name;
        if (short) add({ id: short, abbreviation: short, name: t.full_name || short, language: langName }, 'Bolls.life');
      });
    });
    (Array.isArray(churchAppsItems) ? churchAppsItems : []).forEach(function (it) {
      if (it && typeof it === 'object') add(it, 'ChurchApps');
    });
    var list = Object.keys(map).map(function (k) { return map[k]; });
    list.sort(function (a, b) {
      var aEng = a.language === 'English', bEng = b.language === 'English';
      if (aEng !== bEng) return aEng ? -1 : 1;
      if (a.language !== b.language) return a.language < b.language ? -1 : 1;
      return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
    });
    return list;
  }

  /** HTML tags and Strong's-number markup out of a Bolls verse. */
  function cleanVerseText(raw) {
    var out = '';
    var inTag = false;
    // A line break inside a verse is a space, not "beginningGod".
    var s = String(raw).replace(/<br\s*\/?>/gi, ' ');
    for (var i = 0; i < s.length; i++) {
      var c = s.charAt(i);
      if (c === '<') inTag = true;
      else if (c === '>') inTag = false;
      else if (!inTag) out += c;
    }
    return out.replace(/\s+/g, ' ').trim();
  }

  /** A Bolls translation file ([{book, chapter, verse, text}, ...]) -> chapters, in book order. */
  function parseBollsTranslation(jsonText, translation) {
    var verses;
    try { verses = JSON.parse(jsonText); } catch (e) { return []; }
    if (!Array.isArray(verses)) return [];
    var version = String(translation).trim().toUpperCase();
    var chapters = {};
    var order = [];
    verses.forEach(function (v) {
      if (!v || v.book < 1 || v.book > 66) return;
      var key = v.book + ':' + v.chapter;
      if (!chapters[key]) { chapters[key] = { book: BOOKS[v.book - 1][0], chapter: v.chapter, version: version, verses: [], _b: v.book }; order.push(key); }
      chapters[key].verses.push({ verse_number: v.verse, text: cleanVerseText(v.text) });
    });
    return order.map(function (k) { return chapters[k]; })
      .sort(function (a, b) { return a._b - b._b || a.chapter - b.chapter; })
      .map(function (c) {
        delete c._b;
        c.verses.sort(function (x, y) { return x.verse_number - y.verse_number; });
        return c;
      });
  }

  /** "KING JAMES BIBLE" -> "KJB"; a single word is kept as it is ("KJV" -> "KJV"). */
  function makeAbbreviation(s) {
    var words = String(s).trim().split(/\s+/).filter(Boolean);
    if (words.length <= 1) return (words[0] || '').toUpperCase().slice(0, 12);
    return words.map(function (w) { return w.charAt(0); }).join('').toUpperCase();
  }

  function titleCase(s) {
    return String(s).trim().split(/\s+/).filter(Boolean).map(function (w) { return w.charAt(0).toUpperCase() + w.slice(1).toLowerCase(); }).join(' ');
  }

  /** "owner/repo" -> { owner, repo } or null. */
  function parseRepo(text) {
    var parts = String(text).trim().replace(/^\/+|\/+$/g, '').split('/');
    if (parts.length !== 2 || !/^[A-Za-z0-9_.-]+$/.test(parts[0]) || !/^[A-Za-z0-9_.-]+$/.test(parts[1])) return null;
    // "." and ".." would be path segments in the API URL, not names.
    if (/^\.+$/.test(parts[0]) || /^\.+$/.test(parts[1])) return null;
    return { owner: parts[0], repo: parts[1] };
  }

  function githubContentsUrl(repo, path) {
    var clean = String(path).split('/').filter(function (p) { return p && p !== '..' && p !== '.'; }).map(encodeURIComponent).join('/');
    return 'https://api.github.com/repos/' + repo.owner + '/' + repo.repo + '/contents/' + clean;
  }

  /** Entries of a GitHub "versions/<lang>/<NAME>.json" listing -> catalog entries. */
  function githubCatalogEntries(repo, langCode, files) {
    var out = [];
    (Array.isArray(files) ? files : []).forEach(function (f) {
      if (!f || f.type !== 'file' || !/\.json$/.test(f.name || '')) return;
      var stem = f.name.replace(/\.json$/, '').trim();
      var abbr = makeAbbreviation(stem);
      var url = f.download_url || ('https://raw.githubusercontent.com/' + repo.owner + '/' + repo.repo + '/main/' + f.path);
      out.push({ id: 'gh_' + langCode + '_' + abbr.toLowerCase(), name: titleCase(stem), language: normalizeLanguage(langCode),
                 abbreviation: abbr, source: 'GitHub: ' + repo.owner + '/' + repo.repo, provider: 'github', sourceKey: url });
    });
    return out;
  }

  function isRawGithubUrl(url) {
    return /^https:\/\/raw\.githubusercontent\.com\//.test(String(url));
  }

  /** { "Book": { "1": { "1": "text" } } } (+ the file's URL for the version name) -> chapters. */
  function parseGithubBible(jsonText, url) {
    var data;
    try { data = JSON.parse(jsonText); } catch (e) { return []; }
    if (!data || typeof data !== 'object' || Array.isArray(data)) return [];
    var file = String(url).split('/').pop().replace(/\.json$/, '');
    var version = (file && makeAbbreviation(decodeURIComponent(file))) || 'BIBLE';
    var items = [];
    Object.keys(data).forEach(function (book) {
      var chapters = data[book];
      if (!chapters || typeof chapters !== 'object') return;
      Object.keys(chapters).map(Number).filter(function (n) { return !isNaN(n); }).sort(function (a, b) { return a - b; }).forEach(function (ch) {
        var verseMap = chapters[String(ch)];
        if (!verseMap || typeof verseMap !== 'object') return;
        var verses = Object.keys(verseMap).map(Number).filter(function (n) { return !isNaN(n); }).sort(function (a, b) { return a - b; })
          .map(function (n) { return { verse_number: n, text: String(verseMap[String(n)] == null ? '' : verseMap[String(n)]).trim() }; });
        if (verses.length) items.push({ book: book, chapter: ch, version: version, verses: verses });
      });
    });
    return items;
  }

  // ---- single passages (bible-api.com) -----------------------------------------------------------

  var BOOK_BY_LOWER = {};
  BOOKS.forEach(function (b) { BOOK_BY_LOWER[b[0].toLowerCase()] = b; });
  ['psalm', 'song of songs', 'revelations'].forEach(function (alias) {
    BOOK_BY_LOWER[alias] = alias === 'psalm' ? BOOK_BY_LOWER['psalms'] : alias === 'song of songs' ? BOOK_BY_LOWER['song of solomon'] : BOOK_BY_LOWER['revelation'];
  });

  /** The canonical [name, chapters] when the query is exactly a book name ("Psalms"), else null. */
  function exactBook(query) {
    return BOOK_BY_LOWER[String(query).trim().toLowerCase().replace(/\.$/, '')] || null;
  }

  var SINGLE_CHAPTER_RANGES = { Obadiah: '1:1-21', Philemon: '1:1-25', '2 John': '1:1-13', '3 John': '1:1-14', Jude: '1:1-25' };

  /** bible-api.com URL for one chapter of a book (the one-chapter books need an explicit verse range). */
  function chapterUrl(bookName, chapter, translation) {
    var q = (chapter === 1 && SINGLE_CHAPTER_RANGES[bookName])
      ? bookName.replace(/ /g, '+') + '+' + SINGLE_CHAPTER_RANGES[bookName]
      : bookName.replace(/ /g, '+') + '+' + chapter;
    return 'https://bible-api.com/' + q + '?translation=' + encodeURIComponent(String(translation).toLowerCase());
  }

  function passageUrl(query, translation) {
    return 'https://bible-api.com/' + encodeURIComponent(String(query).trim()) + '?translation=' + encodeURIComponent(String(translation).toLowerCase());
  }

  /** A bible-api.com response -> one chapter-shaped item (the passage's own verses), or null. */
  function parseBibleApi(jsonText, translation) {
    var d;
    try { d = JSON.parse(jsonText); } catch (e) { return null; }
    if (!d || !Array.isArray(d.verses) || !d.verses.length) return null;
    var first = d.verses[0];
    var version = String(d.translation_id || translation).toUpperCase();
    return {
      book: first.book_name, chapter: first.chapter, version: version,
      verses: d.verses.map(function (v) { return { verse_number: v.verse, text: String(v.text).trim() }; })
    };
  }

  function chunk(list, size) {
    var out = [];
    for (var i = 0; i < list.length; i += size) out.push(list.slice(i, i + size));
    return out;
  }

  var api = {
    BOOKS: BOOKS, CURATED: CURATED, normalizeLanguage: normalizeLanguage, buildCatalog: buildCatalog, cleanVerseText: cleanVerseText,
    parseBollsTranslation: parseBollsTranslation, makeAbbreviation: makeAbbreviation, titleCase: titleCase, parseRepo: parseRepo,
    githubContentsUrl: githubContentsUrl, githubCatalogEntries: githubCatalogEntries, isRawGithubUrl: isRawGithubUrl,
    parseGithubBible: parseGithubBible, exactBook: exactBook, chapterUrl: chapterUrl, passageUrl: passageUrl, parseBibleApi: parseBibleApi,
    chunk: chunk,
    bollsUrl: function (abbr) { return 'https://bolls.life/static/translations/' + encodeURIComponent(String(abbr).trim().toUpperCase()) + '.json'; },
    BOLLS_LANGUAGES_URL: 'https://bolls.life/static/bolls/app/views/languages.json',
    CHURCHAPPS_URL: 'https://api.churchapps.org/content/bibles'
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.Bibles = api;
})(typeof self !== 'undefined' ? self : this);
