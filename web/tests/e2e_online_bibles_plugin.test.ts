import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer, expectNativeIcons, pluginBridge } from "./e2e_helpers";

describe("E2E Live Test: Online Bibles plugin replaces the built-in Bible downloader", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9075;
  let artifactDir: string;

  beforeAll(async () => {
    // Bibles are stored per translation in the Bibles directory (default ./bibles in the repo): use our own.
    const biblesDir = mkdtempSync(join(tmpdir(), "os-next-bibles-"));
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-bibles-plugin-e2e-", extraArgs: ["--bibles-dir", biblesDir] });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const frameOf = () => page.frames().find((f) => f.url().includes("/plugin-frame/online_bibles/"));
  const bridge = pluginBridge(frameOf);

  test("the built-in downloader is gone; the plugin installs a Bible and stages a passage", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    for (const gone of ["/api/bibles/online/catalog", "/api/bibles/github/catalog"]) {
      expect((await fetch(`http://127.0.0.1:${PORT}${gone}`)).status).toBe(404);
    }
    expect((await fetch(`http://127.0.0.1:${PORT}/api/bibles/online/download`, { method: "POST" })).status).not.toBe(200);
    // The Import dialog no longer has Bible tabs.
    expect(await page.locator("#tab-btn-api, #tab-btn-github").count()).toBe(0);

    // Turn the plugin on from Settings > Plugins: its panel appears without a reload.
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="online_bibles"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    const text = await card.innerText();
    for (const host of ["bolls.life", "api.churchapps.org", "bible-api.com", "api.github.com", "raw.githubusercontent.com"]) {
      expect(text).toContain(host);
    }
    await page.screenshot({ path: `${artifactDir}/screenshot_online_bibles_review.png` });
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="online_bibles"]'), null, { timeout: 15000 });
    await page.keyboard.press("Escape");
    // It lives in the Scriptures library sidebar (where "Install Bibles" used to be).
    await page.click('.tab-btn[data-tab="scriptures"]');
    const node = page.locator('.category-node:has-text("Online Bibles")');
    await node.waitFor({ state: "visible", timeout: 10000 });
    await node.click();
    const frame = frameOf()!;
    await frame.waitForSelector("#catalog-list li", { state: "visible", timeout: 40000 }); // the curated list at least
    await expectNativeIcons(frame, 3);
    await page.screenshot({ path: `${artifactDir}/screenshot_online_bibles_panel.png` });
    await page.screenshot({ path: `../docs/screenshots/online-bibles-plugin.png` });

    // Install a translation through the host API, the way the plugin's Download button does after parsing.
    const added = await bridge("library.addScriptures", { items: [
      { book: "John", chapter: 3, version: "plug", verses: [{ verse_number: 16, text: "For God so loved the world" }] }] });
    expect(added.error ?? "").toBe("");
    expect(added.ok).toBe(true);
    const bibles = await (await fetch(`http://127.0.0.1:${PORT}/api/bibles`)).json();
    expect(bibles.map((b: any) => b.id)).toContain("PLUG");
    // ...and it can add it to the schedule, since the optional command permission was approved.
    const staged = await bridge("sendCommand", { command: { AddToSchedule: { item_type: "scripture", item_id: added.result.ids[0] } } });
    expect(staged.ok).toBe(true);
    const state = await (await fetch(`http://127.0.0.1:${PORT}/api/state`)).json();
    expect(JSON.stringify(state.schedule.items)).toContain("John 3");

    // Outside its grant: refused.
    const refused = await bridge("net.fetch", { url: "https://example.com/" });
    expect(refused.ok).toBe(false);
  }, 150000);
});
