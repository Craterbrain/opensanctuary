import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Live Test: Slide Editor Bulk Paste Upgrades", () => {
  let serverProc: Subprocess;
  let browser: Browser;
  let page: Page;
  let testDir: string;
  let DB_PATH: string;
  const PORT = 9022;
  const artifactDir = "/home/jasonb/.gemini/antigravity/brain/f8991531-6ae4-4b9a-9bd5-2b8c056c8256";

  beforeAll(async () => {
    testDir = mkdtempSync(join(tmpdir(), "os-next-bulk-paste-e2e-"));
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
    page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));

    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
    await page.waitForLoadState("networkidle");
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    if (serverProc) {
      serverProc.kill();
      await serverProc.exited;
    }
    try { rmSync(testDir, { recursive: true, force: true }); } catch (_) {}
  });

  test("Bulk Paste workflow: live counter badge, chord cleaning, CCLI extraction, and slide section tagging", async () => {
    // 1. Open slide creator for a song
    await page.evaluate(() => (window as any).openSlideEditor('song'));
    await page.waitForSelector("#create-modal", { state: "visible", timeout: 8000 });
    await page.waitForTimeout(500);

    // 2. Click "Paste Text → Slides" to open bulk overlay
    await page.click("#btn-open-bulk-paste");
    await page.waitForSelector("#studio-bulk-view", { state: "visible" });

    // 3. Verify Bulk toolbar elements are present
    const splitRule = await page.$("#bulk-split-rule");
    const actionMode = await page.$("#bulk-action-mode");
    const cleanChordsBtn = await page.$("#btn-bulk-clean-chords");
    const counterBadge = await page.$("#bulk-slide-count-badge");
    const tagsRow = await page.$("#studio-bulk-tags-row");

    expect(splitRule).not.toBeNull();
    expect(actionMode).not.toBeNull();
    expect(cleanChordsBtn).not.toBeNull();
    expect(counterBadge).not.toBeNull();
    expect(tagsRow).not.toBeNull();

    // 4. Fill in dirty song with chords, annotations, and CCLI metadata
    const songInput = `
Verse 1
[G]Bless the Lord [D/F#]O my soul
[Em7]O my [C]soul
(Repeat 2x)

Chorus
[C]Sing like [G]never before
[Em7]O my [C]soul

Bridge
Ten thousand reasons for my heart to find

Words & Music by Matt Redman
CCLI Song # 6016350
© 2011 Thankyou Music
    `.trim();

    await page.fill("#create-item-content", songInput);
    await page.waitForTimeout(100);

    // 5. Verify live counter badge updated
    const badgeText = await page.textContent("#bulk-slide-count-badge");
    expect(badgeText).toContain("3 Slides");

    // Capture screenshot of the Bulk Paste overlay
    await page.screenshot({ path: `${artifactDir}/screenshot_bulk_paste_overlay.png` });

    // 6. Test one-click Clean Chords button
    await page.click("#btn-bulk-clean-chords");
    await page.waitForTimeout(100);

    const cleanedVal = await page.inputValue("#create-item-content");
    expect(cleanedVal).not.toContain("[G]");
    expect(cleanedVal).not.toContain("[D/F#]");
    expect(cleanedVal).not.toContain("(Repeat 2x)");
    expect(cleanedVal).toContain("Bless the Lord O my soul");

    // 7. Click "Convert & Build Slide Deck"
    await page.click("#btn-bulk-to-slides");
    await page.waitForSelector("#studio-bulk-view", { state: "hidden" });
    await page.waitForTimeout(500);

    // 8. Verify slides generated in filmstrip
    const slideCards = await page.$$(".editor-filmstrip-card");
    expect(slideCards.length).toBe(3);

    // 9. Verify Section Tags in Filmstrip
    const tagBadges = await page.$$(".fs-slide-tag-btn");
    expect(tagBadges.length).toBe(3);

    const tag1 = await tagBadges[0].textContent();
    const tag2 = await tagBadges[1].textContent();
    const tag3 = await tagBadges[2].textContent();

    expect(tag1).toContain("V1");
    expect(tag2).toContain("C");
    expect(tag3).toContain("B");

    // 10. Verify CCLI metadata auto-extracted into input fields
    const authorVal = await page.inputValue("#create-item-author");
    const copyrightVal = await page.inputValue("#create-item-copyright");

    expect(authorVal).toBe("Matt Redman");
    expect(copyrightVal).toContain("2011 Thankyou Music");

    // Capture screenshot of resulting slide deck in editor
    await page.screenshot({ path: `${artifactDir}/screenshot_bulk_paste_deck_created.png` });
  }, 35000);
});
