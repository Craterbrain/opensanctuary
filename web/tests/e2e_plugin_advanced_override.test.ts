import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Settings > Plugins Advanced override", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9078;
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-plugin-advanced-e2e-" });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("override what a plugin asks for: warning first, acknowledgement required, extras toggleable, change is shown", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="stock_media"]');
    await card.waitFor({ state: "visible", timeout: 10000 });

    // Hidden until asked for; opening it shows the warning before anything else.
    expect(await card.locator(".plugin-advanced-panel").isVisible()).toBe(false);
    await card.locator(".plugin-advanced-toggle input").check();
    const panel = card.locator(".plugin-advanced-panel");
    await panel.waitFor({ state: "visible", timeout: 5000 });
    const warning = await panel.locator(".plugin-warning").first().innerText();
    expect(warning).toContain("can make it misbehave or stop working");
    expect(warning).toContain("more access than its author intended");

    // Every permission it asked for is toggleable, with an editable scope; others it did not ask for are listed.
    const hosts = panel.locator('input[aria-label="network.outbound hosts"]');
    await hosts.waitFor({ state: "visible", timeout: 5000 });
    expect(await hosts.inputValue()).toContain("api.pexels.com");
    expect(await panel.locator('.plugin-advanced-row:has-text("keyring.own") input[type=checkbox]').first().isChecked()).toBe(true);
    expect(await panel.locator('.plugin-advanced-row:has-text("show.observe")').count()).toBe(1);
    // A sandboxed page can't have these, so they aren't offered.
    expect(await panel.locator('.plugin-advanced-row:has-text("ui.dom")').count()).toBe(0);
    expect(await panel.locator('.plugin-advanced-row:has-text("native.code")').count()).toBe(0);
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_advanced.png` });

    // Make the changes: withhold keyring.own, narrow the hosts, grant show.observe (never asked for).
    await panel.locator('.plugin-advanced-row:has-text("keyring.own") input[type=checkbox]').first().uncheck();
    await hosts.fill("api.pexels.com");
    await panel.locator('.plugin-advanced-row:has-text("show.observe") input[type=checkbox]').first().check();

    // The plain Approve button is replaced; the override can't be applied without acknowledging.
    expect(await card.locator("button:has-text('Approve')").first().isVisible()).toBe(true);
    const apply = card.locator("button:has-text('Apply override')");
    expect(await apply.isVisible()).toBe(true);
    await apply.click();
    await page.waitForTimeout(400);
    expect(await card.locator(".plugin-state").first().innerText()).not.toContain("Approved");
    await panel.locator(".plugin-advanced-ack input").check();
    await apply.click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="stock_media"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );

    // The result is what the server enforces and the card says so.
    const view = (await (await fetch(`http://127.0.0.1:${PORT}/api/internal/host-token`)).json()).host_token;
    const info = (await (await fetch(`http://127.0.0.1:${PORT}/api/plugins`, { headers: { "x-host-token": view } })).json())
      .find((p: any) => p.id === "stock_media");
    expect(info.overridden).toBe(true);
    expect(info.granted.map((g: any) => g.id).sort()).toEqual(["library.write", "network.outbound", "notifications.post", "show.observe", "ui.panel"]);
    expect(info.granted.find((g: any) => g.id === "network.outbound").hosts).toEqual(["api.pexels.com"]);
    await card.locator(".plugin-warning:has-text('You overrode')").waitFor({ state: "visible", timeout: 5000 });
    expect(await card.innerText()).toContain("Permissions overridden");
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_advanced_applied.png` });
  }, 60000);
});
