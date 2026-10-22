import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer, expectNativeIcons, pluginBridge } from "./e2e_helpers";

describe("E2E Live Test: Stock Images plugin (Pexels & Pixabay) replaces the built-in Online Images", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9077;
  let artifactDir: string;

  beforeAll(async () => {
    const mediaDir = mkdtempSync(join(tmpdir(), "os-next-stock-media-"));
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-stock-plugin-e2e-", extraArgs: ["--media-dir", mediaDir] });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const frameOf = () => page.frames().find((f) => f.url().includes("/plugin-frame/stock_media/"));
  const bridge = pluginBridge(frameOf);

  test("the built-in search is gone; the plugin takes API keys, explains a rejected key, and adds a photo", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    // Gone from the server, the library sidebar and Settings.
    expect((await fetch(`http://127.0.0.1:${PORT}/api/media/online/search?q=sun`)).status).toBe(404);
    expect((await fetch(`http://127.0.0.1:${PORT}/api/media/online/import`, { method: "POST" })).status).not.toBe(200);
    await page.click('.tab-btn[data-tab="media"]');
    // Not until approved -- only its "not turned on" hint.
    expect(await page.locator('.category-node:not(.plugin-source-node--off):has-text("Online Images")').count()).toBe(0);
    expect(await page.locator('.plugin-source-node--off[data-plugin-id="stock_media"]').count()).toBe(1);

    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    expect(await page.locator("text=Pexels API Key").count()).toBe(0);
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="stock_media"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    const text = await card.innerText();
    for (const host of ["api.pexels.com", "images.pexels.com", "pixabay.com", "cdn.pixabay.com"]) expect(text).toContain(host);
    await page.screenshot({ path: `${artifactDir}/screenshot_stock_media_review.png` });
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="stock_media"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );
    await page.keyboard.press("Escape");

    const node = page.locator('.category-node:not(.plugin-source-node--off):has-text("Online Images (Pexels/Pixabay)")');
    await node.waitFor({ state: "visible", timeout: 10000 });
    await node.click();
    const frame = frameOf()!;
    await frame.waitForSelector("#q", { state: "visible", timeout: 15000 });
    await frame.waitForFunction(() => document.getElementById("status")?.textContent?.includes("Add a free API key"), null, { timeout: 10000 });
    expect(await frame.locator("#keys").evaluate((el) => (el as HTMLDetailsElement).open)).toBe(true); // opens itself when there are no keys
    await expectNativeIcons(frame, 1);
    await page.screenshot({ path: `${artifactDir}/screenshot_stock_media_panel.png` });

    // Searching without a key never reaches the network.
    await frame.fill("#q", "sunrise");
    await frame.click("#search-btn");
    await frame.waitForFunction(() => document.getElementById("status")?.textContent?.includes("Turn on a source"), null, { timeout: 5000 });

    // A made-up key is saved in the keyring, and each site refuses it in words the operator can act on.
    await frame.fill("#key-pexels", "not-a-real-pexels-key");
    await frame.click("#save-pexels");
    await frame.waitForFunction(() => document.getElementById("status")?.textContent?.includes("Pexels key saved"), null, { timeout: 10000 });
    expect(await frame.locator("#key-pexels").inputValue()).toBe(""); // never left sitting in the field
    await frame.fill("#key-pixabay", "not-a-real-pixabay-key");
    await frame.click("#save-pixabay");
    await frame.waitForFunction(() => document.getElementById("status")?.textContent?.includes("Pixabay key saved"), null, { timeout: 10000 });
    await frame.click("#search-btn");
    await frame.waitForFunction(() => /did not accept that API key|rejected that request/.test(document.getElementById("status")?.textContent || ""), null, { timeout: 30000 });
    const status = await frame.locator("#status").innerText();
    expect(status).toContain("Pexels did not accept that API key");
    expect(status).toContain("Pixabay rejected that request");
    // A reload of the panel remembers the keys (they come back from the keyring, not the page).
    const keyBack = await bridge("keyring.get", { account: "pexels" });
    expect(keyBack.result.secret).toBe("not-a-real-pexels-key");

    // Adding a photo: the host downloads it from a granted host into the Media library.
    const refused = await bridge("library.addMediaUrl", { url: "https://example.com/a.jpg", title: "nope" });
    expect(refused.ok).toBe(false);
    const html = await bridge("library.addMediaUrl", { url: "https://pixabay.com/", title: "a page, not a picture" });
    expect(html.ok).toBe(false);
    const added = await bridge("library.addMediaUrl", {
      url: "https://images.pexels.com/photos/1366919/pexels-photo-1366919.jpeg?auto=compress&cs=tinysrgb&w=400",
      title: "forest (Pexels photo by E2E)", id: "pexels_1366919" });
    expect(added.error ?? "").toBe("");
    expect(added.ok).toBe(true);
    const media = await (await fetch(`http://127.0.0.1:${PORT}/api/media`)).json();
    const item = media.find((m: any) => m.id === added.result.id);
    expect(item?.name).toBe("forest (Pexels photo by E2E)");
    expect(item?.media_type).toBe("image");
    expect(item?.file_path).toMatch(/^\/media\/images\/stockmedia_[0-9a-f-]+\.jpg$/);
    const file = await fetch(`http://127.0.0.1:${PORT}${item.file_path}`);
    expect(file.status).toBe(200);
    expect(file.headers.get("content-type")).toContain("image/jpeg");

    // And it's in the Media list like any other background.
    await page.locator('.category-node:has-text("All Media")').click();
    await page.waitForSelector("text=forest (Pexels photo by E2E)", { timeout: 10000 });

    // Removing a key forgets it.
    await page.locator('.category-node:not(.plugin-source-node--off):has-text("Online Images (Pexels/Pixabay)")').click();
    const again = frameOf()!;
    await again.waitForSelector("#q", { state: "visible", timeout: 15000 });
    await again.locator("#keys").evaluate((el) => { (el as HTMLDetailsElement).open = true; });
    await again.click("#clear-pexels");
    await again.waitForFunction(() => document.getElementById("status")?.textContent?.includes("Pexels key removed"), null, { timeout: 10000 });
    expect((await bridge("keyring.get", { account: "pexels" })).result?.secret ?? "").toBe("");
  }, 150000);
});
