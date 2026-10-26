import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { existsSync } from "fs";
import { join } from "path";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

// Import dialog > "EasyWorship Presentations": a real announcement deck (not committed; $OS_TEST_DOCS) joins the Presentations
// library with a readable background on every slide. Skips when the deck isn't on this machine (docs/workflow.md).
const docs = process.env.OS_TEST_DOCS || join(process.env.HOME || "", "Documents/OpenSanctuary-test-docs");
const deck = join(docs, "LCspring.ewpx");

describe.skipIf(!existsSync(deck))("E2E: EasyWorship presentation import", () => {
  const PORT = 9088;
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-ewpx-import-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 60000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  test("the deck is added to the Presentations library and its slides have a background", async () => {
    await page.evaluate(() => (window as any).openImportModal());
    await page.waitForSelector("#import-modal", { state: "visible", timeout: 8000 });
    // The theme list is the presentation themes plus the default.
    await page.locator("#ewpx-theme").focus();
    await page.waitForFunction(() => (document.querySelectorAll("#ewpx-theme option").length > 1), null, { timeout: 8000 });
    await page.setInputFiles("#file-ewpx-import", deck);
    await page.waitForFunction(() => /Imported .LCspring./.test(document.getElementById("ewpx-import-status")?.textContent || ""), null, { timeout: 20000 });
    expect(await page.locator("#ewpx-import-status").innerText()).toContain("theme background");

    const pres: any[] = await (await fetch(`http://127.0.0.1:${PORT}/api/presentations`)).json();
    expect(pres.length).toBe(1);
    expect(pres[0].title).toBe("LCspring");
    expect(pres[0].slides.length).toBeGreaterThan(10);
    expect(pres[0].slides.every((s: any) => !s.background)).toBe(true);
    expect(pres[0].theme_name).toBe("Presentation Dark");

    await page.keyboard.press("Escape");
    await page.click('.tab-btn[data-tab="presentations"]');
    await page.waitForFunction(() => /LCspring/.test(document.body.innerText), null, { timeout: 10000 });
  }, 60000);
});
