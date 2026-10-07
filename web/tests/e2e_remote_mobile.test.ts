import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Mobile Remote Control Webpage", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9028;
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-remote-mobile-e2e-" });
    artifactDir = ensureArtifactDir();

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
    await teardownTestServer(server);
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

    // /api/command now requires the host token for callers with no paired-
    // device token (docs/CLIENT_PAIRING.md), same as /api/pairing/remote-
    // session below — this test's own fetch is on 127.0.0.1, so it can
    // fetch that token the same way a real console browser tab would.
    const hostTokenRes = await fetch(`http://127.0.0.1:${PORT}/api/internal/host-token`);
    const hostTokenData = await hostTokenRes.json();
    expect(typeof hostTokenData.host_token).toBe("string");

    // /api/command also enforces "one console at a time" now
    // (docs/CLIENT_PAIRING.md) -- x-console-session-id identifies this
    // caller as one console; nothing else in this test ever claims the
    // lock (the mobile page below is a paired remote, not a console), so
    // any non-empty id works.
    const consoleSessionId = "e2e-remote-mobile-test-console";
    await fetch(`http://127.0.0.1:${PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "x-host-token": hostTokenData.host_token, "x-console-session-id": consoleSessionId },
      body: JSON.stringify({ AddToSchedule: sampleHymn })
    });

    // Go live with item
    await fetch(`http://127.0.0.1:${PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "x-host-token": hostTokenData.host_token, "x-console-session-id": consoleSessionId },
      body: JSON.stringify({ GoLive: { item_index: 0, slide_index: 0 } })
    });

    // 2. Navigate to /remote (testing short URL redirect), carrying a real
    // paired-device token the same way the console's QR code does — /remote
    // now requires pairing. Minting that token is console-only server-side,
    // gated behind the same host session token fetched above.

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
