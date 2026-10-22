import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, openSlideEditor, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// Regression: "Save to Schedule" saved scriptures and presentations to the library but never
// scheduled them. Closing the editor reset its type to "song" before AddToSchedule was sent, so
// the server was asked for a song with that id, found none, and the command did nothing.
describe("E2E: the slide editor's Save to Schedule, for every item type", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9031;
  const base = `http://127.0.0.1:${PORT}`;
  const scheduled = async () => (await (await fetch(`${base}/api/state`)).json()).schedule.items.map((i: any) => `${i.item_type}:${i.title}`);

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-save-to-schedule-e2e-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${base}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 15000 });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  for (const [kind, title, text] of [
    ["scripture", "Romans 8:28-29", "28 And we know that all things work together for good\n\n29 For whom he did foreknow"],
    ["presentation", "Notices", "Welcome\n\nPotluck on Sunday"],
    ["song", "Test Hymn", "Verse 1\nline one\n\nChorus\nline two"],
  ] as const) {
    test(`a ${kind} lands in the library and the schedule`, async () => {
      const before = (await scheduled()).length;
      await openSlideEditor(page, kind);
      await page.fill("#create-item-title", title);
      await page.click("#btn-open-bulk-paste");
      await page.fill("#create-item-content", text);
      await page.click("#btn-bulk-to-slides");
      await page.waitForSelector("#studio-bulk-view", { state: "hidden" });
      await page.click("#btn-save-to-schedule");

      const items = await pollUntil(scheduled, (s) => s.length === before + 1, 8000, `${kind} in the schedule`);
      expect(items.at(-1)).toBe(`${kind}:${title}`);
      const lib = await (await fetch(`${base}/api/${kind === "song" ? "songs" : kind === "scripture" ? "scriptures" : "presentations"}`)).json();
      expect(lib.some((x: any) => (x.title ?? x.reference) === title)).toBe(true);
    }, 30000);
  }
});
