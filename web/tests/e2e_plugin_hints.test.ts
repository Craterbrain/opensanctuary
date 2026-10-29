import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, webDirWithPlugins, ensureArtifactDir, nextFrame, type SpawnedTestServer } from "./e2e_helpers";

// The console never names a plugin: what it shows for one comes from that plugin's manifest
// (core/plugin_hints.ts). So with the bundled plugins installed but not approved, each declared
// placement shows a "not turned on" hint that opens its review card; with none installed -- what
// v0.2.0's packages actually shipped -- nothing plugin-related appears at all.

/** Resolves when the console has fetched the installed-plugin list (what every hint is built from) and used it. */
async function pluginListLoaded(page: Page, during: () => Promise<unknown>): Promise<void> {
  const listed = page.waitForResponse((r) => new URL(r.url()).pathname === "/api/plugins" && r.request().method() === "GET");
  await during();
  await (await listed).finished();
  await nextFrame(page);
}

async function openConsole(browser: Browser, port: number): Promise<Page> {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await pluginListLoaded(page, () => page.goto(`http://127.0.0.1:${port}/`, { waitUntil: "networkidle" }));
  return page;
}

const hintNodes = (page: Page) => page.locator(".category-node.plugin-source-node--off");

/** Opens the first-time setup from Settings > About and skips forward to `steps` steps in. */
async function openSetupAt(page: Page, steps: number): Promise<void> {
  await page.keyboard.press("F2");
  await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
  await page.locator(".settings-category-btn:has-text('About')").click();
  await page.locator("button:has-text('Re-run First-Time Setup')").click();
  await page.waitForSelector("#first-time-setup-body", { state: "visible", timeout: 5000 });
  await page.click("#btn-first-time-next"); // "Let's Go" off the welcome step (it has no Skip)
  for (let i = 1; i < steps; i++) await page.click("#btn-first-time-skip");
}
// The Store button is hidden for now (docs/TODO.md); the dialog still works, so the tests open it directly.
async function openStore(page: Page): Promise<void> {
  expect(await page.locator("#btn-store").isHidden()).toBe(true);
  await page.evaluate(() => (document.getElementById("btn-store") as HTMLElement).click());
}
const BIBLES_STEP = 3; // welcome, data-dir, church-name, bibles

describe("E2E Live Test: plugin hints come from the installed plugins' manifests", () => {
  let browser: Browser;
  let withPlugins: SpawnedTestServer;
  let withoutPlugins: SpawnedTestServer;
  const PORT_WITH = 9088;
  const PORT_WITHOUT = 9089;
  let artifactDir: string;

  beforeAll(async () => {
    withPlugins = await spawnTestServer({ port: PORT_WITH, tempPrefix: "os-next-hints-with-" });
    withoutPlugins = await spawnTestServer({ port: PORT_WITHOUT, tempPrefix: "os-next-hints-without-", webDir: webDirWithPlugins({}, { bundled: false }) });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
  }, 60000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(withPlugins);
    await teardownTestServer(withoutPlugins);
  });

  test("first-time setup offers the installed plugins that add Bibles and songs, from their manifests", async () => {
    const page = await openConsole(browser, PORT_WITH);
    await openSetupAt(page, BIBLES_STEP);
    const bibles = page.locator('.fts-plugin-offer[data-plugin-id="online_bibles"]');
    await bibles.waitFor({ state: "visible", timeout: 5000 });
    expect(await bibles.innerText()).toContain("raw.githubusercontent.com"); // the hosts it asks for, from its manifest
    expect(await page.locator(".fts-plugin-offer").count()).toBe(1);
    await page.click("#btn-first-time-skip");
    await page.locator('.fts-plugin-offer[data-plugin-id="genius_lyrics"]').waitFor({ state: "visible", timeout: 5000 });
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_hints_setup_songs.png` });
    await page.close();
  }, 60000);

  test("installed but not approved: a hint in each declared place, opening that plugin's review card", async () => {
    const page = await openConsole(browser, PORT_WITH);

    // Scriptures: the Online Bibles manifest's own label, muted, where its page will go.
    await page.click('.tab-btn[data-tab="scriptures"]');
    const bibles = page.locator('.plugin-source-node--off[data-plugin-id="online_bibles"]');
    await bibles.waitFor({ state: "visible", timeout: 10000 });
    expect(await bibles.innerText()).toContain("Online Bibles");
    // Songs and Media get theirs too (Genius; Stock Images and the Video Downloader).
    await page.click('.tab-btn[data-tab="songs"]');
    await page.locator('.plugin-source-node--off[data-plugin-id="genius_lyrics"]').waitFor({ state: "visible", timeout: 5000 });
    await page.click('.tab-btn[data-tab="media"]');
    await page.locator('.plugin-source-node--off[data-plugin-id="stock_media"]').waitFor({ state: "visible", timeout: 5000 });
    await page.locator('.plugin-source-node--off[data-plugin-id="ytdlp"]').waitFor({ state: "visible", timeout: 5000 });

    // The video picker's plugin tab is there, named after the one plugin that declares it.
    expect(await page.locator("#btn-video-subtab-ytdl").evaluate((el) => (el as HTMLElement).hidden)).toBe(false);
    expect(await page.textContent("#btn-video-subtab-ytdl")).toContain("Video Downloader");

    // The Store: one card per plugin not turned on yet, described by its own manifest.
    await openStore(page);
    const storeCard = page.locator('#store-plugin-cards .store-card[data-plugin-id="online_bibles"]');
    await storeCard.waitFor({ state: "visible", timeout: 5000 });
    expect(await storeCard.innerText()).toContain("Download");
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_hints_store.png` });
    await page.keyboard.press("Escape");

    // A hint opens Settings > Plugins on that plugin's card.
    await page.click('.tab-btn[data-tab="scriptures"]');
    await bibles.click();
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    const card = page.locator('.plugin-card[data-plugin-id="online_bibles"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="online_bibles"]')?.classList.contains("plugin-card--focus"), null, { timeout: 5000 });
    // (The emoji in the label is drawn as an icon, so match around it.)
    expect(await card.innerText()).toMatch(/Appears in: the scriptures library \(".*Online Bibles"\)/);

    // Approving it replaces the hint with the plugin's own (live) entry.
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="online_bibles"] .plugin-state')?.textContent?.includes("Approved"), null, { timeout: 10000 });
    await page.keyboard.press("Escape");
    await page.click('.tab-btn[data-tab="scriptures"]');
    await page.waitForFunction(() => {
      const nodes = Array.from(document.querySelectorAll(".category-node.plugin-source-node"));
      return nodes.some((n) => n.textContent?.includes("Online Bibles") && !n.classList.contains("plugin-source-node--off"))
        && !document.querySelector('.plugin-source-node--off[data-plugin-id="online_bibles"]');
    }, null, { timeout: 15000 });
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_hints_after_approve.png` });
    await page.close();
  }, 120000);

  test("no plugins installed (v0.2.0's packages): no plugin hints, tabs or Store cards anywhere", async () => {
    // openConsole waits for the plugin list the hints come from; then check every place one could appear.
    const page = await openConsole(browser, PORT_WITHOUT);
    for (const tab of ["songs", "scriptures", "media"]) {
      await page.click(`.tab-btn[data-tab="${tab}"]`);
      expect(await page.locator(".category-node.plugin-source-node").count()).toBe(0);
    }
    expect(await hintNodes(page).count()).toBe(0);
    expect(await page.locator("#btn-video-subtab-ytdl").evaluate((el) => (el as HTMLElement).hidden)).toBe(true);
    expect(await page.locator("#plugin-picker-hint").evaluate((el) => (el as HTMLElement).hidden)).toBe(true);
    await openStore(page);
    expect(await page.locator("#store-plugin-cards .store-card").count()).toBe(0);
    // Nothing in the console text still points at a plugin by name.
    const text = await page.evaluate(() => document.body.innerText);
    for (const name of ["Online Bibles", "Genius", "Song Lyrics", "Video Downloader", "yt-dlp", "Pexels", "Pixabay"]) {
      expect(text.includes(name)).toBe(false);
    }
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_hints_none_installed.png` });
    await page.keyboard.press("Escape");
    // First-time setup just explains pointing at your own files: no plugin to offer.
    await pluginListLoaded(page, () => openSetupAt(page, BIBLES_STEP)); // each step asks for the list itself
    expect(await page.locator("#fts-bibles-dir").isVisible()).toBe(true);
    expect(await page.locator(".fts-plugin-offer").count()).toBe(0);
    await pluginListLoaded(page, () => page.click("#btn-first-time-skip"));
    expect(await page.locator("#fts-songs-dir").isVisible()).toBe(true);
    expect(await page.locator(".fts-plugin-offer").count()).toBe(0);
    await page.close();
  }, 60000);
});
