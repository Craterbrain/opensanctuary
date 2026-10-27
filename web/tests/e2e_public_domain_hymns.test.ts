import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

// The public-domain hymn download, from the Import dialog and from the first-run wizard. A local web server stands in for GitHub.
const hymn = (id: string, title: string, author: string) => ({
  id, title, author, copyright: "Public Domain", ccli_number: null, theme_name: null,
  slides: [{ id: `${id}-1`, text: "Amazing grace how sweet the sound that saved a wretch like me", label: "Verse 1" }],
});

describe("E2E: public-domain hymn library", () => {
  const PORT = 9089;
  let server: SpawnedTestServer;
  let fake: ReturnType<typeof Bun.serve>;
  let browser: Browser;
  let page: Page;
  let hymns = [hymn("pd-ag", "Amazing Grace", "John Newton"), hymn("pd-hh", "Holy, Holy, Holy", "Reginald Heber")];

  beforeAll(async () => {
    fake = Bun.serve({ port: 0, fetch: () => Response.json(hymns) });
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-pd-hymns-", env: { OPENSANCTUARY_HYMNS_URL: `http://127.0.0.1:${fake.port}/hymns.json` } });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
    await page.goto(`http://127.0.0.1:${PORT}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 60000);
  afterAll(async () => { if (browser) await browser.close(); fake?.stop(true); await teardownTestServer(server); });

  const songs = async (): Promise<any[]> => (await fetch(`http://127.0.0.1:${PORT}/api/songs`)).json();

  test("nothing is in a new library until you ask; the Import button downloads the hymns once", async () => {
    expect(await songs()).toHaveLength(0);
    await page.evaluate(() => (window as any).openImportModal());
    await page.waitForSelector("#import-modal", { state: "visible", timeout: 8000 });
    await page.click("#btn-add-public-domain");
    await page.waitForFunction(() => /^Added 2 hymns/.test(document.getElementById("public-domain-status")?.textContent || ""), null, { timeout: 20000 });
    const after = await songs();
    expect(after.map((s) => s.title).sort()).toEqual(["Amazing Grace", "Holy, Holy, Holy"]);
    expect(after.every((s) => s.copyright === "Public Domain")).toBe(true);

    await page.click("#btn-add-public-domain");
    await page.waitForFunction(() => /Nothing to add/.test(document.getElementById("public-domain-status")?.textContent || ""), null, { timeout: 20000 });
    expect(await songs()).toHaveLength(2);
    await page.keyboard.press("Escape");
  }, 60000);

  test("the first-run wizard's Songs step offers the download, and a bad download is reported, not stored", async () => {
    hymns = [hymn("pd-new", "Be Thou My Vision", "Eleanor Hull"), { ...hymn("bad", "A Modern Chorus", "Somebody"), copyright: "© 2001 Somebody Music" }];
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('About')").click();
    await page.locator("button:has-text('Re-run First-Time Setup')").click();
    await page.waitForSelector("#first-time-setup-body", { state: "visible", timeout: 5000 });
    await page.click("#btn-first-time-next");
    for (let i = 1; i < 4; i++) await page.click("#btn-first-time-skip"); // data-dir, church-name, bibles -> songs
    await page.waitForSelector("#fts-songs-download", { state: "visible", timeout: 5000 });

    await page.click("#fts-songs-download");
    await page.waitForFunction(() => /isn't marked public domain/.test(document.getElementById("fts-songs-download-status")?.textContent || ""), null, { timeout: 20000 });
    expect(await songs()).toHaveLength(2);

    hymns = [hymn("pd-new", "Be Thou My Vision", "Eleanor Hull")];
    await page.click("#fts-songs-download");
    await page.waitForFunction(() => /^Added 1 hymn\./.test(document.getElementById("fts-songs-download-status")?.textContent || ""), null, { timeout: 20000 });
    expect((await songs()).map((s) => s.title)).toContain("Be Thou My Vision");
  }, 60000);
});
