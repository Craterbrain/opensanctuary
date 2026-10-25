import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, mkdirSync, writeFileSync, readdirSync, readFileSync, existsSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// OpenSanctuary's own files (.ossong .ospres .osservice .osbible), driven the way a person would: right-click a song and
// save it, then open the file in a second, empty library. Copyrighted songs get an advisory pop-up with a Continue button
// (never a blocker); a file whose songs are public domain, or already in the library, opens without one.
describe("E2E: saving and opening OpenSanctuary files between two libraries", () => {
  // A real 1x1 PNG, so the browser can draw the background without complaint.
  const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", "base64");
  const A = 9440;
  const B = 9441;
  const slide = (text: string, background: string | null = null) => ({ text, header: null, label: "Verse 1", background });

  let a: SpawnedTestServer;
  let b: SpawnedTestServer;
  let browser: Browser;
  let pageA: Page;
  let pageB: Page;
  let mediaA: string;
  let mediaB: string;
  let downloads: string;

  const asConsole = (page: Page, method: string, path: string, body: unknown) =>
    page.evaluate(async ([method, path, body]) => {
      const j = await (await fetch("/api/internal/host-token")).json();
      const tok = j.token ?? j.host_token;
      const r = await fetch(path as string, { method: method as string, headers: { "Content-Type": "application/json", "x-host-token": tok, "x-console-session-id": sessionStorage.getItem("os_console_session_id") ?? "" }, body: JSON.stringify(body) });
      return r.status;
    }, [method, path, body] as const);

  const openPage = async (port: number) => {
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 }, acceptDownloads: true });
    await page.goto(`http://127.0.0.1:${port}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
    return page;
  };

  /** Right-click `id` in a library tab and click "Save as OpenSanctuary File"; resolves to the downloaded file's path. */
  const saveFromLibrary = async (page: Page, tab: string, id: string): Promise<{ path: string; name: string }> => {
    await closeDialogs(page);
    await page.click(`.tab-btn[data-tab="${tab}"]`);
    const row = page.locator(`#catalog-table-body tr[data-id="${id}"], .catalog-grid-card[data-id="${id}"]`).first();
    await row.waitFor({ state: "visible", timeout: 10000 });
    await row.click({ button: "right" });
    const item = page.locator("#ctx-lib-export-bundle");
    await item.waitFor({ state: "visible", timeout: 5000 });
    const [download] = await Promise.all([page.waitForEvent("download", { timeout: 15000 }), item.click()]);
    const path = join(downloads, download.suggestedFilename());
    await download.saveAs(path);
    return { path, name: download.suggestedFilename() };
  };

  const openImportDialog = async (page: Page) => {
    await closeDialogs(page);
    await clearToasts(page);
    await page.evaluate(() => (window as any).openImportModal());
    await page.waitForSelector("#import-modal", { state: "visible", timeout: 8000 });
  };

  const toastWith = (page: Page, text: string) => page.locator(".os-toast", { hasText: text }).first().waitFor({ state: "visible", timeout: 15000 });
  // Toasts from earlier steps stay in the DOM, so a wait for "Import cancelled" would match an old one and race
  // ahead of the app; clear them before each action that should produce a new one.
  const clearToasts = (page: Page) => page.evaluate(() => document.querySelectorAll(".os-toast").forEach((t) => t.remove()));
  const closeDialogs = async (page: Page) => {
    for (let i = 0; i < 3; i++) await page.keyboard.press("Escape");
    await page.waitForFunction(() => [...document.querySelectorAll<HTMLElement>(".modal-backdrop")].every((m) => getComputedStyle(m).display === "none"), null, { timeout: 5000 });
  };
  const songsIn = async (port: number) => (await (await fetch(`http://127.0.0.1:${port}/api/songs`)).json()) as any[];
  const filesIn = (dir: string): string[] => (existsSync(dir) ? readdirSync(dir) : []);
  const advisoryVisible = (page: Page) => page.evaluate(() => getComputedStyle(document.getElementById("bundle-advisory-modal")!).display !== "none");

  beforeAll(async () => {
    const root = mkdtempSync(join(tmpdir(), "os-next-bundles-e2e-"));
    mediaA = join(root, "media-a");
    mediaB = join(root, "media-b");
    downloads = join(root, "downloads");
    for (const d of [join(mediaA, "images"), mediaB, downloads]) mkdirSync(d, { recursive: true });
    writeFileSync(join(mediaA, "images", "dawn.png"), PNG);
    a = await spawnTestServer({ port: A, tempPrefix: "os-next-bundles-a-", extraArgs: ["--media-dir", mediaA] });
    b = await spawnTestServer({ port: B, tempPrefix: "os-next-bundles-b-", extraArgs: ["--media-dir", mediaB] });
    browser = await chromium.launch({ headless: true });
    pageA = await openPage(A);
    pageB = await openPage(B);

    // Library A: one copyrighted song and one public-domain song (both on the same sunrise picture), a presentation, a Bible.
    expect(await asConsole(pageA, "POST", "/api/songs", { id: "s_cr", title: "New Anthem", author: "J. Writer", copyright: "© 2020 Worship Co", ccli_number: "123456", slides: [slide("Rise up, rise up", "url('/media/images/dawn.png')")] })).toBe(200);
    expect(await asConsole(pageA, "POST", "/api/songs", { id: "s_pd", title: "Old Hymn", author: "Anon", copyright: "Public Domain", ccli_number: null, slides: [slide("Come thou fount", "/media/images/dawn.png")] })).toBe(200);
    expect(await asConsole(pageA, "POST", "/api/presentations", { id: "p1", title: "Welcome Deck", author: "", slides: [slide("Welcome!")] })).toBe(200);
    for (const [ch, text] of [[1, "In the beginning"], [3, "For God so loved"]] as const) {
      expect(await asConsole(pageA, "POST", "/api/scriptures", { id: `tst${ch}`, book: ch === 1 ? "Genesis" : "John", chapter: ch, verse_start: 1, verse_end: 1, version: "TST", reference: `${ch === 1 ? "Genesis" : "John"} ${ch}`, verses: [{ verse_number: 1, text }] })).toBe(200);
    }
    await pageA.reload();
    await pageA.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 90000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(a);
    await teardownTestServer(b);
  });

  test("a copyrighted song saves from the library menu, and opening it shows the advisory; Cancel imports nothing", async () => {
    const { path, name } = await saveFromLibrary(pageA, "songs", "s_cr");
    expect(name).toBe("New Anthem.ossong");
    expect(readFileSync(path).subarray(0, 2).toString()).toBe("PK"); // a zip, openable with any unzip tool

    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await pageB.waitForSelector("#bundle-advisory-modal", { state: "visible", timeout: 10000 });
    const text = await pageB.locator("#bundle-advisory-modal").innerText();
    expect(text).toContain("under copyright");
    expect(text).toContain("New Anthem (J. Writer) — © 2020 Worship Co · CCLI #123456");
    expect(await pageB.locator("#btn-continue-bundle-advisory").innerText()).toBe("Continue");

    await clearToasts(pageB);
    await pageB.click("#btn-cancel-bundle-advisory");
    await toastWith(pageB, "Import cancelled");
    expect(await advisoryVisible(pageB)).toBe(false);
    expect(await songsIn(B)).toHaveLength(0);
    expect(filesIn(join(mediaB, "images"))).toHaveLength(0);
  }, 60000);

  test("Continue imports it with its picture, and opening the same file again changes nothing", async () => {
    const path = join(downloads, "New Anthem.ossong");
    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await pageB.waitForSelector("#bundle-advisory-modal", { state: "visible", timeout: 10000 });
    await clearToasts(pageB);
    await pageB.click("#btn-continue-bundle-advisory");
    await toastWith(pageB, "Imported 1 song");

    const songs = await songsIn(B);
    expect(songs.map((s) => s.title)).toEqual(["New Anthem"]);
    const restored = filesIn(join(mediaB, "images"));
    expect(restored).toHaveLength(1);
    expect(restored[0]).toMatch(/^dawn-[0-9a-f]{8}\.png$/);
    expect(songs[0].slides[0].background).toBe(`url('/media/images/${restored[0]}')`);
    const served = await fetch(`http://127.0.0.1:${B}/media/images/${restored[0]}`);
    expect(served.status).toBe(200);
    expect(Buffer.from(await served.arrayBuffer()).equals(PNG)).toBe(true);
    // The library list shows it.
    await closeDialogs(pageB);
    await pageB.click('.tab-btn[data-tab="songs"]');
    await pageB.locator('#catalog-table-body tr[data-id="s_cr"]').waitFor({ state: "visible", timeout: 10000 });

    // The same file again: every song in it is already here, so there is nothing to warn about or add.
    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await toastWith(pageB, "Nothing new");
    expect(await advisoryVisible(pageB)).toBe(false);
    expect(await songsIn(B)).toHaveLength(1);
    expect(filesIn(join(mediaB, "images"))).toHaveLength(1);
  }, 60000);

  test("a public-domain song opens with no pop-up at all", async () => {
    const { path } = await saveFromLibrary(pageA, "songs", "s_pd");
    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await toastWith(pageB, "Imported 1 song");
    expect(await advisoryVisible(pageB)).toBe(false);
    expect((await songsIn(B)).map((s) => s.title).sort()).toEqual(["New Anthem", "Old Hymn"]);
    // Both songs use the same picture, so the second import reused the file the first one wrote.
    expect(filesIn(join(mediaB, "images"))).toHaveLength(1);
  }, 60000);

  test("a presentation saves and opens", async () => {
    const { path, name } = await saveFromLibrary(pageA, "presentations", "p1");
    expect(name).toBe("Welcome Deck.ospres");
    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await toastWith(pageB, "Imported 1 presentation");
    const pres = (await (await fetch(`http://127.0.0.1:${B}/api/presentations`)).json()) as any[];
    expect(pres.map((p) => p.title)).toEqual(["Welcome Deck"]);
  }, 60000);

  test("a whole library of songs saves from the Import dialog in one file", async () => {
    await openImportDialog(pageA);
    const [download] = await Promise.all([pageA.waitForEvent("download", { timeout: 15000 }), pageA.click("#btn-export-all-songs")]);
    expect(download.suggestedFilename()).toBe("2 songs.ossong");
  }, 30000);

  test("a schedule saves as .osservice from the Save dialog and opens in the other library, picture and all", async () => {
    for (const [type, id] of [["song", "s_pd"], ["presentation", "p1"]]) await asConsole(pageA, "POST", "/api/command", { AddToSchedule: { item_type: type, item_id: id } });
    await pollUntil(async () => (await (await fetch(`http://127.0.0.1:${A}/api/state`)).json()).schedule.items.length, (n) => n === 2, 8000, "two schedule items");

    await closeDialogs(pageA);
    await pageA.evaluate(() => (document.getElementById("schedule-title-display") as HTMLElement).click());
    await pageA.waitForSelector("#save-modal", { state: "visible", timeout: 8000 });
    await pageA.fill("#save-schedule-title", "Easter Sunrise");
    await pageA.selectOption("#save-format-select", "osservice");
    expect(await pageA.locator("#btn-confirm-save").innerText()).toContain(".osservice");
    const [download] = await Promise.all([pageA.waitForEvent("download", { timeout: 15000 }), pageA.click("#btn-confirm-save")]);
    expect(download.suggestedFilename()).toBe("Easter Sunrise.osservice");
    const path = join(downloads, "Easter Sunrise.osservice");
    await download.saveAs(path);

    // Open it from the Open Schedule dialog of library B.
    await openImportDialog(pageB);
    await pageB.click("#btn-open-schedule-hub");
    await pageB.waitForSelector("#open-modal", { state: "visible", timeout: 8000 });
    await pageB.setInputFiles("#file-schedule-input", path);
    await toastWith(pageB, 'Opened service "Easter Sunrise"');
    const state = await pollUntil(async () => (await (await fetch(`http://127.0.0.1:${B}/api/state`)).json()), (s) => s.schedule.items.length === 2, 8000, "the service in library B");
    expect(state.schedule.title).toBe("Easter Sunrise");
    expect(state.schedule.items.map((i: any) => i.title)).toEqual(["Old Hymn", "Welcome Deck"]);
    // The console shows it too.
    await pageB.waitForFunction(() => /Easter Sunrise/.test(document.getElementById("schedule-title-display")?.textContent ?? ""), null, { timeout: 8000 });
  }, 60000);

  test("a Bible saves from the Import dialog and installs in the other library; installing it again asks before replacing", async () => {
    await openImportDialog(pageA);
    await pageA.waitForFunction(() => [...(document.getElementById("select-export-bible") as HTMLSelectElement).options].some((o) => o.value === "TST"), null, { timeout: 8000 });
    await pageA.selectOption("#select-export-bible", "TST");
    const [download] = await Promise.all([pageA.waitForEvent("download", { timeout: 15000 }), pageA.click("#btn-export-bible")]);
    expect(download.suggestedFilename()).toBe("TST.osbible");
    const path = join(downloads, "TST.osbible");
    await download.saveAs(path);

    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await toastWith(pageB, "Installed TST (2 chapters)");
    const bibles = (await (await fetch(`http://127.0.0.1:${B}/api/bibles`)).json()) as any[];
    expect(bibles.map((x) => x.id)).toContain("TST");
    const passage = await (await fetch(`http://127.0.0.1:${B}/api/bibles/passage?book=John&chapter=3&version=TST&verse_start=1&verse_end=1`)).json();
    expect(JSON.stringify(passage)).toContain("For God so loved");

    // Again: it's already installed, so it asks. Cancel leaves it; Replace overwrites it.
    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await pageB.waitForSelector("#confirm-modal", { state: "visible", timeout: 8000 });
    expect(await pageB.locator("#confirm-modal-message").innerText()).toContain("TST is already installed");
    await clearToasts(pageB);
    await pageB.click("#btn-cancel-confirm");
    await toastWith(pageB, "Import cancelled");

    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", path);
    await pageB.waitForSelector("#confirm-modal", { state: "visible", timeout: 8000 });
    expect(await pageB.locator("#btn-accept-confirm").innerText()).toContain("Replace");
    await clearToasts(pageB);
    await pageB.click("#btn-accept-confirm");
    await toastWith(pageB, "Installed TST");
  }, 60000);

  test("a file that isn't one of ours is refused with a reason and changes nothing", async () => {
    const before = (await songsIn(B)).length;
    const fake = join(downloads, "not really.ossong");
    writeFileSync(fake, "this is not a zip file at all");
    await openImportDialog(pageB);
    await pageB.setInputFiles("#file-bundle-import", fake);
    await toastWith(pageB, "isn't an OpenSanctuary file");
    expect(await advisoryVisible(pageB)).toBe(false);
    expect(await songsIn(B)).toHaveLength(before);
  }, 30000);
});
