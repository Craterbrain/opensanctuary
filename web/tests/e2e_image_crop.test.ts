import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Live Test: Interactive Image Crop Tool", () => {
  let serverProc: Subprocess;
  let browser: Browser;
  let page: Page;
  let testDir: string;
  let DB_PATH: string;
  const PORT = 8997;

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

  test("Dragging a crop handle persists a new CropRect", async () => {
    await page.evaluate(() => (window as any).openSlideEditor('presentation'));
    await page.waitForSelector('#create-modal', { state: 'visible', timeout: 8000 });
    await page.waitForTimeout(500);

    const inserted = await page.evaluate(() => (window as any).__debugInsertElement({
      type: 'Image',
      id: 'img-crop-test',
      transform: { x: 0.1, y: 0.1, w: 0.6, h: 0.6, rotation_deg: 0, z_index: 5, locked: false, opacity: 1 },
      file_path: 'media/images/sunset-beach-waves.jpg',
      crop: { x: 0, y: 0, w: 1, h: 1 }
    }));
    expect(inserted).toBe(true);
    await page.waitForTimeout(300);

    // Right-click the image element — mousedown-before-contextmenu selects it,
    // satisfying the context menu's isSingleImage check.
    await page.click('.editor-slide-canvas-stage .slide-canvas-element-image', { button: 'right' });
    await page.waitForTimeout(150);

    const cropMenuItem = page.locator('.editor-context-menu >> text=Crop Image');
    await cropMenuItem.waitFor({ state: 'visible', timeout: 5000 });
    await cropMenuItem.click();
    await page.waitForTimeout(300);

    await page.waitForSelector('.editor-image-crop-overlay', { state: 'visible', timeout: 5000 });

    // Drag the SE handle inward to shrink the crop window.
    const seHandleBox = await page.evaluate(() => {
      const overlay = document.querySelector('.editor-image-crop-overlay') as HTMLElement;
      const handles = Array.from(overlay.querySelectorAll('div')).filter(d => (d as HTMLElement).style.cursor === 'nwse-resize' && (d as HTMLElement).style.top !== '-4.5px');
      const se = handles[handles.length - 1] as HTMLElement | undefined;
      if (!se) return null;
      const r = se.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    });
    expect(seHandleBox).not.toBeNull();

    await page.mouse.move(seHandleBox!.x, seHandleBox!.y);
    await page.mouse.down();
    await page.mouse.move(seHandleBox!.x - 60, seHandleBox!.y - 40, { steps: 10 });
    await page.mouse.up();
    await page.waitForTimeout(200);

    await page.click('.editor-image-crop-overlay button');
    await page.waitForTimeout(200);

    await page.fill('#create-item-title', 'Crop Test Presentation');
    await page.click('#btn-save-created-item');
    await page.waitForTimeout(800);

    const saved = await fetch(`http://127.0.0.1:${PORT}/api/presentations`).then(r => r.json());
    const list = saved.presentations || saved || [];
    const item = list.find((p: any) => p.title === 'Crop Test Presentation');
    expect(item).toBeTruthy();
    const el = item.slides[0].elements.find((e: any) => e.id === 'img-crop-test');
    expect(el).toBeTruthy();
    expect(el.crop).toBeTruthy();
    const cropChanged = el.crop.x !== 0 || el.crop.y !== 0 || el.crop.w !== 1 || el.crop.h !== 1;
    expect(cropChanged).toBe(true);
  }, 30000);
});
