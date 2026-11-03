import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// Hiding (or showing) a slide of the item already on the Preview or Live deck re-renders that deck: the card is
// dimmed at once and stops being clickable. (The decks skip re-rendering when their content key is unchanged, and
// the hidden flag used to be missing from that key, so the card stayed as it was until something else changed.)
describe("E2E: hiding a slide updates the Preview and Live decks", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9091;
  const base = `http://127.0.0.1:${PORT}`;
  const slide = (text: string, label: string) => ({ text, header: null, label, background: null });

  const asConsole = (method: string, path: string, body: unknown) => page.evaluate(async ([method, path, body]) => {
    const j = await (await fetch("/api/internal/host-token")).json();
    const tok = j.token ?? j.host_token;
    const r = await fetch(path as string, { method: method as string, headers: { "Content-Type": "application/json", "x-host-token": tok, "x-console-session-id": sessionStorage.getItem("os_console_session_id") ?? "" }, body: JSON.stringify(body) });
    return r.status;
  }, [method, path, body] as const);
  const hiddenCards = (which: "preview" | "live") => page.evaluate((which) =>
    [...document.querySelectorAll(`#${which}-slide-matrix .slide-card`)].map((c) => c.classList.contains("slide-hidden")), which);

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-hidden-decks-e2e-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${base}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
    expect(await asConsole("POST", "/api/songs", { id: "s1", title: "Song", author: "A", copyright: null, ccli_number: null, slides: [slide("Amazing grace", "Verse 1"), slide("How sweet the sound", "Verse 2"), slide("That saved a wretch", "Verse 3")] })).toBe(200);
    await asConsole("POST", "/api/command", { AddToSchedule: { item_type: "song", item_id: "s1" } });
    await pollUntil(async () => (await (await fetch(`${base}/api/state`)).json()).schedule.items.length, (n) => n === 1, 8000, "song scheduled");
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("the staged item's card dims when its slide is hidden, and clears when shown again", async () => {
    await asConsole("POST", "/api/command", { StageItem: { item_index: 0, slide_index: 0 } });
    await page.waitForFunction(() => document.querySelectorAll("#preview-slide-matrix .slide-card").length === 3, null, { timeout: 8000 });
    expect(await hiddenCards("preview")).toEqual([false, false, false]);

    await asConsole("POST", "/api/command", { SetSlideHidden: { item_index: 0, slide_index: 1, hidden: true } });
    await page.waitForFunction(() => document.querySelectorAll("#preview-slide-matrix .slide-card")[1]?.classList.contains("slide-hidden"), null, { timeout: 8000 });
    expect(await hiddenCards("preview")).toEqual([false, true, false]);

    await asConsole("POST", "/api/command", { SetSlideHidden: { item_index: 0, slide_index: 1, hidden: false } });
    await page.waitForFunction(() => !document.querySelectorAll("#preview-slide-matrix .slide-card")[1]?.classList.contains("slide-hidden"), null, { timeout: 8000 });
  }, 30000);

  test("the live item's card dims too, and clicking it no longer changes the live slide", async () => {
    await asConsole("POST", "/api/command", { GoLive: { item_index: 0, slide_index: 0 } });
    await page.waitForFunction(() => document.querySelectorAll("#live-slide-matrix .slide-card").length === 3, null, { timeout: 8000 });

    await asConsole("POST", "/api/command", { SetSlideHidden: { item_index: 0, slide_index: 2, hidden: true } });
    await page.waitForFunction(() => document.querySelectorAll("#live-slide-matrix .slide-card")[2]?.classList.contains("slide-hidden"), null, { timeout: 8000 });
    expect(await hiddenCards("live")).toEqual([false, false, true]);

    await page.locator("#live-slide-matrix .slide-card").nth(2).click();
    await page.waitForTimeout(300);
    expect((await (await fetch(`${base}/api/state`)).json()).state.live_slide_index).toBe(0);
  }, 30000);
});
