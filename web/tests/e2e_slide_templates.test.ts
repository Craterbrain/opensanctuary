import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Live Test: Save/Apply Slide Template Round Trip", () => {
  let serverProc: Subprocess;
  let browser: Browser;
  let page: Page;
  let testDir: string;
  let DB_PATH: string;
  const PORT = 8996;

  beforeAll(async () => {
    // Isolated OS temp dir: Database::new derives songs/bibles folders from
    // the db path's parent, so a project-root db path (the old behavior
    // here) meant every e2e run silently mutated the real song library.
    testDir = mkdtempSync(join(tmpdir(), "os-next-e2e-"));
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
      stdout: "pipe",
      stderr: "pipe"
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
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
    // Auto-accept the prompt() dialog used for naming a template.
    page.on("dialog", async (dialog) => {
      await dialog.accept("E2E Test Template");
    });
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

  test("Save current slide as template, apply it to a new slide, verify it persists and renders", async () => {
    // 1. Open a new song (default lyric archetype has one TextBlock) and save
    //    it as a template.
    await page.evaluate(() => (window as any).openSlideEditor('song'));
    await page.waitForSelector('#create-modal', { state: 'visible', timeout: 8000 });
    await page.waitForTimeout(500);

    const saveBtn = page.locator('#tb-save-template');
    await saveBtn.click();
    await page.waitForTimeout(500);

    const templatesAfterSave = await fetch(`http://127.0.0.1:${PORT}/api/slide-templates`).then(r => r.json());
    const saved = templatesAfterSave.find((t: any) => t.name === 'E2E Test Template');
    expect(saved).toBeTruthy();
    expect(saved.elements.length).toBeGreaterThan(0);

    // 2. Start a fresh presentation with a blank slide, then apply the
    //    template to replace its content.
    await page.click('#btn-close-create');
    await page.waitForTimeout(300);
    await page.evaluate(() => (window as any).openSlideEditor('presentation'));
    await page.waitForSelector('#create-modal', { state: 'visible', timeout: 8000 });
    await page.waitForTimeout(500);

    const elementCountBeforeApply = await page.evaluate(() =>
      (window as any).getCanvasSlideEditorDebugState().activeSlideElements.length
    );

    const templateSelect = page.locator('#tb-template-select');
    await templateSelect.selectOption({ label: 'E2E Test Template' });
    await page.waitForTimeout(300);

    const stateAfterApply = await page.evaluate(() =>
      (window as any).getCanvasSlideEditorDebugState().activeSlideElements
    );
    expect(stateAfterApply.length).toBe(saved.elements.length);

    // 3. Save this presentation and verify the applied template's elements
    //    persisted, then confirm they actually render (Phase 2's element
    //    renderer) rather than just existing in the data model.
    await page.fill('#create-item-title', 'Template Applied Presentation');
    await page.click('#btn-save-created-item');
    await page.waitForTimeout(800);

    const presentations = await fetch(`http://127.0.0.1:${PORT}/api/presentations`).then(r => r.json());
    const item = presentations.find((p: any) => p.title === 'Template Applied Presentation');
    expect(item).toBeTruthy();
    expect(item.slides[0].elements.length).toBe(saved.elements.length);

    // Go live with it and verify the elements actually render on the in-app
    // Live preview (ties Phase 5 back into Phase 2's renderer end-to-end).
    await fetch(`http://127.0.0.1:${PORT}/api/command`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ AddToSchedule: { item_type: 'presentation', item_id: item.id } })
    });
    const state = await fetch(`http://127.0.0.1:${PORT}/api/state`).then(r => r.json());
    const itemIndex = state.schedule.items.findIndex((it: any) => it.id.includes(item.id) || it.title === 'Template Applied Presentation');
    await fetch(`http://127.0.0.1:${PORT}/api/command`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ GoLive: { item_index: itemIndex, slide_index: 0 } })
    });
    await page.waitForTimeout(500);

    const renderedCount = await page.evaluate(() =>
      document.querySelectorAll('#live-canvas-elements .slide-canvas-element').length
    );
    expect(renderedCount).toBe(saved.elements.length);
  }, 30000);
});
