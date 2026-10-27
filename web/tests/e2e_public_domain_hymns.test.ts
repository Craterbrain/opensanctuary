import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

// Import dialog > "Public-domain hymns": the bundled hymns are added once, to the real song library, and never twice.
describe("E2E: public-domain hymn library", () => {
  const PORT = 9089;
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-pd-hymns-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 60000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  const songs = async (): Promise<any[]> => (await fetch(`http://127.0.0.1:${PORT}/api/songs`)).json();

  test("one click adds the hymns, a second visit offers nothing more", async () => {
    // A test server starts with an empty song library (the first-run seeding is off in tests).
    const before = (await songs()).length;
    await page.evaluate(() => (window as any).openImportModal());
    await page.waitForSelector("#import-modal", { state: "visible", timeout: 8000 });
    await page.waitForFunction(() => /hymns/.test(document.getElementById("public-domain-state")?.textContent || ""), null, { timeout: 8000 });
    expect(await page.locator("#btn-add-public-domain").isDisabled()).toBe(false);

    await page.click("#btn-add-public-domain");
    await page.waitForFunction(() => /^Added \d+ hymns/.test(document.getElementById("public-domain-status")?.textContent || ""), null, { timeout: 20000 });
    const after = await songs();
    expect(after.length).toBeGreaterThan(before + 150);
    expect(after.some((s) => s.title === "Amazing Grace" && s.copyright === "Public Domain")).toBe(true);
    // Nothing copyrighted came with them.
    expect(after.some((s) => /here i am to worship/i.test(s.title))).toBe(false);

    await page.waitForFunction(() => /already in your song library/.test(document.getElementById("public-domain-state")?.textContent || ""), null, { timeout: 8000 });
    expect(await page.locator("#btn-add-public-domain").isDisabled()).toBe(true);
    expect((await songs()).length).toBe(after.length);
  }, 60000);
});
