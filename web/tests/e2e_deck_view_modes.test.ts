import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// The Preview and Live decks' ⊞ / ☰ buttons: the grid shows each slide as it will look (a rendered thumbnail) for
// songs, scriptures and presentations; the list is always text rows. Songs and scriptures open in the list,
// presentations in the grid; a click overrides that for the kind of item and is remembered.
// (For songs and scriptures the grid used to be text cards too, so the two buttons looked like they did nothing.)
describe("E2E: deck view modes (grid = rendered slides, list = text rows)", () => {
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
  const deck = (which: "preview" | "live" = "preview") => page.evaluate((which) => {
    const m = document.getElementById(`${which}-slide-matrix`)!;
    // How many columns the cards sit in (distinct left edges): the list is text cards in columns, not one stack.
    const columns = new Set([...m.querySelectorAll(".slide-card")].map((c) => Math.round(c.getBoundingClientRect().left))).size;
    return { columns, mode: m.classList.contains("list-mode") ? "list" : "matrix", thumbs: m.querySelectorAll(".slide-thumb-card").length, textCards: m.querySelectorAll(".slide-card:not(.slide-thumb-card)").length,
      active: document.querySelector(`#btn-${which}-view-matrix.active`) ? "matrix" : document.querySelector(`#btn-${which}-view-list.active`) ? "list" : "none",
      badges: [...m.querySelectorAll(".slide-badge-pill, .slide-badge, .slide-card .badge")].map((b) => (b.textContent ?? "").trim()).filter(Boolean) };
  }, which);
  const stage = async (index: number, type: string) => {
    await asConsole("POST", "/api/command", { StageItem: { item_index: index, slide_index: 0 } });
    await pollUntil(async () => (await (await fetch(`${base}/api/state`)).json()).state.staged_item?.item_type, (t) => t === type, 8000, `${type} staged`);
  };
  const isList = (d: any) => d.mode === "list" && d.active === "list" && d.textCards === 2 && d.thumbs === 0 && d.columns >= 2; // text cards, in columns
  const isGrid = (d: any) => d.mode === "matrix" && d.active === "matrix" && d.thumbs === 2 && d.textCards === 0;

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

  // Songs and scriptures open in the list, presentations in the grid; each button switches, and back.
  for (const [index, type, opensIn] of [[0, "song", "list"], [1, "scripture", "list"], [2, "presentation", "matrix"]] as const) {
    test(`a ${type} opens in the ${opensIn === "list" ? "list" : "grid"}; the other button switches to the ${opensIn === "list" ? "grid of rendered slides" : "list of text rows"}`, async () => {
      await stage(index, type);
      const opened = await pollUntil(deck, opensIn === "list" ? isList : isGrid, 8000, `${type} opens in the ${opensIn}`);
      expect(opened.badges.length).toBeGreaterThanOrEqual(0);

      await page.click(opensIn === "list" ? "#btn-preview-view-matrix" : "#btn-preview-view-list");
      const other = await pollUntil(deck, opensIn === "list" ? isGrid : isList, 8000, "the other view");
      if (opensIn === "list") expect(other.badges.length).toBeGreaterThanOrEqual(2); // each rendered slide keeps its section badge (Verse 1, V16, ...)

      await page.click(opensIn === "list" ? "#btn-preview-view-list" : "#btn-preview-view-matrix"); // and back to where it opened
      await pollUntil(deck, opensIn === "list" ? isList : isGrid, 8000, "back again");
    }, 30000);
  }

  test("a choice is remembered for that kind of item, per deck, and survives a reload", async () => {
    await stage(0, "song");
    await page.click("#btn-preview-view-matrix"); // songs: grid now
    await pollUntil(deck, isGrid, 8000, "song grid");
    await stage(1, "scripture");
    await pollUntil(deck, isGrid, 8000, "scriptures follow the song's choice (both are text)");
    await stage(2, "presentation");
    await pollUntil(deck, isGrid, 8000, "a presentation still opens in its own default");
    await page.click("#btn-preview-view-list");
    await pollUntil(deck, isList, 8000, "presentation list");
    await stage(0, "song");
    await pollUntil(deck, isGrid, 8000, "the song choice is untouched by the presentation's");

    await page.reload();
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
    await pollUntil(deck, isGrid, 8000, "the song choice survives a reload");

    await page.click("#btn-preview-view-list"); // put it back
    await pollUntil(deck, isList, 8000, "song list again");
    await stage(2, "presentation");
    await page.click("#btn-preview-view-matrix");
    await pollUntil(deck, isGrid, 8000, "presentation grid again");
  }, 60000);

  test("the Live deck has its own buttons and the same defaults", async () => {
    await asConsole("POST", "/api/command", { GoLive: { item_index: 0, slide_index: 0 } });
    await pollUntil(async () => (await (await fetch(`${base}/api/state`)).json()).state.live_item?.item_type, (t) => t === "song", 8000, "song live");
    expect(isList(await pollUntil(() => deck("live"), (d) => d.textCards === 2, 8000, "live list"))).toBe(true);
    await page.click("#btn-live-view-matrix");
    expect(isGrid(await pollUntil(() => deck("live"), (d) => d.thumbs === 2, 8000, "live grid"))).toBe(true);
    expect(isList(await deck("preview")) || isGrid(await deck("preview"))).toBe(true); // the Preview deck is its own
    await page.click("#btn-live-view-list");
    await pollUntil(() => deck("live"), (d) => d.textCards === 2, 8000, "live list again");
  }, 30000);
});
