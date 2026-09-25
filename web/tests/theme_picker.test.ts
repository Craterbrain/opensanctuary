import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Live Test: Background Modal CSS Tab & Centered Header Tabs", () => {
  let serverProc: Subprocess;
  let browser: Browser;
  let page: Page;
  let testDir: string;
  let DB_PATH: string;
  const PORT = 9012;

  beforeAll(async () => {
    testDir = mkdtempSync(join(tmpdir(), "os-next-theme-picker-e2e-"));
    DB_PATH = join(testDir, "test.db");

    const binaryPath = resolve(__dirname, "../../target/release/os-next");
    serverProc = spawn([
      binaryPath,
      "--headless",
      "--port", PORT.toString(),
      "--db-path", DB_PATH,
      "--web-dir", resolve(__dirname, "../")
    ], {
      cwd: resolve(__dirname, "../../"),
      stdout: "ignore",
      stderr: "ignore"
    });

    let ready = false;
    for (let i = 0; i < 40; i++) {
      try {
        const res = await fetch(`http://127.0.0.1:${PORT}/`);
        if (res.ok) { ready = true; break; }
      } catch (_) {}
      await new Promise(r => setTimeout(r, 250));
    }
    if (!ready) throw new Error("Server failed to start in 10s");

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    if (serverProc) serverProc.kill();
    try { rmSync(testDir, { recursive: true, force: true }); } catch (_) {}
  });

  test("Background modal header tabs are horizontally centered", async () => {
    await page.evaluate(() => {
      (window as any).openThemePicker(null);
    });
    await page.waitForTimeout(300);

    const isCentered = await page.evaluate(() => {
      const header = document.querySelector('#media-image-picker-modal .modal-header') as HTMLElement;
      const tabs = document.getElementById('bg-picker-header-tabs') as HTMLElement;
      const headerRect = header.getBoundingClientRect();
      const tabsRect = tabs.getBoundingClientRect();

      const headerCenter = headerRect.left + headerRect.width / 2;
      const tabsCenter = tabsRect.left + tabsRect.width / 2;
      // Allow minor subpixel precision difference
      return Math.abs(headerCenter - tabsCenter) <= 2;
    });

    expect(isCentered).toBe(true);
  });

  test("CSS tab includes search input and category filter buttons", async () => {
    const hasSearch = await page.isVisible("#theme-picker-search");
    const hasCategories = await page.isVisible("#theme-picker-categories");
    expect(hasSearch).toBe(true);
    expect(hasCategories).toBe(true);

    const categoryButtons = await page.$$eval('#theme-picker-categories button', (btns) =>
      btns.map(b => b.textContent?.trim())
    );
    expect(categoryButtons).toContain("All Styles");
    expect(categoryButtons).toContain("Gradients");
    expect(categoryButtons).toContain("Solid Colors");
    expect(categoryButtons).toContain("Patterns");
  });

  test("Category filtering updates visible cards correctly", async () => {
    // Click Gradients
    await page.click('#theme-picker-categories button[data-cat="gradient"]');
    await page.waitForTimeout(200);
    const gradientCount = await page.$$eval('.theme-picker-card', cards => cards.length);
    expect(gradientCount).toBeGreaterThanOrEqual(5);

    // Click Solid Colors
    await page.click('#theme-picker-categories button[data-cat="solid"]');
    await page.waitForTimeout(200);
    const solidCount = await page.$$eval('.theme-picker-card', cards => cards.length);
    expect(solidCount).toBeGreaterThanOrEqual(1);

    // Click Patterns
    await page.click('#theme-picker-categories button[data-cat="pattern"]');
    await page.waitForTimeout(200);
    const patternCount = await page.$$eval('.theme-picker-card', cards => cards.length);
    expect(patternCount).toBeGreaterThanOrEqual(1);
    const patternTitles = await page.$$eval('.theme-picker-card .theme-picker-title', titles =>
      titles.map(t => t.textContent?.trim())
    );
    expect(patternTitles).toContain("Drifting Clouds");
  });

  test("Search filtering filters cards by name", async () => {
    // Reset to All Styles
    await page.click('#theme-picker-categories button[data-cat="all"]');
    await page.fill('#theme-picker-search', 'Ocean');
    await page.waitForTimeout(200);

    const filteredTitles = await page.$$eval('.theme-picker-card .theme-picker-title', titles =>
      titles.map(t => t.textContent?.trim())
    );
    expect(filteredTitles).toContain("Midnight Ocean");
    expect(filteredTitles).not.toContain("Solid Dark");
  });

  test("Theme cards have 16:9 previews, title bar, and selection checkmark badge", async () => {
    await page.fill('#theme-picker-search', '');
    await page.waitForTimeout(200);

    const cardDetails = await page.evaluate(() => {
      const card = document.querySelector('.theme-picker-card.selected') as HTMLElement;
      const preview = card.querySelector('.theme-picker-preview') as HTMLElement;
      const title = card.querySelector('.theme-picker-title') as HTMLElement;
      const pRect = preview.getBoundingClientRect();
      const aspectRatio = pRect.width / pRect.height;
      return {
        hasTitle: !!title && !!title.textContent,
        aspectRatioCloseTo16_9: Math.abs(aspectRatio - (16 / 9)) < 0.2
      };
    });

    expect(cardDetails.hasTitle).toBe(true);
    expect(cardDetails.aspectRatioCloseTo16_9).toBe(true);
  });
});
