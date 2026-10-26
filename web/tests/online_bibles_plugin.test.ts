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

  test("the catalog is our own public-domain Bibles, English first, then by language and name, without duplicates", () => {
    const entries = [
      { id: "kjv", abbreviation: "KJV", name: "King James Version", language: "eng", sourceKey: "https://x/kjv.json" },
      { id: "kjv2", abbreviation: "kjv", name: "A duplicate", language: "English", sourceKey: "https://x/dup.json" },
      { id: "zz", abbreviation: "ZZZ", name: "Zed", language: "Klingon", sourceKey: "https://x/zzz.json" },
      { id: "lsg", abbreviation: "LSG", name: "Louis Segond", language: "fra", sourceKey: "https://x/lsg.json" },
      { id: "nokey", abbreviation: "NOKEY", name: "No download address", language: "eng" },
    ];
    const cat = Bibles.buildCatalog(entries);
    expect(cat.filter((e: any) => e.abbreviation === "KJV").length).toBe(1);
    expect(cat.find((e: any) => e.abbreviation === "KJV").name).toBe("King James Version");
    expect(cat.find((e: any) => e.abbreviation === "NOKEY")).toBeUndefined(); // nothing to download from: not listed
    expect(cat.every((e: any) => e.provider === "craterbrain" && e.source === "OpenSanctuary Bibles \u00b7 public domain")).toBe(true);
    // English first, then other languages alphabetically.
    expect(cat.map((e: any) => e.language)).toEqual(["English", "French", "Klingon"]);
    expect(Bibles.buildCatalog(undefined)).toEqual([]); // offline: nothing, not an error
    expect(Bibles.buildCatalog([])).toEqual([]);
  });

  test("survives missing or malformed sources", () => {
    expect(Bibles.buildCatalog(null)).toEqual([]);
    expect(Bibles.buildCatalog("nope")).toEqual([]);
    expect(Bibles.buildCatalog([{}, null, 5, { translations: [{}, null] }])).toEqual([]);
  });
});

describe("Online Bibles plugin: the OpenSanctuary Bibles repository", () => {
  const OWN = "https://raw.githubusercontent.com/Craterbrain/opensanctuary-bibles/main/";
  const catalogJson = JSON.stringify({ format: 1, bibles: [
    { id: "eng-kjv2006", abbreviation: "KJV", name: "King James Version (Authorized Version)", language: "English", file: "bibles/eng-kjv2006.json" },
    { id: "fraLSG", abbreviation: "LSG", name: "Louis Segond (1910)", language: "French", file: "bibles/fraLSG.json" },
    { id: "evil", abbreviation: "EVIL", name: "Not from our folder", language: "English", file: "../../evil/x.json" },
    { id: "bad", abbreviation: "bad abbr!", name: "Bad abbreviation", language: "English", file: "bibles/bad.json" },
    { id: "nameless", abbreviation: "NONAME", language: "English", file: "bibles/nameless.json" },
    null, "not an object"] });

  test("reads catalog.json, and never accepts an address outside its bibles/ folder", () => {
    const entries = Bibles.ownCatalogEntries(catalogJson);
    expect(entries.map((e: any) => e.abbreviation)).toEqual(["KJV", "LSG"]);
    expect(entries[0].sourceKey).toBe(OWN + "bibles/eng-kjv2006.json");
    expect(Bibles.ownCatalogEntries("not json")).toEqual([]);
    expect(Bibles.ownCatalogEntries(JSON.stringify({ bibles: "nope" }))).toEqual([]);
    expect(Bibles.OWN_CATALOG_URL).toBe(OWN + "catalog.json");
  });

  test("only files in our repository's bibles/ folder are ours to download", () => {
    expect(Bibles.isOwnBibleUrl(OWN + "bibles/engwebp.json")).toBe(true);
    for (const bad of [OWN + "bibles/../x.json", OWN + "bibles/sub/x.json", OWN + "catalog.json", OWN + "bibles/x.txt",
      "https://raw.githubusercontent.com/someone-else/opensanctuary-bibles/main/bibles/a.json", "http://raw.githubusercontent.com/Craterbrain/opensanctuary-bibles/main/bibles/a.json"]) {
      expect(Bibles.isOwnBibleUrl(bad)).toBe(false);
    }
  });

  test("our entries are downloaded from our repository, English first", () => {
    const cat = Bibles.buildCatalog(Bibles.ownCatalogEntries(catalogJson));
    const kjv = cat.filter((e: any) => e.abbreviation === "KJV" && e.language === "English");
    expect(kjv.length).toBe(1);
    expect(kjv[0].provider).toBe("craterbrain");
    expect(kjv[0].sourceKey).toBe(OWN + "bibles/eng-kjv2006.json");
    expect(kjv[0].name).toBe("King James Version (Authorized Version)");
    expect(kjv[0].source).toBe("OpenSanctuary Bibles \u00b7 public domain");
    expect(cat[0].language).toBe("English");
    expect(cat.find((e: any) => e.abbreviation === "LSG")?.provider).toBe("craterbrain");
  });

  test("our Bible files parse with the translation's own abbreviation as the version", () => {
    const body = JSON.stringify({ John: { "3": { "16": "For God so loved the world", "17": "For God sent not his Son" } }, Tobit: { "1": { "1": "The book of the words of Tobias" } } });
    const items = Bibles.parseGithubBible(body, OWN + "bibles/eng-kjv2006.json", "kjv");
    expect(items.map((i: any) => `${i.book} ${i.chapter} ${i.version}`)).toEqual(["John 3 KJV", "Tobit 1 KJV"]);
    expect(items[0].verses.map((v: any) => v.verse_number)).toEqual([16, 17]);
    // Without one it is still derived from the file name.
    expect(Bibles.parseGithubBible(body, "https://raw.githubusercontent.com/o/r/main/versions/en/KJV.json")[0].version).toBe("KJV");
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

  test("abbreviations and titles from file names", () => {
    expect(Bibles.makeAbbreviation("KING JAMES BIBLE")).toBe("KJB");
    expect(Bibles.makeAbbreviation("WORLD ENGLISH BIBLE")).toBe("WEB");
    expect(Bibles.makeAbbreviation("KJV")).toBe("KJV");
    expect(Bibles.titleCase("KING JAMES BIBLE")).toBe("King James Bible");
    // The GitHub-repository browser is gone (Import > Bible from a Link or File replaced it).
    for (const gone of ["parseRepo", "githubContentsUrl", "githubCatalogEntries", "isRawGithubUrl"]) expect((Bibles as any)[gone]).toBeUndefined();
  });

  test("nested-JSON Bible (our own format) -> chapters, sorted numerically", () => {
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
