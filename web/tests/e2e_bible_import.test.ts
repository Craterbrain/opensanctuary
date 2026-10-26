import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

// Import dialog > "Bible from a Link or File": a Bible file is picked, read by the server (USFM, USX, OSIS, Zefania, OpenSong; zip)
// and appears in the Scriptures sidebar. (The link box needs the internet; its refusals are covered by the
// server tests, and the same code path runs for a file.)
describe("E2E: Bible from a file", () => {
  const PORT = 9087;
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  let dir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-bible-import-" });
    dir = mkdtempSync(join(tmpdir(), "os-bible-files-"));
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 60000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  const openImport = async () => {
    await page.evaluate(() => (window as any).openImportModal());
    await page.waitForSelector("#import-modal", { state: "visible", timeout: 8000 });
  };
  const status = () => page.locator("#bible-link-status");
  const pick = async (name: string, text: string) => {
    const path = join(dir, name);
    writeFileSync(path, text);
    await page.setInputFiles("#file-bible-import", path);
  };

  test("a USFM file is read and its Bible shows in the sidebar; an OSIS file too", async () => {
    await openImport();
    await page.fill("#bible-link-abbr", "tstb");
    await pick("test.usfm", "\\id JHN\n\\c 3\n\\p\n\\v 16 For God so loved the world.\n\\v 17 For God did not send his Son.\n");
    await status().waitFor({ state: "visible", timeout: 10000 });
    await page.waitForFunction(() => /Installed TSTB/.test(document.getElementById("bible-link-status")?.textContent || ""), null, { timeout: 15000 });
    expect(await status().innerText()).toContain("1 chapters, 2 verses (USFM)");
    const list: any[] = await (await fetch(`http://127.0.0.1:${PORT}/api/bibles`)).json();
    expect(list.some((b) => b.id === "TSTB")).toBe(true);

    await page.fill("#bible-link-abbr", "");
    await pick("o.xml", `<osis><osisText osisIDWork="OSB"><div type="book" osisID="Gen"><chapter osisID="Gen.1"><verse osisID="Gen.1.1">In the beginning.</verse></chapter></div></osisText></osis>`);
    await page.waitForFunction(() => /Installed OSB/.test(document.getElementById("bible-link-status")?.textContent || ""), null, { timeout: 15000 });
    expect(await status().innerText()).toContain("OSIS XML");
    await page.keyboard.press("Escape");
    await page.click('.tab-btn[data-tab="scriptures"]');
    await page.waitForSelector('.bible-tree-node[data-bible-id="TSTB"]', { timeout: 10000 });
    await page.waitForSelector('.bible-tree-node[data-bible-id="OSB"]', { timeout: 10000 });
  }, 60000);

  test("importing over an installed Bible asks first; saying no leaves it alone", async () => {
    await openImport();
    await page.fill("#bible-link-abbr", "TSTB");
    await pick("other.usfm", "\\id GEN\n\\c 1\n\\p\n\\v 1 In the beginning.\n");
    await page.waitForSelector("#confirm-modal", { state: "visible", timeout: 10000 });
    expect(await page.locator("#confirm-modal").innerText()).toContain("TSTB is already installed");
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => /Left the installed Bible/.test(document.getElementById("bible-link-status")?.textContent || ""), null, { timeout: 10000 });
    const items: any[] = await (await fetch(`http://127.0.0.1:${PORT}/api/scriptures`)).json();
    expect(items.filter((i) => i.version === "TSTB").map((i) => i.book)).toEqual(["John"]);

    // Yes: it is replaced.
    await openImport();
    await page.fill("#bible-link-abbr", "TSTB");
    await pick("other2.usfm", "\\id GEN\n\\c 1\n\\p\n\\v 1 In the beginning.\n");
    await page.waitForSelector("#confirm-modal", { state: "visible", timeout: 10000 });
    await page.locator("#confirm-modal button:has-text('Replace')").click();
    await page.waitForFunction(() => /Installed TSTB/.test(document.getElementById("bible-link-status")?.textContent || ""), null, { timeout: 15000 });
    const after: any[] = await (await fetch(`http://127.0.0.1:${PORT}/api/scriptures`)).json();
    expect(after.filter((i) => i.version === "TSTB").map((i) => i.book)).toEqual(["Genesis"]);
  }, 60000);

  test("a file that is not a Bible says so", async () => {
    await openImport();
    await page.fill("#bible-link-abbr", "");
    await pick("notes.txt", "Remember to buy milk.");
    await page.waitForFunction(() => /couldn.t recognise/.test(document.getElementById("bible-link-status")?.textContent || ""), null, { timeout: 10000 });
  }, 30000);
});
