import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Settings Dialog Network Tab & Option 12 Management", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9045;
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-network-settings-e2e-" });
    artifactDir = ensureArtifactDir();

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({
      viewport: { width: 1280, height: 800 }
    });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("Opens Settings modal, navigates to Network tab, validates controls, conflict checks, and broadcasts Option 12", async () => {
    // 1. Load Desktop OpenSanctuary UI
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });

    // 2. Open Settings Dialog via keyboard shortcut F2 or menu
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });

    // 3. Locate and click the Network category button in sidebar
    const networkBtn = page.locator(".settings-category-btn:has-text('Network')");
    expect(await networkBtn.isVisible()).toBe(true);
    await networkBtn.click();

    // 4. Verify Network Panel renders and finishes discovery fetch
    await page.waitForSelector(".network-overview-card", { state: "visible", timeout: 5000 });

    // 5. Verify Overview Card stats
    const overviewCard = page.locator(".network-overview-card");
    expect(await overviewCard.isVisible()).toBe(true);
    const overviewText = await overviewCard.textContent();
    expect(overviewText).toContain(String(PORT));
    expect(overviewText).toContain("opensanctuary.local");

    // 6. Verify Hostname & Option 12 section
    const hostInput = page.locator("#network-hostname-input");
    expect(await hostInput.isVisible()).toBe(true);
    expect(await hostInput.inputValue()).toBe("opensanctuary");

    const hostBadge = page.locator("#network-hostname-badge");
    expect(await hostBadge.isVisible()).toBe(true);
    // Wait for debounced LAN check to finish
    await page.waitForFunction(() => {
      const el = document.getElementById("network-hostname-badge");
      return el && el.textContent && el.textContent.includes("Available");
    }, { timeout: 5000 });

    // 7. Verify Port section
    const portInput = page.locator("#network-port-input");
    expect(await portInput.isVisible()).toBe(true);
    expect(await portInput.inputValue()).toBe(String(PORT));

    const portBadge = page.locator("#network-port-badge");
    expect(await portBadge.isVisible()).toBe(true);
    await page.waitForFunction(() => {
      const el = document.getElementById("network-port-badge");
      return el && el.textContent && el.textContent.includes("active server port");
    }, { timeout: 5000 });

    // 8. Verify Dedicated MAC section & Virtual Adapter controls
    const dedicatedBtn = page.locator("#btn-toggle-dedicated-mac");
    expect(await dedicatedBtn.isVisible()).toBe(true);

    // 9. Verify Multi-Interface Broadcast and Detected Connections
    const bcastAllToggle = page.locator("#network-broadcast-all-toggle");
    expect(await bcastAllToggle.isVisible()).toBe(true);

    const ifaceCards = page.locator(".network-iface-card");
    const count = await ifaceCards.count();
    expect(count).toBeGreaterThan(0);

    // 10. Test Broadcast Option 12 Now
    const bcastBtn = page.locator("#btn-broadcast-option12");
    expect(await bcastBtn.isVisible()).toBe(true);
    await bcastBtn.click();

    // Verify broadcast badge feedback
    await page.waitForFunction(() => {
      const badge = document.getElementById("network-broadcast-status");
      return badge && badge.style.display !== "none" && badge.textContent && badge.textContent.includes("Option 12");
    }, { timeout: 8000 });

    // 11. Capture top screenshot of the Network Settings panel
    await page.screenshot({
      path: `${artifactDir}/screenshot_settings_network_tab.png`,
      fullPage: false,
    });

    // 12. Scroll settings-content down to reveal Dedicated Adapter and Interfaces
    await page.locator("#settings-content").evaluate((el: HTMLElement) => {
      el.scrollTop = el.scrollHeight;
    });
    await page.waitForTimeout(300);

    await page.screenshot({
      path: `${artifactDir}/screenshot_settings_network_interfaces.png`,
      fullPage: false,
    });

    console.log("Screenshots successfully saved to artifact directory.");
  }, 30000);
});
