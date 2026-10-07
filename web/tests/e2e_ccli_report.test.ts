import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: CCLI Usage Report modal (docs/CCLI_REPORTING.md)", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9050;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-ccli-report-e2e-" });

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("Opens from Settings > Integrations, loads an empty report, and exports a CSV", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });

    // Open Settings and navigate to Integrations
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Integrations')").click();

    const openBtn = page.locator("#setting-action-ccliReport");
    expect(await openBtn.isVisible()).toBe(true);
    await openBtn.click();

    // The report modal opens and auto-loads the (empty) default date range
    await page.waitForSelector("#ccli-report-modal", { state: "visible", timeout: 5000 });
    await page.waitForFunction(() => {
      const el = document.getElementById("ccli-report-reportable-list");
      return !!el && el.textContent!.includes("No songs went live");
    }, { timeout: 5000 });

    const reportableCount = page.locator("#ccli-report-reportable-count");
    expect(await reportableCount.textContent()).toBe("0 songs");
    const excludedCount = page.locator("#ccli-report-excluded-count");
    expect(await excludedCount.textContent()).toBe("0 songs");

    // Export CSV -- confirms the host-token-gated download round-trips
    // without throwing (download event firing is the proof; this server has
    // no song-went-live history yet, so the file itself is just headers).
    const [download] = await Promise.all([
      page.waitForEvent("download", { timeout: 5000 }),
      page.locator("#btn-ccli-report-export").click(),
    ]);
    expect(download.suggestedFilename()).toBe("ccli-usage-report.csv");

    // No credentials saved yet -- the status line says so.
    await page.waitForFunction(() => {
      const el = document.getElementById("ccli-report-creds-status");
      return !!el && el.textContent!.includes("No login saved");
    }, { timeout: 5000 });

    // Save a CCLI login via the real keyring round-trip (OS credential
    // store, not a mock), then confirm the status line picks it up.
    await page.fill("#ccli-report-username", "pastor@example.church");
    await page.fill("#ccli-report-password", "s3cret-pw");
    await page.locator("#btn-ccli-report-save-creds").click();
    await page.waitForFunction(() => {
      const el = document.getElementById("ccli-report-creds-status");
      return !!el && el.textContent!.includes("pastor@example.church");
    }, { timeout: 5000 });

    // Open Upload Assistant -- this test server runs --headless (no native
    // desktop window), so this must degrade to a clear error rather than
    // throwing or silently doing nothing.
    await page.locator("#btn-ccli-report-open-assist").click();
    await page.waitForFunction(() => {
      const toasts = Array.from(document.querySelectorAll(".os-toast-msg"));
      return toasts.some(t => t.textContent?.includes("no native desktop window"));
    }, { timeout: 5000 });

    // Clear the saved login and confirm the status line reverts.
    await page.locator("#btn-ccli-report-clear-creds").click();
    await page.waitForFunction(() => {
      const el = document.getElementById("ccli-report-creds-status");
      return !!el && el.textContent!.includes("No login saved");
    }, { timeout: 5000 });

    // Close via the header close button
    await page.locator("#btn-close-ccli-report").click();
    await page.waitForSelector("#ccli-report-modal", { state: "hidden", timeout: 5000 });
  }, 30000);
});
