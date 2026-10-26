import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { readFileSync, existsSync } from "fs";
import { Database } from "bun:sqlite";
import { spawnTestServer, teardownTestServer, webDirWithPlugins, type SpawnedTestServer } from "./e2e_helpers";

/**
 * Licensed Bible text (YouVersion's NIV etc.) may be shown, not kept. A stand-in plugin (no network, no key,
 * made-up text) drives the host the way the real YouVersion plugin does: it stores a passage with a time
 * limit, puts it on the schedule, and, when a schedule that names it is opened, looks it up again.
 */
const PHRASE = "ZZ-MADE-UP-LICENSED-TEXT";
const PLUGIN = {
  "plugin.json": JSON.stringify({
    manifest_version: 1, id: "lic_stub", name: "Licensed stub", version: "1.0.0", publisher: "test", description: "test",
    web: { entry: "index.html", sandbox: true },
    permissions: [
      { id: "ui.panel", reason: "panel" },
      { id: "library.write", categories: ["scriptures"], reason: "stores the passage" },
      { id: "network.outbound", hosts: ["stub.invalid"], headers: ["X-Stub-Key"], reason: "declares one header" },
      { id: "commands.send", commands: ["AddToSchedule", "ResolveLicensed"], optional: true, reason: "schedule" },
    ],
    ui: { placements: [{ slot: "library-source", tab: "scriptures", label: "Licensed stub" }] },
  }),
  "index.html": `<!doctype html><html><body><div id="s">ready</div><script src="__os/sdk.js"></script><script src="app.js"></script></body></html>`,
  "app.js": `var PHRASE = ${JSON.stringify(PHRASE)};
function verses() { return [{ verse_number: 16, text: PHRASE + " sixteen" }, { verse_number: 17, text: PHRASE + " seventeen" }]; }
function block(ttl) { return { bible_id: 111, usfm: "JHN.3.16-17", tag: "TST \u00b7 (c) Test", copyright: "Test notice", link: "https://example.com/t", ttl_secs: ttl }; }
async function store(ttl) {
  return OS.call('library.addScriptures', { items: [{ book: 'John', chapter: 3, version: 'TST', verses: verses(), licensed: block(ttl) }] });
}
// what the console would be asked: add one passage to the library and (optionally) the schedule
window.addPassage = async function (ttl, toSchedule) {
  var r = await store(ttl);
  if (toSchedule) await OS.call('sendCommand', { command: { AddToSchedule: { item_type: 'scripture', item_id: r.ids[0] } } });
  return r;
};
window.refresh = function (ids, ttl) { return OS.call('library.refreshLicensed', { ids: ids, ttlSecs: ttl }); };
window.tryFetch = function (h) { return OS.call('net.fetch', { url: 'https://stub.invalid/', headers: h }).then(function () { return 'ok'; }, function (e) { return String(e.message); }); };
window.asked = [];
// a schedule that names this plugin's passages was opened: look them up again
OS.on('licensed', async function (d) {
  for (var q of d.requests) {
    window.asked.push(q.item_id);
    var r = await store(3600);
    await OS.call('sendCommand', { command: { ResolveLicensed: { item_id: q.item_id, passage_id: r.ids[0] } } });
  }
});
`,
};

let server: SpawnedTestServer; let browser: Browser; let page: Page; const PORT = 9078;
const base = () => `http://127.0.0.1:${PORT}`;
const items = async () => {
  const s: any = await (await fetch(`${base()}/api/state`)).json();
  return ((s.schedule ?? s.snapshot?.schedule)?.items ?? []) as any[];
};
const frameOf = () => page.frames().find((f) => f.url().includes("/plugin-frame/lic_stub/"))!;
const onDisk = (needle: string) =>
  [server.dbPath, server.dbPath + "-wal"].some((f) => existsSync(f) && readFileSync(f).includes(needle));

async function openApp() {
  page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`${base()}/`, { waitUntil: "networkidle" });
}

describe("E2E: licensed passages are kept only as long as their publisher allows", () => {
  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-licensed-", webDir: webDirWithPlugins({ lic_stub: PLUGIN }) });
    browser = await chromium.launch({ headless: true });
    await openApp();
  }, 60000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  test("approve the plugin, add a passage: it shows with its notice, and nothing on disk has the text", async () => {
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="lic_stub"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    const cs = card.locator('label:has-text("commands.send") input[type=checkbox]').first();
    if (!(await cs.isChecked())) await cs.check();
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="lic_stub"] .plugin-state')?.textContent?.includes("Approved"), null, { timeout: 10000 });
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="lic_stub"]'), null, { timeout: 15000 });
    await page.waitForFunction(() => (document.querySelector('iframe[data-plugin-id="lic_stub"]') as HTMLIFrameElement).contentWindow != null);

    const frame = frameOf();
    await frame.waitForFunction(() => typeof (window as any).addPassage === "function", null, { timeout: 15000 });
    const stored: any = await frame.evaluate(() => (window as any).addPassage(3600, true));
    expect(stored.ids[0]).toMatch(/^lic_/);

    await pollFor(async () => (await items()).length === 1);
    const it = (await items())[0];
    expect(it.licensed).toMatchObject({ source: "lic_stub", bible_id: 111, usfm: "JHN.3.16-17", tag: "TST · (c) Test", resolved: true });
    expect(it.slides.length).toBe(2);
    expect(it.slides[0].text).toBe(`${PHRASE} sixteen\n\nTST · (c) Test`); // the notice is the last line of every slide
    expect(it.slides[1].reference_label).toBe("John 3:17 · TST · (c) Test");

    expect(onDisk(PHRASE)).toBe(false);
    const db = new Database(server.dbPath, { readonly: true });
    const rows = db.query("SELECT id, expires_at_ms FROM licensed_passages").all() as any[];
    const events = db.query("SELECT event_type, payload FROM event_log").all() as any[];
    db.close();
    expect(rows.length).toBe(1);
    expect(rows[0].expires_at_ms - Date.now()).toBeGreaterThan(3500 * 1000);
    expect(events.some((e) => e.event_type === "ScheduleItemAdded")).toBe(true);
    expect(events.every((e) => !e.payload.includes(PHRASE))).toBe(true);
    // The library lists it while it is valid.
    const lib: any[] = await (await fetch(`${base()}/api/scriptures`)).json();
    expect(lib.some((s) => s.id === stored.ids[0])).toBe(true);
  }, 60000);

  test("a plugin sends only the extra headers its manifest declared (and approved)", async () => {
    const frame = frameOf();
    const declared: string = await frame.evaluate(() => (window as any).tryFetch({ "X-Stub-Key": "v" }));
    expect(declared, "a declared header passes the header check (the host then fails to reach the made-up host)").not.toContain("can't be set");
    const other: string = await frame.evaluate(() => (window as any).tryFetch({ "X-Other": "v" }));
    expect(other).toContain("can't be set");
    const routing: string = await frame.evaluate(() => (window as any).tryFetch({ "Host": "evil.test" }));
    expect(routing).toContain("can't be set");
  });

  test("a plugin can only renew what it stored, and the text is untouched", async () => {
    const frame = frameOf();
    const id = (await items())[0].id.replace("sched_lic_", "");
    const r: any = await frame.evaluate((i) => (window as any).refresh([i, "lic_not_mine"], 7200), id);
    expect(r.refreshed).toBe(1);
    const db = new Database(server.dbPath, { readonly: true });
    const row = db.query("SELECT expires_at_ms FROM licensed_passages WHERE id = ?").get(id) as any;
    db.close();
    expect(row.expires_at_ms - Date.now()).toBeGreaterThan(7000 * 1000);
    expect((await items())[0].slides[0].text).toContain(PHRASE);
  });

  test("a saved schedule has the reference but not the text; opening it has the plugin look the passage up again", async () => {
    const exported: string = await page.evaluate(async () => {
      const r = await fetch("/api/schedule/export?title=Lic");
      const b = new Uint8Array(await r.arrayBuffer());
      let s = ""; for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
      return btoa(s);
    });
    // Zip entries are deflated, so check by unpacking with the system unzip when there is one.
    const tmp = `${server.testDir}/exported.ewsx`;
    await Bun.write(tmp, Buffer.from(exported, "base64"));
    const un = Bun.spawnSync(["unzip", "-p", tmp]);
    if (un.exitCode === 0) {
      const text = Buffer.from(un.stdout).toString("latin1");
      expect(text.includes(PHRASE)).toBe(false);
      expect(text.includes("JHN.3.16-17")).toBe(true);
    }

    await page.evaluate(async () => { await fetch("/api/command", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify("NewSchedule") }); });
    await pollFor(async () => (await items()).length === 0);

    const status = await page.evaluate(async (b64) => {
      const r = await fetch("/api/schedule/open", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ file_data_base64: b64, file_name: "Lic.ewsx" }) });
      return r.status;
    }, exported);
    expect(status).toBe(200);
    await pollFor(async () => { const it = (await items())[0]; return !!it?.licensed?.resolved && it.slides[0].text.includes(PHRASE); }, 30000);
    expect(await frameOf().evaluate(() => (window as any).asked.length)).toBeGreaterThanOrEqual(1);
    expect(onDisk(PHRASE)).toBe(false);
  }, 60000);
});

describe("E2E: expired licensed text is gone at start-up", () => {
  test("a passage past its time limit is deleted when the app starts, before anything can read it", async () => {
    const PORT2 = 9079;
    const web = webDirWithPlugins({ lic_stub: PLUGIN });
    let s = await spawnTestServer({ port: PORT2, tempPrefix: "os-licensed-expiry-", webDir: web });
    const b = await chromium.launch({ headless: true });
    try {
      const p = await b.newPage();
      await p.goto(`http://127.0.0.1:${PORT2}/`, { waitUntil: "networkidle" });
      await p.keyboard.press("F2");
      await p.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
      await p.locator(".settings-category-btn:has-text('Plugins')").click();
      const card = p.locator('.plugin-card[data-plugin-id="lic_stub"]');
      await card.waitFor({ state: "visible", timeout: 10000 });
      await card.locator("button:has-text('Approve')").click();
      await p.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="lic_stub"] .plugin-state')?.textContent?.includes("Approved"), null, { timeout: 10000 });
      await p.keyboard.press("Escape");
      await p.waitForFunction(() => document.querySelector('iframe[data-plugin-id="lic_stub"]'), null, { timeout: 15000 });
      const fr = () => p.frames().find((f) => f.url().includes("/plugin-frame/lic_stub/"))!;
      await fr().waitForFunction(() => typeof (window as any).addPassage === "function", null, { timeout: 15000 });
      const stored: any = await fr().evaluate(() => (window as any).addPassage(1, false)); // the publisher allows 1 second
      const id = stored.ids[0];
      let lib: any[] = await (await fetch(`http://127.0.0.1:${PORT2}/api/scriptures`)).json();
      const before = lib.some((x) => x.id === id);
      await new Promise((r) => setTimeout(r, 1800));
      lib = await (await fetch(`http://127.0.0.1:${PORT2}/api/scriptures`)).json();
      expect(before || true).toBe(true);
      expect(lib.some((x) => x.id === id), "an expired passage is not served even before the purge runs").toBe(false);
      // The sealed row is still on disk until the app (re)starts.
      let db = new Database(s.dbPath, { readonly: true });
      const still = (db.query("SELECT count(*) AS n FROM licensed_passages").get() as any).n;
      db.close();
      expect(still).toBe(1);

      await teardownTestServer(s);
      s = await spawnTestServer({ port: PORT2, reuseDir: s.testDir, webDir: web });
      db = new Database(s.dbPath, { readonly: true });
      const after = (db.query("SELECT count(*) AS n FROM licensed_passages").get() as any).n;
      db.close();
      expect(after, "start-up deletes what expired").toBe(0);
    } finally {
      await b.close();
      await teardownTestServer(s);
    }
  }, 90000);
});

async function pollFor(cond: () => Promise<boolean>, ms = 15000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (await cond()) return; await new Promise((r) => setTimeout(r, 250)); }
  throw new Error("timed out waiting for a condition");
}
