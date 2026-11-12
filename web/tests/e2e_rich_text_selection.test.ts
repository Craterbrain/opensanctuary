import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, openSlideEditor, type SpawnedTestServer } from "./e2e_helpers";

// Verifies per-run rich text editing works against a REAL browser Selection/
// Range API — a hand-mocked DOM (as other editor tests use) can't exercise
// genuine partial-text selection splitting, so this needs a live browser.
// Entering edit mode focuses the box and selects all its text a moment later (a timer in text_block.ts).
// A selection the test makes before then is replaced by that one -- which, on a loaded machine, is when
// this test used to fail. The focus and the select-all happen together, so the focus says it's done.
async function editModeSettled(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const box = document.querySelector('.editor-slide-canvas-stage .slide-canvas-element[contenteditable="true"]');
    return !!box && document.activeElement === box;
  }, null, { timeout: 4000 });
}

describe("E2E Live Test: Per-Run Rich Text Selection Splitting", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 8998;

  beforeAll(async () => {
    // Isolated OS temp dir: Database::new derives songs/bibles folders from
    // the db path's parent, so a project-root db path (the old behavior
    // here) meant every e2e run silently mutated the real song library.
    server = await spawnTestServer({ port: PORT });

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage();
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
    // Not waitForLoadState("networkidle") -- the app's persistent time-sync
    // WebSocket means "idle" may never fire; __APP_READY__ above is the
    // real readiness signal (see main.ts).
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("Bolding a mid-word selection splits the run without losing the rest of the text", async () => {
    await openSlideEditor(page, 'song');

    // Enter edit mode on the lyric text box via a real double-click. On a busy machine the first
    // double-click can land before the editor has wired its handlers, so try again until it takes.
    const editable = '.editor-slide-canvas-stage .slide-canvas-element[contenteditable="true"]';
    for (let attempt = 0; attempt < 5; attempt++) {
      await page.dblclick('.editor-slide-canvas-stage .slide-canvas-element');
      if (await page.waitForSelector(editable, { timeout: 4000 }).then(() => true, () => false)) break;
    }
    await page.waitForSelector(editable, { timeout: 4000 });
    await editModeSettled(page);

    const editingId = await page.evaluate(() =>
      (document.querySelector('.editor-slide-canvas-stage .slide-canvas-element[contenteditable="true"]') as HTMLElement | null)?.dataset.elementId ?? null
    );
    expect(editingId).not.toBeNull();

    // The element being edited, by id: the slide can hold more than one text box.
    const runsOf = (id: string) => page.evaluate((id) => {
      const state = (window as any).getCanvasSlideEditorDebugState();
      return state.activeSlideElements.find((e: any) => e.id === id).block.runs;
    }, id);
    const before = (await runsOf(editingId!)).map((r: any) => r.text).join('');
    expect(before.length).toBeGreaterThan(0);

    // Select the word "sweet" within the single run's text node via a real
    // browser Range/Selection, then apply bold — exercising applyStyleToSelection
    // against actual DOM Range semantics (not a mock).
    const selectionMade = await page.evaluate(({ wordToSelect, id }: { wordToSelect: string; id: string }) => {
      // Inside the text box being edited -- the style applies only to a selection there.
      const node = document.querySelector(`.editor-slide-canvas-stage [data-element-id="${id}"].slide-element-text-content`) as HTMLElement | null;
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
    }, { wordToSelect: 'sweet', id: editingId! });
    expect(selectionMade).toBe(true);

    await page.evaluate(() => (window as any).__debugApplyTextStyle({ bold: true }));
    await page.waitForFunction((id) => (window as any).getCanvasSlideEditorDebugState().activeSlideElements.find((e: any) => e.id === id).block.runs.some((r: any) => r.bold), editingId, { timeout: 10000 });

    const after = await runsOf(editingId!);

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
  }, 60000);

  test("Bold with the whole box selected, as edit mode leaves it, bolds the whole box", async () => {
    await openSlideEditor(page, 'song');
    const editable = '.editor-slide-canvas-stage .slide-canvas-element[contenteditable="true"]';
    for (let attempt = 0; attempt < 5; attempt++) {
      await page.dblclick('.editor-slide-canvas-stage .slide-canvas-element');
      if (await page.waitForSelector(editable, { timeout: 4000 }).then(() => true, () => false)) break;
    }
    await editModeSettled(page);
    const id = await page.evaluate((sel) => (document.querySelector(sel) as HTMLElement).dataset.elementId!, editable);

    // Restyling the whole box re-renders it; the box being replaced used to write its old, unstyled text
    // back into the slide as it went (its blur), undoing the Bold.
    await page.evaluate(() => (window as any).__debugApplyTextStyle({ bold: true }));
    const runs = await page.evaluate((id) => (window as any).getCanvasSlideEditorDebugState().activeSlideElements.find((e: any) => e.id === id).block.runs, id);
    expect(runs.length).toBeGreaterThan(0);
    expect(runs.every((r: any) => r.bold)).toBe(true);
  }, 60000);
});
