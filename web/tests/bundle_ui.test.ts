import { describe, test, expect } from "bun:test";
import { advisoryApplies, bundleKindForFile, copyrightLine, filenameFromDisposition, importMessage, BUNDLE_ACCEPT, type ImportResult } from "../src/ui/bundle_ui";

const result = (over: Partial<ImportResult> = {}): ImportResult => ({
  kind: null, added: 0, skipped_identical: 0, added_as_copies: 0, media_written: 0, media_reused: 0, themes_added: [], bible: null, warnings: [], ...over,
});

describe("OpenSanctuary file types", () => {
  test("each extension maps to its kind, in any case, and other files to none", () => {
    expect(bundleKindForFile("Amazing Grace.ossong")).toBe("song");
    expect(bundleKindForFile("Easter.OSSERVICE")).toBe("service");
    expect(bundleKindForFile("web.osbible")).toBe("bible");
    expect(bundleKindForFile("notices.ospres")).toBe("presentation");
    // The ones other programs own were deliberately not taken.
    for (const taken of ["a.osp", "a.osb", "a.osz", "a.oss", "a.ewsx", "a.zip", "noextension", ""]) expect(bundleKindForFile(taken)).toBeNull();
  });

  test("the file picker accepts exactly the four", () => {
    expect(BUNDLE_ACCEPT.split(",").sort()).toEqual([".osbible", ".ospres", ".osservice", ".ossong"]);
  });
});

describe("the copyright advisory", () => {
  test("a line names the song, its author, and what it says about its rights", () => {
    expect(copyrightLine({ title: "10,000 Reasons", author: "M. Redman", copyright: "© 2011 Worship Co", ccli_number: "6016351" })).toBe("10,000 Reasons (M. Redman) — © 2011 Worship Co · CCLI #6016351");
    expect(copyrightLine({ title: "Only CCLI", author: "", copyright: "", ccli_number: " 42 " })).toBe("Only CCLI — CCLI #42");
    expect(copyrightLine({ title: "Bare", author: "", copyright: "", ccli_number: "" })).toBe("Bare");
  });

  test("it applies only when copyrighted songs would actually be added", () => {
    const note = { title: "t", author: "", copyright: "c", ccli_number: "" };
    expect(advisoryApplies({ copyrighted: [], library: { new: 3, identical: 0, different: 0 } })).toBe(false);
    expect(advisoryApplies({ copyrighted: [note], library: { new: 1, identical: 0, different: 0 } })).toBe(true);
    expect(advisoryApplies({ copyrighted: [note], library: { new: 0, identical: 0, different: 1 } })).toBe(true);
    expect(advisoryApplies({ copyrighted: [note], library: { new: 0, identical: 2, different: 0 } })).toBe(false); // all already here
    expect(advisoryApplies({ copyrighted: [note], library: null })).toBe(true); // a service has no per-item status
  });
});

describe("what the user is told", () => {
  test("songs and presentations say what was added, skipped and copied", () => {
    expect(importMessage("song", "x", result({ added: 1 }))).toBe("✓ Imported 1 song");
    expect(importMessage("song", "x", result({ added: 2, skipped_identical: 1 }))).toBe("✓ Imported 2 songs (1 already in your library)");
    expect(importMessage("song", "x", result({ added_as_copies: 1 }))).toContain("1 added as a copy because yours differ");
    expect(importMessage("song", "x", result({ skipped_identical: 3 }))).toBe("✓ Nothing new: 3 songs are already in your library");
    expect(importMessage("presentation", "x", result({ skipped_identical: 1 }))).toBe("✓ Nothing new: 1 presentation is already in your library");
  });
  test("services and Bibles name themselves", () => {
    expect(importMessage("service", "Easter", result({ added: 5, themes_added: ["Advent"] }))).toBe('✓ Opened service "Easter" (5 items), 1 theme added');
    expect(importMessage("presentation", "Deck", result({ added: 1, backgrounds_added: 17 }))).toContain("17 pictures added to Media as backgrounds");
    expect(importMessage("bible", "World English Bible", result({ added: 1189, bible: "WEB" }))).toBe("✓ Installed WEB (1189 chapters)");
  });
});

describe("saved file names", () => {
  test("the server's name wins, with its non-ASCII form preferred", () => {
    expect(filenameFromDisposition(`attachment; filename="Gracia ___.ossong"; filename*=UTF-8''Gracia%20%E2%9C%9D.ossong`, "x.ossong")).toBe("Gracia ✝.ossong");
    expect(filenameFromDisposition(`attachment; filename="plain.osbible"`, "x.osbible")).toBe("plain.osbible");
    expect(filenameFromDisposition(null, "fallback.ospres")).toBe("fallback.ospres");
    expect(filenameFromDisposition(`attachment; filename*=UTF-8''%E0%A4%A`, "fallback.ospres")).toBe("fallback.ospres"); // malformed escape
  });
});
