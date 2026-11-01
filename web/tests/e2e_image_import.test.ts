import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, writeFileSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";
import { spawnTestServer, teardownTestServer, openSlideEditor, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// A real 2x2 PNG (the server checks content, not the file name).
const PNG_2X2 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR4nGP8z8Dwn4GBgYGJAQoAHhgCAh6X4CYAAAAASUVORK5CYII=",
  "base64",
);

describe("E2E Live Test: importing an image from the computer in the slide editor", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  let pngPath: string;
  const PORT = 9011;

  beforeAll(async () => {
    pngPath = join(mkdtempSync(join(tmpdir(), "os-next-img-")), "My Photo.png");
    writeFileSync(pngPath, PNG_2X2);
    server = await spawnTestServer({ port: PORT });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    page.on("pageerror", err => console.log("PAGE ERR:", err.message));
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("Insert Image opens the file chooser, stores the picture and puts it on the slide", async () => {
    await openSlideEditor(page, "presentation");
    const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.click("#tb-insert-image")]);
    await chooser.setFiles(pngPath);
    await page.waitForSelector(".editor-slide-canvas-stage .slide-canvas-element-image", { timeout: 8000 });

    const media = await pollUntil(
      () => fetch(`http://127.0.0.1:${PORT}/api/media`).then(r => r.json()),
      (m: any[]) => m.some(x => /^\/media\/images\/My-Photo_[0-9a-f]{8}\.png$/.test(x.file_path)),
      8000, "the uploaded picture in the media library");
    expect(media.length).toBe(1);
  }, 30000);

  test("the background picker imports from the computer, selects it, and applies it; re-importing reuses the file", async () => {
    await page.click("#tb-background");
    await page.waitForSelector("#media-image-picker-modal", { state: "visible" });
    for (let i = 0; i < 2; i++) {
      const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.click("#btn-image-picker-import")]);
      await chooser.setFiles(pngPath);
      await page.waitForTimeout(800);
    }
    const cards = await page.$$eval(".media-image-card", els => els.map(e => e.classList.contains("selected")));
    expect(cards).toEqual([true]); // one picture, already selected
    await page.click("#btn-apply-image-slide");
    const bg = await page.$eval(".editor-slide-canvas-stage", el => (el as HTMLElement).style.background || (el as HTMLElement).style.backgroundImage);
    expect(bg).toContain("/media/images/My-Photo_");
  }, 30000);

  test("the server refuses anything that is not a picture, and anonymous callers", async () => {
    const url = `http://127.0.0.1:${PORT}/api/media/upload?name=x.png`;
    expect((await fetch(url, { method: "POST", body: PNG_2X2 })).status).toBe(401);
    const token = await page.evaluate(() => (window as any).__hostToken ?? sessionStorage.getItem("os-host-token"));
    if (token) {
      const bad = await fetch(url, { method: "POST", headers: { "x-host-token": token }, body: "<svg onload='x()'/>" });
      expect(bad.status).toBe(415);
    }
  });
});
