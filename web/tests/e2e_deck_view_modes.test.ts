import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// The Preview and Live decks' ⊞ / ☰ buttons: the grid shows each slide as it will look (a rendered thumbnail) for
// songs, scriptures and presentations; the list is always text rows. For songs and scriptures the grid used to be
// text cards too, so the two buttons looked like they did nothing.
describe("E2E: Preview deck view modes (grid = rendered slides, list = text rows)", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9035;
  const base = `http://127.0.0.1:${PORT}`;
  const slide = (text: string, label: string) => ({ text, header: null, label, background: null });

  // Library writes and show commands as the console itself (its host token and session id).
  const asConsole = (method: string, path: string, body: unknown) => page.evaluate(async ([method, path, body]) => {
    const j = await (await fetch("/api/internal/host-token")).json();
    const tok = j.token ?? j.host_token;
    const r = await fetch(path as string, { method: method as string, headers: { "Content-Type": "application/json", "x-host-token": tok, "x-console-session-id": sessionStorage.getItem("os_console_session_id") ?? "" }, body: JSON.stringify(body) });
    return r.status;
  }, [method, path, body] as const);
  const deck = () => page.evaluate(() => {
    const m = document.getElementById("preview-slide-matrix")!;
    return { mode: m.classList.contains("list-mode") ? "list" : "matrix", thumbs: m.querySelectorAll(".slide-thumb-card").length, textCards: m.querySelectorAll(".slide-card:not(.slide-thumb-card)").length,
      active: document.querySelector("#btn-preview-view-matrix.active") ? "matrix" : document.querySelector("#btn-preview-view-list.active") ? "list" : "none",
      badges: [...m.querySelectorAll(".slide-badge-pill, .slide-badge, .slide-card .badge")].map((b) => (b.textContent ?? "").trim()).filter(Boolean) };
  });

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-deck-modes-e2e-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
    await page.goto(`${base}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
    expect(await asConsole("POST", "/api/songs", { id: "s1", title: "Song", author: "A", copyright: null, ccli_number: null, slides: [slide("Amazing grace, how sweet the sound", "Verse 1"), slide("Bless the Lord, O my soul", "Chorus")] })).toBe(200);
    expect(await asConsole("POST", "/api/scriptures", { id: "sc1", book: "John", chapter: 3, verse_start: 16, verse_end: 17, version: "KJV", reference: "John 3:16-17", verses: [{ verse_number: 16, text: "For God so loved the world" }, { verse_number: 17, text: "For God sent not his Son" }] })).toBe(200);
    expect(await asConsole("POST", "/api/presentations", { id: "p1", title: "Notices", author: "", slides: [slide("Welcome", "1"), slide("Potluck on Sunday", "2")] })).toBe(200);
    for (const [type, id] of [["song", "s1"], ["scripture", "sc1"], ["presentation", "p1"]]) await asConsole("POST", "/api/command", { AddToSchedule: { item_type: type, item_id: id } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  for (const [index, type] of [[0, "song"], [1, "scripture"], [2, "presentation"]] as const) {
    test(`a ${type}: the grid button shows rendered slides, the list button text rows`, async () => {
      await asConsole("POST", "/api/command", { StageItem: { item_index: index, slide_index: 0 } });
      await pollUntil(async () => (await (await fetch(`${base}/api/state`)).json()).state.staged_item?.item_type, (t) => t === type, 8000, `${type} staged`);

      await page.click("#btn-preview-view-matrix");
      const grid = await pollUntil(deck, (d) => d.thumbs === 2, 8000, "grid thumbnails");
      expect(grid).toMatchObject({ mode: "matrix", active: "matrix", thumbs: 2, textCards: 0 });
      expect(grid.badges.length).toBeGreaterThanOrEqual(2); // each rendered slide keeps its section badge (Verse 1, V16, 1, ...)

      await page.click("#btn-preview-view-list");
      const list = await pollUntil(deck, (d) => d.textCards === 2, 8000, "list text rows");
      expect(list).toMatchObject({ mode: "list", active: "list", thumbs: 0, textCards: 2 });

      await page.click("#btn-preview-view-matrix"); // and back
      expect(await pollUntil(deck, (d) => d.thumbs === 2, 8000, "grid again")).toMatchObject({ mode: "matrix", active: "matrix" });
    }, 30000);
  }
});
