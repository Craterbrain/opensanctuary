import { test, expect, describe } from "bun:test";
import { installedMessage, looksLikeLink } from "../src/ui/bible_import_ui";

describe("Bible import UI helpers", () => {
  test("the confirmation after an install", () => {
    expect(installedMessage({ abbreviation: "WEB", name: "World English Bible", format: "USFM", chapters: 1189, verses: 31102 }))
      .toBe("Installed WEB (World English Bible): 1,189 chapters, 31,102 verses (USFM).");
    expect(installedMessage({ abbreviation: "X", name: "X", format: "OSIS XML", chapters: 1, verses: 2 })).toBe("Installed X: 1 chapters, 2 verses (OSIS XML).");
  });
  test("only https links with a host name are sent", () => {
    expect(looksLikeLink("https://ebible.org/Scriptures/eng-web_usfm.zip")).toBe(true);
    expect(looksLikeLink("  https://raw.githubusercontent.com/o/r/main/b.zip ")).toBe(true);
    for (const bad of ["http://ebible.org/a.zip", "ebible.org/a.zip", "https://localhost/a.zip", "file:///etc/passwd", "", "javascript:alert(1)"]) {
      expect(looksLikeLink(bad)).toBe(false);
    }
  });
});
