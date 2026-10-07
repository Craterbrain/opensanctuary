import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: Interactive Image Crop Tool", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 8997;

  beforeAll(async () => {
    // Isolated OS temp dir: Database::new derives songs/bibles folders from
    // the db path's parent, so a project-root db path (the old behavior
    // here) meant every e2e run silently mutated the real song library.
    server = await spawnTestServer({ port: PORT });

    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
    // Not waitForLoadState("networkidle") -- the app keeps a persistent
    // WebSocket open once loaded (time sync), so "no network activity for
    // 500ms" can time out unpredictably instead of ever firing.
    // __APP_READY__ above already guarantees app_core.ts/app_ui.ts finished
    // loading and the WS connected.
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
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
