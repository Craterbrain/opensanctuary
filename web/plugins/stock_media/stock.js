// Pexels and Pixabay search: building the requests and reading the answers. Pure functions (no
// DOM, no network) so they can be tested on their own; app.js makes the calls through the host.
(function (root) {
  'use strict';

  var PEXELS_API = 'https://api.pexels.com/v1/search';
  var PIXABAY_API = 'https://pixabay.com/api/';
  var IMAGE_HOSTS = ['images.pexels.com', 'pixabay.com', 'cdn.pixabay.com'];
  var PER_PAGE = 24;

  function clean(text, max) {
    return String(text == null ? '' : text).replace(/[\u0000-\u001f\u007f]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, max || 300);
  }

  /** Pexels wants the key in an Authorization header, so the request is a URL plus headers. */
  function pexelsRequest(query, key) {
    var q = clean(query, 200);
    if (!q || !clean(key)) return null;
    return { url: PEXELS_API + '?query=' + encodeURIComponent(q) + '&per_page=' + PER_PAGE, headers: { Authorization: clean(key, 200) } };
  }

  function pixabayRequest(query, key) {
    var q = clean(query, 100); // Pixabay rejects longer queries
    if (!q || !clean(key)) return null;
    return {
      url: PIXABAY_API + '?key=' + encodeURIComponent(clean(key, 200)) + '&q=' + encodeURIComponent(q) +
        '&image_type=photo&per_page=' + PER_PAGE + '&safesearch=true',
      headers: {}
    };
  }

  function isImageUrl(url) {
    try {
      var u = new URL(String(url));
      return u.protocol === 'https:' && !u.username && !u.password && IMAGE_HOSTS.indexOf(u.hostname.toLowerCase()) !== -1;
    } catch (e) {
      return false;
    }
  }

  function good(hit) {
    return hit.id && isImageUrl(hit.thumb) && isImageUrl(hit.full);
  }

  function parsePexels(body) {
    var data = typeof body === 'string' ? JSON.parse(body) : body;
    return ((data && data.photos) || []).map(function (p) {
      var src = p.src || {};
      return {
        id: 'pexels_' + p.id, provider: 'Pexels', thumb: src.medium, full: src.large2x || src.large || src.original,
        width: p.width, height: p.height, author: clean(p.photographer, 120), authorUrl: p.photographer_url,
        pageUrl: p.url, tags: clean(p.alt, 200)
      };
    }).filter(good);
  }

  function parsePixabay(body) {
    var data = typeof body === 'string' ? JSON.parse(body) : body;
    return ((data && data.hits) || []).map(function (h) {
      return {
        id: 'pixabay_' + h.id, provider: 'Pixabay', thumb: h.webformatURL || h.previewURL, full: h.largeImageURL,
        width: h.imageWidth, height: h.imageHeight, author: clean(h.user, 120), authorUrl: null,
        pageUrl: h.pageURL, tags: clean(h.tags, 200)
      };
    }).filter(good);
  }

  /** Alternate the two providers so neither one's results bury the other's. */
  function interleave(lists) {
    var out = [];
    for (var i = 0; lists.some(function (l) { return i < l.length; }); i++) {
      lists.forEach(function (l) { if (i < l.length) out.push(l[i]); });
    }
    return out;
  }

  /** The library name: keywords first (so a later Media search finds it), then who made it. */
  function libraryName(hit) {
    var tags = clean(hit.tags, 60);
    var more = clean(hit.tags, 1000).length > 60 ? '…' : '';
    var credit = hit.author ? hit.provider + ' photo by ' + hit.author : hit.provider + ' photo';
    return tags ? tags + more + ' (' + credit + ')' : credit;
  }

  /** What a failed answer means, in words for the operator. */
  function explainStatus(provider, status) {
    if (status === 401 || status === 403) return provider + ' did not accept that API key. Check it in “API keys” below.';
    if (status === 429) return provider + ' says too many searches were made. Wait a little and try again.';
    if (status === 400) return provider + ' rejected that request. Check the API key and try different words.';
    return provider + ' answered ' + status + '.';
  }

  var api = {
    pexelsRequest: pexelsRequest, pixabayRequest: pixabayRequest, parsePexels: parsePexels, parsePixabay: parsePixabay,
    interleave: interleave, libraryName: libraryName, explainStatus: explainStatus, isImageUrl: isImageUrl, clean: clean
  };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.Stock = api;
})(typeof self !== 'undefined' ? self : this);
