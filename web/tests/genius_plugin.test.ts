import { test, expect, describe } from "bun:test";
// The plugin's pure parsing module (CommonJS, also loaded as a plain <script> in the sandbox).
// @ts-ignore
import Genius from "../plugins/genius_lyrics/genius.js";

describe("Genius plugin: search", () => {
  test("biases toward worship music unless the query already says so", () => {
    expect(Genius.searchTerms("Way Maker")).toBe("Way Maker christian worship");
    expect(Genius.searchTerms("  Oceans  ")).toBe("Oceans christian worship");
    expect(Genius.searchTerms("Goodness of God")).toBe("Goodness of God"); // 'god' counts as a Christian term
    expect(Genius.searchTerms("Holy Forever")).toBe("Holy Forever"); // 'holy' already a Christian term
    expect(Genius.searchTerms("   ")).toBe("");
    expect(Genius.searchUrl("")).toBe("");
    expect(Genius.searchUrl("Way Maker")).toBe("https://genius.com/api/search/multi?q=Way%20Maker%20christian%20worship");
  });

  const response = (hits: any[], type = "song") => JSON.stringify({ response: { sections: [{ type, hits }] } });
  const hit = (over: any) => ({ result: { id: 1, title: "T", artist_names: "A", url: "https://genius.com/a-t-lyrics", ...over } });

  test("keeps lyrics pages, drops calendars/tracklists/non-lyrics, de-duplicates by URL", () => {
    const body = response([
      hit({ id: 1, title: "Way Maker", artist_names: "Sinach", url: "https://genius.com/sinach-way-maker-lyrics" }),
      hit({ id: 2, title: "Way Maker", artist_names: "Sinach", url: "https://genius.com/sinach-way-maker-lyrics" }), // dup
      hit({ id: 3, title: "Worship Release Calendar", url: "https://genius.com/calendar-lyrics" }),
      hit({ id: 4, title: "Some Tracklist", url: "https://genius.com/x-tracklist-lyrics" }),
      hit({ id: 5, title: "Not a lyrics page", url: "https://genius.com/artists/sinach" }),
    ]);
    const hits = Genius.parseSearchResponse(body, "way maker");
    expect(hits.map((h: any) => h.id)).toEqual(["genius_1"]);
    expect(hits[0]).toMatchObject({ title: "Way Maker", artist: "Sinach", url: "https://genius.com/sinach-way-maker-lyrics" });
  });

  test("lists only results that answer the search, never a padded list of unrelated songs", () => {
    const many = Array.from({ length: 8 }, (_, i) =>
      hit({ id: 10 + i, title: `Pop ${i}`, artist_names: "Band", url: `https://genius.com/band-pop-${i}-lyrics` }));
    // Genius always answers with something; a song it doesn't have must not produce five unrelated ones.
    expect(Genius.parseSearchResponse(response(many), "so much life.church")).toEqual([]);
    expect(Genius.parseSearchResponse(response(many), "pop").length).toBe(8);
    const mixed = [...many, hit({ id: 99, title: "So Much", artist_names: "Life.Church Worship", url: "https://genius.com/life-church-worship-so-much-lyrics" })];
    expect(Genius.parseSearchResponse(response(mixed), "so much life.church").map((h: any) => h.id)).toEqual(["genius_99"]);
  });

  test("answersQuery: 60% of the typed words, ignoring filler words", () => {
    expect(Genius.answersQuery("goodness of god", "Goodness of God", "Bethel", "")).toBe(true);
    expect(Genius.answersQuery("goodness of god", "Way Maker", "Sinach", "")).toBe(false);
    expect(Genius.answersQuery("the", "Anything", "Anyone", "")).toBe(true); // nothing to match on
    expect(Genius.answersQuery("so much life.church", "So Much", "Life.Church Worship", "")).toBe(true);
  });

  test("tolerates garbage", () => {
    expect(Genius.parseSearchResponse("not json", "x")).toEqual([]);
    expect(Genius.parseSearchResponse("{}", "x")).toEqual([]);
    expect(Genius.parseSearchResponse(response([{}, { result: null }]), "x")).toEqual([]);
    expect(Genius.parseSearchResponse(response([hit({})], "artist"), "x")).toEqual([]); // wrong section type
  });
});

describe("Genius plugin: pages", () => {
  test("only plain genius.com https URLs are accepted", () => {
    expect(Genius.isGeniusUrl("https://genius.com/Hillsong-united-oceans-lyrics")).toBe(true);
    expect(Genius.isGeniusUrl("https://www.genius.com/x")).toBe(true);
    for (const bad of ["http://genius.com/x", "https://genius.com@127.0.0.1/x", "https://genius.com.evil.test/x",
                       "https://127.0.0.1:8080/api/settings", "file:///etc/passwd", "genius.com/x"]) {
      expect(Genius.isGeniusUrl(bad)).toBe(false);
    }
  });

  test("strips markup and decodes entities", () => {
    expect(Genius.stripHtml("a<br/>b<br>c<br />d</p>e</div>f")).toBe("a\nb\nc\nd\n\ne\nf");
    expect(Genius.stripHtml("<i>It&#39;s</i> &quot;ok&quot; &amp; &lt;fine&gt;&nbsp;&#x27;")).toBe("It's \"ok\" & <fine> '");
  });

  test("extracts lyrics from the lyrics containers and falls back to the legacy div", () => {
    const html = `<html><div data-lyrics-container="true" class="x">[Verse 1]<br/>Line one<br/>Line two</div>
      <div>noise</div><div data-lyrics-container="true">[Chorus]<br/>Sing it</div></html>`;
    expect(Genius.extractLyrics(html)).toBe("[Verse 1]\nLine one\nLine two\n[Chorus]\nSing it");
    const legacy = `<div class="lyrics">[Verse 1]<br>Old page</div>`;
    expect(Genius.extractLyrics(legacy).trim()).toBe("[Verse 1]\nOld page");
    expect(Genius.extractLyrics("<html>nothing</html>")).toBe("");
  });

  test("keeps lyrics that follow a nested div (ads/headers) inside a container", () => {
    const html = `<div data-lyrics-container="true"><div class="LyricsHeader">1 ContributorSong Lyrics</div>[Verse 1]<br/>First<div class="ad">ADVERT</div><br/>Second<br/>[Chorus]<br/>Sing</div><div>after</div>`;
    const lyrics = Genius.extractLyrics(html);
    expect(lyrics).toContain("First");
    expect(lyrics).toContain("Second");
    expect(lyrics).toContain("[Chorus]\nSing");
    expect(lyrics).not.toContain("after");
    // Unbalanced markup is dropped rather than looping or swallowing the page.
    expect(Genius.extractLyrics('<div data-lyrics-container="true">never closed')).toBe("");
  });

  test("drops the LyricsHeader (contributors, title, song bio) Genius puts inside the container", () => {
    // Shape of a real 2026 Genius page: the header is marked data-exclude-from-selection and its
    // bio preview has no "Contributors"/"Embed" furniture words, so only the marker can catch it.
    const html = `<div data-lyrics-container="true" class="Lyrics__Container"><div data-exclude-from-selection="true" class="LyricsHeader__Container">` +
      `<button><span>8 Contributors</span></button><div><div><h2>Amazing Grace Lyrics</h2></div></div>` +
      `<div class="SongBioPreview"><a href="#about">Two artists rework the classic hymn. <span>Read More</span></a></div></div>` +
      `[Verse 1]<br/>Amazing grace, how sweet the sound<br/>[Chorus]<br/>My chains are gone</div>`;
    const lyrics = Genius.extractLyrics(html);
    expect(lyrics).not.toContain("rework the classic hymn");
    expect(lyrics).not.toContain("Amazing Grace Lyrics");
    const song = Genius.pageToSong(html, "Amazing Grace", "Chris Tomlin");
    expect(song.slides.map((s: any) => s.label)).toEqual(["Verse 1", "Chorus 1"]);
    expect(song.slides[0].text).toBe("Amazing grace, how sweet the sound");
  });

  test("finds a CCLI number near the word CCLI", () => {
    expect(Genius.extractCcli("Song info CCLI #7115744 here")).toBe("7115744");
    expect(Genius.extractCcli("ccli song number: 123")).toBe(null); // too short
    expect(Genius.extractCcli("CCLI 12 then later CCLI License 7115744")).toBe("7115744");
    expect(Genius.extractCcli("no mention")).toBe(null);
  });
});

describe("Genius plugin: lyrics -> song", () => {
  test("maps section headers to labelled slides in the library's shape", () => {
    const lyrics = "[Verse 1]\nA1\nA2\n\n[Chorus]\nC1\n\n[Verse 2]\nB1\n\n[Bridge]\nBr\n\n[Chorus]\nC1\n\n[Outro]\nEnd\n\n[Pre-Chorus]\nP\n\n[Tag]\nT";
    const song = Genius.lyricsToSong("Title", "Artist", lyrics);
    expect(song.title).toBe("Title");
    expect(song.author).toBe("Artist");
    expect(song.copyright).toBe("Genius Christian Lyrics");
    expect(song.slides.map((s: any) => s.label)).toEqual(["Verse 1", "Chorus 1", "Verse 2", "Bridge 1", "Chorus 2", "Ending", "Chorus 3", "Tag"]);
    expect(song.slides.map((s: any) => s.header)).toEqual(["V1", "C1", "V2", "B1", "C2", "E1", "C3", "V3"]);
    expect(song.slides[0]).toEqual({ text: "A1\nA2", header: "V1", label: "Verse 1", tag: "Verse 1" });
  });

  test("drops page furniture and falls back to paragraphs when there are no headers", () => {
    const withFurniture = Genius.lyricsToSong("T", "A", "[Verse 1]\nReal line\n12 Contributors\nTranslations\n42Embed");
    expect(withFurniture.slides.length).toBe(1);
    expect(withFurniture.slides[0].text).toBe("Real line");
    // The header Genius puts ahead of the lyrics ("1 Contributor" + "<Title> Lyrics", run together) is not a slide.
    const withHeader = Genius.lyricsToSong("Way Maker", "Sinach", "1 ContributorWay Maker Lyrics\n[Verse 1]\nYou are here\n[Chorus]\nWaymaker");
    expect(withHeader.slides.map((s: any) => s.label)).toEqual(["Verse 1", "Chorus 1"]);
    expect(withHeader.slides[0].text).toBe("You are here");
    const plain = Genius.lyricsToSong("T", "A", "first para\nstill first\n\nsecond para\n\n12 Contributors");
    // Without headers it is one slide (blank lines inside a section are kept), furniture removed.
    expect(plain.slides.map((s: any) => s.text)).toEqual(["first para\nstill first\n\nsecond para"]);
    expect(plain.slides.map((s: any) => s.label)).toEqual(["Verse 1"]);
    expect(Genius.lyricsToSong("T", "A", "").slides).toEqual([]);
    expect(Genius.lyricsToSong("T", "A", "12 Contributors\n\nEmbed").slides).toEqual([]);
  });

  test("pageToSong combines the lyrics and the CCLI number", () => {
    const html = `<div data-lyrics-container="true">[Verse 1]<br/>Hello</div><p>CCLI #7115744</p>`;
    const song = Genius.pageToSong(html, "Hello", "Me");
    expect(song.ccli_number).toBe("7115744");
    expect(song.slides.length).toBe(1);
    expect(Genius.pageToSong("<html></html>", undefined, undefined).title).toBe("Worship Song");
  });
});

describe("Lyrics plugin: LRCLIB", () => {
  const row = (over: any) => ({ id: 1, trackName: "So Much", artistName: "Life.Church Worship", albumName: "The Way",
    duration: 203, instrumental: false, plainLyrics: "Line a\nLine b", syncedLyrics: "[00:01.00] Line a", ...over });

  test("builds the search URL", () => {
    expect(Genius.lrclibSearchUrl("So Much Life.Church")).toBe("https://lrclib.net/api/search?q=So%20Much%20Life.Church");
    expect(Genius.lrclibSearchUrl("  ")).toBe("");
  });

  test("keeps real matches with words, one per title/artist/album", () => {
    const body = JSON.stringify([
      row({ id: 1 }), row({ id: 2 }), // the same recording twice
      row({ id: 3, albumName: "A Thousand Blessings", duration: 208 }),
      row({ id: 4, instrumental: true, plainLyrics: "" }),
      row({ id: 5, plainLyrics: "" }),
      row({ id: 6, trackName: "Unrelated", artistName: "Someone" }),
      null,
    ]);
    const hits = Genius.parseLrclibResponse(body, "so much life.church");
    expect(hits.map((h: any) => h.id)).toEqual(["lrclib_1", "lrclib_3"]);
    expect(hits[0]).toMatchObject({ source: "lrclib", title: "So Much", artist: "Life.Church Worship", album: "The Way", length: "3:23" });
    expect(Genius.parseLrclibResponse("not json", "x")).toEqual([]);
    expect(Genius.parseLrclibResponse("{}", "x")).toEqual([]);
  });

  test("plain lyrics become slides in singing order, with repeated stanzas named as the chorus", () => {
    const plain = "V one a\nV one b\n\nChorus a\nChorus b\n\nV two a\nV two b\n\nChorus a\nchorus  b\n\nBridge a\nBridge b";
    const song = Genius.plainLyricsToSong("So Much", "Life.Church Worship", "The Way", plain);
    expect(song.slides.map((s: any) => s.label)).toEqual(["Verse 1", "Chorus 1", "Verse 2", "Chorus 1", "Verse 3"]);
    expect(song.slides[1].text).toBe("Chorus a\nChorus b");
    expect(song).toMatchObject({ title: "So Much", author: "Life.Church Worship", ccli_number: null });
    expect(song.copyright).toContain("LRCLIB");
    expect(Genius.plainLyricsToSong("T", "A", "", "").slides).toEqual([]);
  });
});
