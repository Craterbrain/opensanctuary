import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

// Verifies per-run rich text editing works against a REAL browser Selection/
// Range API — a hand-mocked DOM (as other editor tests use) can't exercise
// genuine partial-text selection splitting, so this needs a live browser.
describe("E2E Live Test: Per-Run Rich Text Selection Splitting", () => {
  let serverProc: Subprocess;
  let browser: Browser;
  let page: Page;
  let testDir: string;
  let DB_PATH: string;
  const PORT = 8998;

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
    page = await browser.newPage();
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

  test("Bolding a mid-word selection splits the run without losing the rest of the text", async () => {
    await page.evaluate(() => (window as any).openSlideEditor('song'));
    await page.waitForSelector('#create-modal', { state: 'visible', timeout: 8000 });
    await page.waitForTimeout(800);

    // Enter edit mode on the lyric text box via a real double-click.
    await page.dblclick('.editor-slide-canvas-stage .slide-canvas-element');
    await page.waitForTimeout(200);

    const editingId = await page.evaluate(() =>
      (document.querySelector('.editor-slide-canvas-stage .slide-canvas-element[contenteditable="true"]') as HTMLElement | null)?.dataset.elementId ?? null
    );
    expect(editingId).not.toBeNull();

    const before = await page.evaluate(() => {
      const state = (window as any).getCanvasSlideEditorDebugState();
      return state.activeSlideElements[0].block.runs.map((r: any) => r.text).join('');
    });
    expect(before.length).toBeGreaterThan(0);

    // Select the word "sweet" within the single run's text node via a real
    // browser Range/Selection, then apply bold — exercising applyStyleToSelection
    // against actual DOM Range semantics (not a mock).
    const selectionMade = await page.evaluate((wordToSelect: string) => {
      const node = document.querySelector('.editor-slide-canvas-stage .slide-canvas-element.slide-element-text-content') as HTMLElement | null;
      if (!node) return false;
      const span = node.querySelector('span');
      const textNode = span?.firstChild;
      if (!textNode || textNode.nodeType !== Node.TEXT_NODE) return false;
      const fullText = textNode.textContent || '';
      const idx = fullText.indexOf(wordToSelect);
      if (idx < 0) return false;
      const range = document.createRange();
      range.setStart(textNode, idx);
      range.setEnd(textNode, idx + wordToSelect.length);
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
      return true;
    }, 'sweet');
    expect(selectionMade).toBe(true);

    await page.evaluate(() => (window as any).__debugApplyTextStyle({ bold: true }));
    await page.waitForTimeout(100);

    const after = await page.evaluate(() => {
      const state = (window as any).getCanvasSlideEditorDebugState();
      return state.activeSlideElements[0].block.runs;
    });

    // Lossless: concatenating all runs' text reproduces the original string.
    const concatenated = after.map((r: any) => r.text).join('');
    expect(concatenated).toBe(before);

    // At least 3 runs (before/selected/after), with exactly one run — the
    // selected word — marked bold, and it's not the entire original text.
    expect(after.length).toBeGreaterThanOrEqual(3);
    const boldRuns = after.filter((r: any) => r.bold);
    expect(boldRuns.length).toBe(1);
    expect(boldRuns[0].text).toBe('sweet');
    expect(boldRuns[0].text).not.toBe(before);
  }, 20000);
});
