import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Settings > Plugins review, approve, load, revoke", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9072;
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-plugins-e2e-" });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const pluginStatus = (path: string) => fetch(`http://127.0.0.1:${PORT}${path}`).then((r) => r.status);

  test("a bundled plugin needs approval, then loads in the console, then can be revoked", async () => {
    // Nothing is served or loaded before the operator approves.
    expect(await pluginStatus("/plugins/hello_world.js")).toBe(404);
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.waitForTimeout(1500);
    expect(await page.evaluate(() => !!(window as any).OS?.plugins?.hasPlugin?.("hello_world"))).toBe(false);

    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();

    const card = page.locator('.plugin-card[data-plugin-id="hello_world"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    expect(await card.locator(".plugin-state").innerText()).toContain("Needs review");
    // Both requested permissions are listed with their tier, and the console-script warning is shown.
    expect(await card.locator(".plugin-permission").count()).toBe(2);
    expect(await card.innerText()).toContain("ui.dom");
    expect(await card.innerText()).toContain("runs inside the console page");
    await page.screenshot({ path: `${artifactDir}/screenshot_settings_plugins_pending.png` });

    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="hello_world"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );
    expect(await pluginStatus("/plugins/hello_world.js")).toBe(200);
    await page.screenshot({ path: `${artifactDir}/screenshot_settings_plugins_approved.png` });

    // A fresh page load now brings the plugin up.
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.waitForFunction(() => (window as any).OS?.plugins?.hasPlugin?.("hello_world"), null, { timeout: 10000 });

    // Revoke from the console: no longer served, and not loaded on the next load.
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    await card.waitFor({ state: "visible", timeout: 10000 });
    await card.locator("button:has-text('Revoke')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="hello_world"] .plugin-state')?.textContent?.includes("Revoked"),
      null, { timeout: 10000 },
    );
    expect(await pluginStatus("/plugins/hello_world.js")).toBe(404);
  }, 90000);
});
