import { test, expect, describe } from "bun:test";
// The plugin's pure parsing module (CommonJS, also loaded as a plain <script> in the sandbox).
// @ts-ignore
import Bibles from "../plugins/online_bibles/bibles.js";

describe("Online Bibles plugin: languages and catalog", () => {
  test("normalizes language names and codes the way the old importer did", () => {
    expect(Bibles.normalizeLanguage("eng")).toBe("English");
    expect(Bibles.normalizeLanguage("Spanish")).toBe("Spanish");
    expect(Bibles.normalizeLanguage("fra")).toBe("French");
    expect(Bibles.normalizeLanguage("en")).toBe("English");
    expect(Bibles.normalizeLanguage("pt")).toBe("Portuguese");
    expect(Bibles.normalizeLanguage("zho")).toBe("Chinese");
    expect(Bibles.normalizeLanguage("")).toBe("English");
    expect(Bibles.normalizeLanguage(undefined)).toBe("English");
    expect(Bibles.normalizeLanguage("Swahili")).toBe("Swahili"); // unknown: kept, capitalised
    expect(Bibles.normalizeLanguage("swahili")).toBe("Swahili");
  });

  test("merges the curated list, Bolls.life and ChurchApps without duplicates", () => {
    const bolls = [{ language: "English", translations: [
      { short_name: "KJV", full_name: "King James Version with Strong's numbers" },
      { short_name: "NKJV", full_name: "NKJV" },
      { short_name: "BBE", full_name: "Bible in Basic English" }] },
      { language: "Español", translations: [{ short_name: "RVR1960", full_name: "Reina Valera 1960" }] }];
    const churchApps = [{ id: "kjv", abbreviation: "KJV", name: "KJV", language: "eng" }, { id: "zz", abbreviation: "ZZZ", name: "Zed", language: "Klingon" }];
    const cat = Bibles.buildCatalog(bolls, churchApps);
    const kjv = cat.filter((e: any) => e.abbreviation === "KJV" && e.language === "English");
    expect(kjv.length).toBe(1);
    expect(kjv[0].source).toBe("OpenLP Community • Bolls.life • ChurchApps");
    expect(kjv[0].name).toBe("King James Version with Strong's numbers"); // the longer, more informative title wins
    expect(cat.find((e: any) => e.abbreviation === "BBE")).toBeDefined();
    // English first, then other languages alphabetically.
    const langs = cat.map((e: any) => e.language);
    const firstNonEnglish = langs.findIndex((l: string) => l !== "English");
    expect(langs.slice(0, firstNonEnglish).every((l: string) => l === "English")).toBe(true);
    expect(langs.slice(firstNonEnglish).includes("English")).toBe(false);
    expect(cat.find((e: any) => e.abbreviation === "ZZZ")?.language).toBe("Klingon");
  });

  test("survives missing or malformed sources", () => {
    expect(Bibles.buildCatalog(null, undefined).length).toBe(Bibles.CURATED.length);
    expect(Bibles.buildCatalog([{}, { translations: [{}, null] }], [null, 5, {}]).length).toBe(Bibles.CURATED.length);
  });
});

describe("Online Bibles plugin: downloads", () => {
  test("knows the canonical books", () => {
    expect(Bibles.BOOKS.length).toBe(66);
    expect(Bibles.BOOKS[0]).toEqual(["Genesis", 50]);
    expect(Bibles.BOOKS[18]).toEqual(["Psalms", 150]);
    expect(Bibles.BOOKS[65]).toEqual(["Revelation", 22]);
    const chapters = Bibles.BOOKS.reduce((n: number, b: any) => n + b[1], 0);
    expect(chapters).toBe(1189);
  });

  test("Bolls translation -> chapters in book order, with markup removed", () => {
    const rows = [
      { book: 43, chapter: 3, verse: 16, text: "For God so <S>25</S>loved the <i>world</i>" },
      { book: 1, chapter: 1, verse: 2, text: "And the earth was without form" },
      { book: 1, chapter: 1, verse: 1, text: "In the beginning<br>God created" },
      { book: 99, chapter: 1, verse: 1, text: "not a real book" },
    ];
    const chapters = Bibles.parseBollsTranslation(JSON.stringify(rows), " kjv ");
    expect(chapters.map((c: any) => `${c.book} ${c.chapter}`)).toEqual(["Genesis 1", "John 3"]);
    expect(chapters[0].version).toBe("KJV");
    // Verses end up in order whatever order the file listed them in, and a <br> becomes a space.
    expect(chapters[0].verses.map((v: any) => v.text)).toEqual(["In the beginning God created", "And the earth was without form"]);
    expect(chapters[1].verses[0].text).toBe("For God so 25loved the world");
    expect(Bibles.parseBollsTranslation("nope", "kjv")).toEqual([]);
    expect(Bibles.parseBollsTranslation("{}", "kjv")).toEqual([]);
  });

  test("GitHub repo names and listings", () => {
    expect(Bibles.parseRepo("arron-taylor/bible-versions")).toEqual({ owner: "arron-taylor", repo: "bible-versions" });
    expect(Bibles.parseRepo("/a/b/")).toEqual({ owner: "a", repo: "b" });
    for (const bad of ["nope", "a/b/c", "a/", "/", "a b/c", "../x"]) expect(Bibles.parseRepo(bad)).toBeNull();
    expect(Bibles.githubContentsUrl({ owner: "o", repo: "r" }, "versions/en")).toBe("https://api.github.com/repos/o/r/contents/versions/en");
    expect(Bibles.githubContentsUrl({ owner: "o", repo: "r" }, "../../etc/passwd")).toBe("https://api.github.com/repos/o/r/contents/etc/passwd");
    expect(Bibles.makeAbbreviation("KING JAMES BIBLE")).toBe("KJB");
    expect(Bibles.makeAbbreviation("WORLD ENGLISH BIBLE")).toBe("WEB");
    expect(Bibles.makeAbbreviation("KJV")).toBe("KJV");
    expect(Bibles.titleCase("KING JAMES BIBLE")).toBe("King James Bible");
    const entries = Bibles.githubCatalogEntries({ owner: "o", repo: "r" }, "en", [
      { type: "file", name: "KING JAMES BIBLE.json", path: "versions/en/KING JAMES BIBLE.json", download_url: "https://raw.githubusercontent.com/o/r/main/versions/en/KING%20JAMES%20BIBLE.json" },
      { type: "file", name: "README.md", path: "x" }, { type: "dir", name: "sub", path: "y" }]);
    expect(entries.length).toBe(1);
    expect(entries[0]).toMatchObject({ id: "gh_en_kjb", name: "King James Bible", language: "English", abbreviation: "KJB", provider: "github" });
    expect(Bibles.isRawGithubUrl(entries[0].sourceKey)).toBe(true);
    expect(Bibles.isRawGithubUrl("https://evil.test/raw.githubusercontent.com/")).toBe(false);
    expect(Bibles.isRawGithubUrl("http://raw.githubusercontent.com/x")).toBe(false);
  });

  test("GitHub nested-JSON Bible -> chapters, sorted numerically", () => {
    const bible = { Genesis: { "10": { "1": "ten", "2": "ten-two" }, "2": { "10": "b", "9": "a" }, bad: "x" }, Junk: 5 };
    const chapters = Bibles.parseGithubBible(JSON.stringify(bible), "https://raw.githubusercontent.com/o/r/main/versions/en/KING%20JAMES%20BIBLE.json");
    expect(chapters.map((c: any) => c.chapter)).toEqual([2, 10]);
    expect(chapters[0].version).toBe("KJB");
    expect(chapters[0].verses.map((v: any) => v.verse_number)).toEqual([9, 10]);
    expect(Bibles.parseGithubBible("[]", "u")).toEqual([]);
    expect(Bibles.parseGithubBible("nope", "u")).toEqual([]);
  });

  test("batches", () => {
    expect(Bibles.chunk([1, 2, 3, 4, 5], 2)).toEqual([[1, 2], [3, 4], [5]]);
    expect(Bibles.chunk([], 3)).toEqual([]);
  });
});

describe("Online Bibles plugin: passages", () => {
  test("recognises a whole-book query", () => {
    expect(Bibles.exactBook("Jude")).toEqual(["Jude", 1]);
    expect(Bibles.exactBook("  psalms ")).toEqual(["Psalms", 150]);
    expect(Bibles.exactBook("Psalm")[0]).toBe("Psalms");
    expect(Bibles.exactBook("Revelations")[0]).toBe("Revelation");
    expect(Bibles.exactBook("John 3:16")).toBeNull();
    expect(Bibles.exactBook("Genesis 1")).toBeNull();
  });

  test("builds bible-api URLs, with explicit ranges for one-chapter books", () => {
    expect(Bibles.chapterUrl("1 Samuel", 3, "KJV")).toBe("https://bible-api.com/1+Samuel+3?translation=kjv");
    expect(Bibles.chapterUrl("Jude", 1, "web")).toBe("https://bible-api.com/Jude+1:1-25?translation=web");
    expect(Bibles.chapterUrl("2 John", 1, "web")).toBe("https://bible-api.com/2+John+1:1-13?translation=web");
    expect(Bibles.passageUrl("John 3:16", "KJV")).toBe("https://bible-api.com/John%203%3A16?translation=kjv");
  });

  test("bible-api response -> one chapter-shaped item", () => {
    const body = JSON.stringify({ reference: "John 3:16", translation_id: "kjv", verses: [{ book_name: "John", chapter: 3, verse: 16, text: " For God so loved the world.\n" }] });
    expect(Bibles.parseBibleApi(body, "web")).toEqual({ book: "John", chapter: 3, version: "KJV", verses: [{ verse_number: 16, text: "For God so loved the world." }] });
    expect(Bibles.parseBibleApi(JSON.stringify({ verses: [] }), "kjv")).toBeNull();
    expect(Bibles.parseBibleApi("nope", "kjv")).toBeNull();
  });
});
