import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, openSlideEditor, type SpawnedTestServer } from "./e2e_helpers";

// Regression: the slide editor's left-sidebar thumbnails drew their text before the card was in the page,
// measured a 0x0 box, and laid it out at full 1080p size (huge fragments of letters). Once drawn properly,
// longer text still overflowed: the autofit's 14px floor is for a 1080p slide, not a 155px thumbnail.
describe("E2E: the slide editor's filmstrip thumbnails", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9034;

  const SLIDES = [
    { tag: "Verse 1", text: "Amazing grace, how sweet the sound" },
    { tag: "Chorus", text: "Bless the Lord, O my soul, O my soul\nWorship His holy name\nSing like never before\nO my soul, I'll worship Your holy name" },
    { tag: "Bridge", text: "Ten thousand reasons for my heart to find a reason to sing your praise forever and ever and ever and ever, Lord" },
  ];

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-filmstrip-e2e-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("each thumbnail shows its whole slide text, small enough to fit", async () => {
    await openSlideEditor(page, "song");
    await page.fill("#create-item-title", "Filmstrip Check");
    await page.click("#btn-open-bulk-paste");
    await page.fill("#create-item-content", SLIDES.map((s) => `${s.tag}\n${s.text}`).join("\n\n"));
    await page.click("#btn-bulk-to-slides");
    await page.waitForSelector("#studio-bulk-view", { state: "hidden" });
    await page.waitForFunction((n) => document.querySelectorAll(".editor-filmstrip-card").length === n, SLIDES.length, { timeout: 8000 });
    // Drawn once the card has a width, so wait for the text rather than assuming it is there.
    await page.waitForFunction((n) => [...document.querySelectorAll(".editor-filmstrip-card")].filter((c) => c.querySelector(".slide-text-inner")).length === n, SLIDES.length, { timeout: 8000 });

    const thumbs = await page.evaluate(() => [...document.querySelectorAll<HTMLElement>(".editor-filmstrip-card")].map((card) => {
      const view = card.querySelector<HTMLElement>("div[style*='aspect-ratio']")!;
      const v = view.getBoundingClientRect();
      const inner = card.querySelector<HTMLElement>(".slide-text-inner")!;
      const r = inner.getBoundingClientRect();
      return {
        text: (inner.innerText || inner.textContent || "").replace(/\s+/g, " ").trim(),
        fontPx: parseFloat(getComputedStyle(inner).fontSize),
        thumb: [Math.round(v.width), Math.round(v.height)],
        // how far the text box sticks out past the thumbnail, per side
        overflow: { top: Math.round(v.top - r.top), bottom: Math.round(r.bottom - v.bottom), left: Math.round(v.left - r.left), right: Math.round(r.right - v.right) },
        textScrollOverflow: inner.scrollHeight - inner.clientHeight,
      };
    }));

    expect(thumbs.length).toBe(SLIDES.length);
    thumbs.forEach((t, i) => {
      const label = `slide ${i + 1}: ${JSON.stringify(t)}`;
      expect(t.text, label).toBe(SLIDES[i].text.replace(/\s+/g, " "));
      // Sized for the thumbnail, not the 48px of a full-size slide (and not shrunk to nothing).
      expect(t.fontPx, label).toBeLessThan(30);
      expect(t.fontPx, label).toBeGreaterThan(2);
      // Nothing cropped: the text box is inside the thumbnail on every side.
      for (const side of ["top", "bottom", "left", "right"] as const) expect(t.overflow[side], `${label} (${side})`).toBeLessThanOrEqual(1);
    });
  }, 40000);
});
