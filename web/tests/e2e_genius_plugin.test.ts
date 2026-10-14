import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Genius Lyrics plugin (sandboxed) imports through the host", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9074;
  let artifactDir: string;

  beforeAll(async () => {
    // Imported songs are stored in the songs directory, which defaults to ./songs in the repo:
    // give this test its own so it neither pollutes nor depends on anything else.
    const songsDir = mkdtempSync(join(tmpdir(), "os-next-genius-songs-"));
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-genius-plugin-e2e-", extraArgs: ["--songs-dir", songsDir] });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const frameOf = () => page.frames().find((f) => f.url().includes("/plugin-frame/genius_lyrics/"));

  test("needs approval, then appears as a panel; search and import go through the bridge", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    // The old built-in Genius search mode and category are gone from the song library.
    expect(await page.locator("text=Genius Christian Lyrics").count()).toBe(0);
    expect((await fetch(`http://127.0.0.1:${PORT}/api/songs/genius/search?q=x`)).status).toBe(404);
    expect((await fetch(`http://127.0.0.1:${PORT}/plugin-frame/genius_lyrics/index.html`)).status).toBe(404);

    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="genius_lyrics"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    const text = await card.innerText();
    expect(text).toContain("Genius Lyrics");
    expect(text).toContain("genius.com");
    expect(text).toContain("network.outbound");
    expect(text).toContain("library.write");
    await page.screenshot({ path: `${artifactDir}/screenshot_genius_plugin_review.png` });
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="genius_lyrics"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );

    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="genius_lyrics"]'), null, { timeout: 15000 });
    const frame = frameOf()!;
    await frame.waitForSelector("#q", { timeout: 15000 });
    await page.screenshot({ path: `${artifactDir}/screenshot_genius_plugin_panel.png` });

    // Search goes plugin -> bridge -> server fetch. Whether or not this machine can reach genius.com,
    // it must finish (results or a clear error) rather than hang.
    await frame.fill("#q", "Way Maker");
    await frame.click("#search-btn");
    await frame.waitForFunction(() => {
      const t = document.getElementById("status")?.textContent || "";
      return t !== "" && !t.startsWith("Searching");
    }, null, { timeout: 40000 });

    // Import path, driven the same way the plugin does it: library.addSong over the bridge.
    const song = { title: "E2E Bridge Song", author: "Tester", copyright: "Genius Christian Lyrics", ccli_number: null,
      slides: [{ text: "Line one\nLine two", header: "V1", label: "Verse 1", tag: "Verse 1" }], theme_name: null, alternate_title: null };
    const added = await frame.evaluate((s) => new Promise<any>((resolve) => {
      const id = 9001;
      window.addEventListener("message", (ev) => { if (ev.data?.type === "os.response" && ev.data.id === id) resolve(ev.data); });
      parent.postMessage({ type: "os.request", id, method: "library.addSong", params: { song: s } }, "*");
    }), song);
    expect(added.error ?? "").toBe("");
    expect(added.ok).toBe(true);
    const songs = await (await fetch(`http://127.0.0.1:${PORT}/api/songs`)).json();
    const stored = songs.find((x: any) => x.id === added.result.id);
    expect(stored?.title).toBe("E2E Bridge Song");

    // The console's library list refreshes on its own.
    await page.locator('.tab-btn:has-text("Songs"), [data-tab="songs"]').first().click().catch(() => {});
    await page.waitForSelector("text=E2E Bridge Song", { timeout: 10000 });

    // Anything outside the grant is refused by the bridge.
    const refused = await frame.evaluate(() => new Promise<any>((resolve) => {
      const id = 9002;
      window.addEventListener("message", (ev) => { if (ev.data?.type === "os.response" && ev.data.id === id) resolve(ev.data); });
      parent.postMessage({ type: "os.request", id, method: "net.fetch", params: { url: "https://example.com/" } }, "*");
    }));
    expect(refused.ok).toBe(false);
  }, 150000);
});
