import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Live Test: Mobile Remote Control Webpage", () => {
  let serverProc: Subprocess;
  let browser: Browser;
  let page: Page;
  let testDir: string;
  let DB_PATH: string;
  const PORT = 9028;
  const artifactDir = "/home/jasonb/.gemini/antigravity/brain/f8991531-6ae4-4b9a-9bd5-2b8c056c8256";

  beforeAll(async () => {
    testDir = mkdtempSync(join(tmpdir(), "os-next-remote-mobile-e2e-"));
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
    // Mobile Viewport (iPhone 14 / Pixel 7: 390x844)
    page = await browser.newPage({
      viewport: { width: 390, height: 844 },
      hasTouch: true,
      isMobile: true
    });
    page.on("pageerror", err => console.log("MOBILE PAGE ERR:", err.message));
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    if (serverProc) {
      serverProc.kill();
      await serverProc.exited;
    }
    try { rmSync(testDir, { recursive: true, force: true }); } catch (_) {}
  });

  test("Mobile remote loads, auto-connects to WebSocket, and handles full control navigation", async () => {
    // 1. Populate schedule with sample hymn item
    const sampleHymn = {
      id: "hymn_1",
      title: "Great Is Thy Faithfulness",
      subtitle: null,
      notes: null,
      item_type: "song",
      author_or_ref: "Thomas Chisholm",
      slides: [
        {
          text: "Great is Thy faithfulness, O God my Father,\nThere is no shadow of turning with Thee;",
          header: null,
          label: "Verse 1",
          tag: "V1",
          background: "#000",
          notes: null,
          elements: []
        },
        {
          text: "Great is Thy faithfulness! Great is Thy faithfulness!\nMorning by morning new mercies I see;",
          header: null,
          label: "Chorus",
          tag: "C",
          background: "#111",
          notes: null,
          elements: []
        },
        {
          text: "Summer and winter, and springtime and harvest,\nSun, moon and stars in their courses above",
          header: null,
          label: "Verse 2",
          tag: "V2",
          background: "#222",
          notes: null,
          elements: []
        }
      ],
      background: null,
      theme_name: null,
      is_expanded: false,
      is_section_header: false,
      arrangement: []
    };

    await fetch(`http://127.0.0.1:${PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ AddToSchedule: sampleHymn })
    });

    // Go live with item
    await fetch(`http://127.0.0.1:${PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ GoLive: { item_index: 0, slide_index: 0 } })
    });

    // 2. Navigate to /remote (testing short URL redirect), carrying a real
    // paired-device token the same way the console's QR code does — /remote
    // now requires pairing (see docs/GEMINI_COMMIT_REVIEW_2026-09-22.md).
    // Minting that token is console-only server-side, gated behind the host
    // session token — this test's own fetch is on 127.0.0.1, so it can
    // fetch that the same way a real console browser tab would.
    const hostTokenRes = await fetch(`http://127.0.0.1:${PORT}/api/internal/host-token`);
    const hostTokenData = await hostTokenRes.json();
    expect(typeof hostTokenData.host_token).toBe("string");

    const pairRes = await fetch(`http://127.0.0.1:${PORT}/api/pairing/remote-session`, {
      method: "POST",
      headers: { "x-host-token": hostTokenData.host_token },
    });
    const pairData = await pairRes.json();
    expect(pairData.success).toBe(true);
    expect(typeof pairData.token).toBe("string");

    await page.goto(`http://127.0.0.1:${PORT}/remote#token=${pairData.token}`);
    expect(page.url()).toContain("/remote.html");

    // 3. Wait for WebSocket connection
    await page.waitForSelector(".status-dot.connected", { timeout: 10000 });
    const connStatusText = await page.locator("#conn-text").textContent();
    expect(connStatusText).toBe("Connected");

    // 4. Verify Schedule View
    await page.waitForSelector(".schedule-card", { timeout: 5000 });
    const schedTitle = await page.locator(".item-title").first().textContent();
    expect(schedTitle).toContain("Great Is Thy Faithfulness");

    // Verify LIVE badge on active schedule item
    const isLiveBadgeVisible = await page.locator(".live-badge").first().isVisible();
    expect(isLiveBadgeVisible).toBe(true);

    // Save Schedule screenshot artifact
    await page.screenshot({ path: `${artifactDir}/screenshot_mobile_remote_schedule.png` });

    // 5. Switch to Slides Tab
    await page.click("#tab-slides");
    await page.waitForSelector(".slide-card", { timeout: 5000 });

    const bannerTitle = await page.locator("#slides-banner-title").textContent();
    expect(bannerTitle).toContain("Great Is Thy Faithfulness");

    const slideCount = await page.locator(".slide-card").count();
    expect(slideCount).toBe(3);

    // First slide should be active (.is-active)
    const firstSlideActive = await page.locator(".slide-card").first().evaluate(el => el.classList.contains("is-active"));
    expect(firstSlideActive).toBe(true);

    // Mini output preview should reflect active slide text
    const previewText = await page.locator("#mini-preview-text").textContent();
    expect(previewText).toContain("Great is Thy faithfulness");

    // Save Slides screenshot artifact
    await page.screenshot({ path: `${artifactDir}/screenshot_mobile_remote_slides.png` });

    // 6. Test Navigation: Tap "Next ▶" button to advance to Chorus
    await page.click("#btn-next-slide");
    await page.waitForFunction(() => {
      const cards = document.querySelectorAll(".slide-card");
      return cards[1] && cards[1].classList.contains("is-active");
    }, { timeout: 5000 });

    const chorusPreview = await page.locator("#mini-preview-text").textContent();
    expect(chorusPreview).toContain("Morning by morning");

    // 7. Test Overrides: Blackout Screen
    await page.click("#btn-override-black");
    await page.waitForSelector("#btn-override-black.active-black", { timeout: 5000 });
    const blackState = await page.locator("#btn-override-black").evaluate(el => el.classList.contains("active-black"));
    expect(blackState).toBe(true);

    // Untoggle Blackout
    await page.click("#btn-override-black");
    await page.waitForFunction(() => {
      const el = document.getElementById("btn-override-black");
      return el && !el.classList.contains("active-black");
    }, { timeout: 5000 });

    // 8. Switch to Prompter Tab (Personal Stage Prompter)
    await page.click("#tab-prompter");
    await page.waitForSelector("#prompter-screen", { timeout: 5000 });

    const prompterLyrics = await page.locator("#prompter-lyrics").textContent();
    expect(prompterLyrics).toContain("Great is Thy faithfulness");

    const lookahead = await page.locator("#prompter-next-text").textContent();
    expect(lookahead).toContain("Summer and winter");

    // Save Prompter screenshot artifact
    await page.screenshot({ path: `${artifactDir}/screenshot_mobile_remote_prompter.png` });

    // 9. Landscape View test (rotate mobile device to landscape 844x390)
    await page.setViewportSize({ width: 844, height: 390 });
    await page.click("#tab-slides");
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${artifactDir}/screenshot_mobile_remote_landscape.png` });
  }, 45000);

  test("Mobile remote shows the pairing gate and never connects without a valid device token", async () => {
    const unpairedPage = await browser.newPage({ viewport: { width: 390, height: 844 } });
    try {
      // No token at all
      await unpairedPage.goto(`http://127.0.0.1:${PORT}/remote`);
      await unpairedPage.waitForSelector("#pairing-gate", { state: "visible", timeout: 5000 });
      let appVisible = await unpairedPage.locator("#remote-app").isVisible();
      expect(appVisible).toBe(false);

      // A garbage/unknown token should be rejected the same way
      await unpairedPage.goto(`http://127.0.0.1:${PORT}/remote#token=not-a-real-token`);
      await unpairedPage.waitForSelector("#pairing-gate", { state: "visible", timeout: 5000 });
      appVisible = await unpairedPage.locator("#remote-app").isVisible();
      expect(appVisible).toBe(false);
    } finally {
      await unpairedPage.close();
    }
  }, 30000);
});
