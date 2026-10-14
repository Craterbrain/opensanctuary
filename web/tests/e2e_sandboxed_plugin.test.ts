import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: sandboxed plugin runs isolated and talks through the permission bridge", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9073;
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-sandbox-plugin-e2e-" });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const pluginFrame = () => page.frames().find((f) => f.url().includes("/plugin-frame/sandbox_demo/"));

  test("approve, run in an opaque-origin frame, observe state, send only granted commands", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    // Not approved: nothing is served and no frame exists.
    expect((await fetch(`http://127.0.0.1:${PORT}/plugin-frame/sandbox_demo/index.html`)).status).toBe(404);

    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="sandbox_demo"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    expect(await card.innerText()).toContain("Sandboxed page");
    // Decline the optional command permission first: the plugin must be unable to send.
    await card.locator("label:has-text('commands.send') input").uncheck();
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="sandbox_demo"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );

    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="sandbox_demo"]'), null, { timeout: 15000 });
    await page.waitForFunction(() => !!(window as any).__APP_READY__, null, { timeout: 15000 });
    let frame = pluginFrame();
    expect(frame).toBeDefined();
    await frame!.waitForFunction(() => document.getElementById("status")?.textContent?.startsWith("Items:"), null, { timeout: 15000 });
    await page.screenshot({ path: `${artifactDir}/screenshot_sandboxed_plugin_panel.png` });

    // The frame really is isolated: opaque origin, no parent access, no storage, no network of its own.
    const probe = await frame!.evaluate(async () => {
      const out: Record<string, string> = { origin: String(self.origin) };
      try { void (parent as any).document.title; out.parent = "readable"; } catch { out.parent = "blocked"; }
      try { void localStorage.length; out.storage = "readable"; } catch { out.storage = "blocked"; }
      try { await fetch("/api/state"); out.fetch = "allowed"; } catch { out.fetch = "blocked"; }
      return out;
    });
    expect(probe.origin).toBe("null");
    expect(probe.parent).toBe("blocked");
    expect(probe.storage).toBe("blocked");
    expect(probe.fetch).toBe("blocked");

    // commands.send was declined, so the bridge refuses (it never even reaches the server).
    await frame!.locator("#next").click();
    await frame!.waitForFunction(() => document.getElementById("result")?.textContent?.includes("not granted"), null, { timeout: 5000 });

    // Re-approve with the command permission: the buttons now work. (Focus is inside the plugin's
    // frame after the click; take it back so F2 reaches the console.)
    await page.evaluate(() => { (document.activeElement as HTMLElement | null)?.blur?.(); window.focus(); });
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    await card.waitFor({ state: "visible", timeout: 10000 });
    await card.locator("button:has-text('Revoke')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="sandbox_demo"] .plugin-state')?.textContent?.includes("Revoked"),
      null, { timeout: 10000 },
    );
    await card.locator("button:has-text('Approve')").click(); // all permissions ticked by default
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="sandbox_demo"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="sandbox_demo"]'), null, { timeout: 15000 });
    frame = pluginFrame();
    await frame!.waitForFunction(() => document.getElementById("status")?.textContent?.startsWith("Items:"), null, { timeout: 15000 });
    await frame!.locator("#next").click();
    await frame!.waitForFunction(() => document.getElementById("result")?.textContent === "NextSlide sent", null, { timeout: 8000 });
  }, 120000);
});
