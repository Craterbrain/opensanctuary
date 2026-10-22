// Red-team test: a hostile sandboxed plugin (fixtures/hostile_plugin.ts) that the operator approved
// in full, plus a batch of malicious manifests. Whatever it does, the console and server must stay
// up and the plugin must stay inside its grants. Findings are printed as a table at the end.
import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type BrowserContext, type Page, type Frame } from "playwright";
import { mkdirSync, writeFileSync, symlinkSync } from "node:fs";
import { join } from "node:path";
import { spawnTestServer, teardownTestServer, webDirWithPlugins, type SpawnedTestServer } from "./e2e_helpers";
import { HOSTILE_ID, hostileManifest, hostileHtml, hostileJs } from "./fixtures/hostile_plugin";

const PORT = 9079;
const BASE = `http://127.0.0.1:${PORT}`;
const findings: Array<[string, string]> = [];
const note = (k: string, v: string) => findings.push([k, v.replace(/\s+/g, " ").slice(0, 400)]);

describe("E2E Red Team: hostile plugin cannot break out or take the console down", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  let ctx: BrowserContext;
  let isolated: Browser; // Chromium with site isolation, like desktop Chrome: a hung frame can't take the console with it
  let webDir: string;
  const consoleErrors: string[] = [];
  const frameOf = (): Frame | undefined => page.frames().find((f) => f.url().includes(`/plugin-frame/${HOSTILE_ID}/`));

  /** Is the console still usable, and the server still answering? */
  async function healthy(label: string): Promise<boolean> {
    let serverOk = false, pageOk = false;
    try { serverOk = (await fetch(`${BASE}/api/state`, { signal: AbortSignal.timeout(4000) })).ok; } catch {}
    try {
      pageOk = (await Promise.race([
        page.evaluate(() => document.querySelectorAll("*").length > 0 && typeof (window as any).__pwn === "undefined"),
        new Promise<boolean>((r) => setTimeout(() => r(false), 4000)),
      ]));
    } catch {}
    if (!serverOk || !pageOk) note(`HEALTH ${label}`, `server=${serverOk} page=${pageOk}`);
    return serverOk && pageOk;
  }

  /**
   * After a flood, the console may be busy for a while; what matters is that it comes back, not that
   * it answers within a fixed moment (a slow shared CI runner can take several seconds to drain a
   * burst a desktop clears instantly). Polls healthy() until it passes or `withinMs` runs out.
   */
  async function recovers(label: string, withinMs = 15000): Promise<boolean> {
    const until = Date.now() + withinMs;
    for (;;) {
      if (await healthy(label)) return true;
      if (Date.now() >= until) return false;
      await new Promise((r) => setTimeout(r, 500));
    }
  }

  async function run(name: string, timeout = 8000): Promise<string> {
    const f = frameOf();
    if (!f) return "NO FRAME";
    try {
      return await Promise.race([
        f.evaluate((n) => (window as any).__hostile.run(n), name),
        new Promise<string>((r) => setTimeout(() => r("TIMEOUT (frame busy)"), timeout)),
      ]);
    } catch (e) { return "EVAL FAILED: " + (e as Error).message.slice(0, 120); }
  }

  /** GET an API path as the console page itself (it holds the login). */
  const apiGet = (path: string) => page.evaluate((p) => fetch(p).then((r) => r.status === 200 ? r.json() : { __status: r.status }), path) as Promise<any>;

  async function freshPage() {
    try { await Promise.race([page.close({ runBeforeUnload: false }), new Promise((r) => setTimeout(r, 3000))]); } catch {}
    page = await ctx.newPage();
    page.on("pageerror", (e) => consoleErrors.push(String(e.message).slice(0, 200)));
  }

  async function openFresh() {
    await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
    // A previous test left the console hung: plugins are paused behind the safe-mode banner. Resume them.
    if (await page.waitForSelector("#plugin-safe-mode", { timeout: 3000 }).then(() => true, () => false)) await page.locator("#plugin-safe-mode button:has-text('Resume')").click();
    await page.waitForFunction(() => !!(window as any).__APP_READY__, null, { timeout: 15000 });
    await page.waitForFunction((id) => document.querySelector(`iframe[data-plugin-id="${id}"]`), HOSTILE_ID, { timeout: 15000 });
    await frameOf()!.waitForFunction(() => !!(window as any).__hostile, null, { timeout: 15000 });
  }

  beforeAll(async () => {
    webDir = webDirWithPlugins({
      [HOSTILE_ID]: { "plugin.json": JSON.stringify(hostileManifest), "index.html": hostileHtml, "app.js": hostileJs },
    });
    // Malicious manifests / layouts: none may be loadable, none may break discovery of the rest.
    const bad = (dir: string, files: Record<string, string>) => {
      mkdirSync(join(webDir, "plugins", dir), { recursive: true });
      for (const [f, t] of Object.entries(files)) writeFileSync(join(webDir, "plugins", dir, f), t);
    };
    const base = { manifest_version: 1, version: "1.0.0", publisher: "x", description: "x", web: { entry: "index.html", sandbox: true }, permissions: [{ id: "ui.panel", reason: "x" }] };
    bad("bad_traverse_id", { "plugin.json": JSON.stringify({ ...base, id: "../../evil", name: "t" }), "index.html": "<p>x" });
    bad("bad_reserved_id", { "plugin.json": JSON.stringify({ ...base, id: "__os", name: "t" }), "index.html": "<p>x" });
    bad("bad_dupe_id", { "plugin.json": JSON.stringify({ ...base, id: "genius_lyrics", name: "IMPOSTOR" }), "index.html": "<p>x" });
    bad("bad_html_name", { "plugin.json": JSON.stringify({ ...base, id: "bad_html_name", name: "<img src=x onerror=window.__pwn=1>", description: "<script>window.__pwn=1</script>", publisher: "<b onmouseover=window.__pwn=1>p</b>" }), "index.html": "<p>x" });
    bad("bad_entry_escape", { "plugin.json": JSON.stringify({ ...base, id: "bad_entry_escape", name: "t", web: { entry: "../../index.html", sandbox: true } }) });
    bad("bad_entry_abs", { "plugin.json": JSON.stringify({ ...base, id: "bad_entry_abs", name: "t", web: { entry: "/etc/passwd", sandbox: true } }) });
    bad("bad_json", { "plugin.json": "{ not json \u0000" });
    bad("bad_huge", { "plugin.json": JSON.stringify({ ...base, id: "bad_huge", name: "t", description: "x".repeat(8_000_000) }), "index.html": "<p>x" });
    bad("bad_many_perms", { "plugin.json": JSON.stringify({ ...base, id: "bad_many_perms", name: "t", permissions: Array.from({ length: 5000 }, () => ({ id: "ui.panel", reason: "x" })) }), "index.html": "<p>x" });
    bad("bad_backend", { "plugin.json": JSON.stringify({ ...base, id: "bad_backend", name: "t", backend: { library: "x.so" } }), "index.html": "<p>x" });
    bad("bad_toolbar_html", { "plugin.json": JSON.stringify({ ...base, id: "bad_toolbar_html", name: "t", permissions: [{ id: "ui.toolbar", reason: "x" }], ui: { toolbar: [{ id: "a", label: "<img onerror=x>", icon: "../../x", tooltip: "x" }, { id: "b", label: "Save", icon: "save", tooltip: "x" }] } }), "index.html": "<p>x" });
    bad("bad_unicode_id", { "plugin.json": JSON.stringify({ ...base, id: "pl‮ugin", name: "t‮evil" }), "index.html": "<p>x" });
    bad("bad_csp_host", { "plugin.json": JSON.stringify({ ...base, id: "bad_csp_host", name: "t", permissions: [{ id: "network.outbound", hosts: ["a.com; script-src *", "*", "'unsafe-inline'", "a.com\r\nX-Evil: 1"], reason: "x" }] }), "index.html": "<p>x" });
    bad("bad_symlink", { "plugin.json": JSON.stringify({ ...base, id: "bad_symlink", name: "t" }) });
    try { symlinkSync("/etc/passwd", join(webDir, "plugins", "bad_symlink", "index.html")); } catch {}
    try { symlinkSync("/etc", join(webDir, "plugins", "bad_symlink_dir")); } catch {}

    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-hostile-e2e-", webDir });
    browser = await chromium.launch({ headless: true });
    isolated = await chromium.launch({ headless: true, args: ["--site-per-process", "--enable-features=IsolateSandboxedIframes"] });
    ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    page = await ctx.newPage();
    page.on("pageerror", (e) => consoleErrors.push(String(e.message).slice(0, 200)));
  }, 60000);

  afterAll(async () => {
    console.log("\n=== RED TEAM FINDINGS ===\n" + findings.map(([k, v]) => `${k.padEnd(28)} ${v}`).join("\n") + "\n=== page errors: " + consoleErrors.length + " ===\n" + consoleErrors.slice(0, 10).join("\n"));
    if (isolated) await isolated.close();
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("malicious manifests are rejected or inert, and the rest of discovery survives", async () => {
    await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
    const body: any = await apiGet("/api/plugins");
    expect(body.__status).toBeUndefined();
    const list: any[] = Array.isArray(body) ? body : body.plugins ?? [];
    const ids = list.map((p) => p.id);
    note("discovered ids", ids.join(","));
    for (const p of list) if (p.error || p.problem || p.invalid) note(`rejected ${p.id}`, JSON.stringify(p.error ?? p.problem ?? p.invalid));
    expect(ids).toContain("sandbox_demo");
    expect(ids).toContain(HOSTILE_ID);
    // An impostor with a bundled plugin's id: both copies are flagged (ambiguous), neither may run.
    note("impostor vs bundled", JSON.stringify(list.filter((p) => p.id === "genius_lyrics").map((p) => [p.name, p.error])));
    expect(list.filter((p) => p.id === "genius_lyrics").every((p) => p.error)).toBe(true);
    for (const forbidden of ["../../evil", "__os", "bad_backend", "bad_entry_escape", "bad_entry_abs", "bad_json", "bad_many_perms", "bad_symlink"]) {
      const rec = list.find((p) => p.id === forbidden); // rejected ones are listed, flagged with their error
      if (rec) expect(rec.error ?? rec.problem ?? rec.invalid).toBeTruthy();
    }
    for (const id of ["bad_huge", "bad_csp_host", "bad_toolbar_html", "bad_html_name"]) {
      const p = list.find((x) => x.id === id);
      note(`record ${id}`, p ? JSON.stringify({ ...p, description: String(p.description ?? "").slice(0, 30) + `…(${String(p.description ?? "").length} chars)` }).slice(0, 700) : "absent");
    }
    expect(await healthy("after discovery")).toBe(true);
    // None of their entries are servable, approved or not.
    for (const p of ["bad_entry_escape", "bad_html_name", "bad_symlink", "bad_traverse_id"]) {
      const r = await fetch(`${BASE}/plugin-frame/${p}/index.html`);
      note(`GET frame ${p}`, String(r.status));
      expect(r.status).not.toBe(200);
    }
    for (const u of ["/plugin-frame/%2e%2e/%2e%2e/etc/passwd", "/plugin-frame/hostile_test/..%2f..%2fplugin.json", "/plugin-frame/hostile_test/%2e%2e/%2e%2e/index.html", "/plugins/bad_symlink/index.html", "/plugins/bad_symlink_dir/passwd"]) {
      const r = await fetch(`${BASE}${u}`); const t = await r.text();
      note(`GET ${u.slice(0, 40)}`, `${r.status} ${t.includes("root:") ? "LEAKED /etc/passwd" : ""}`);
      expect(t.includes("root:")).toBe(false);
    }
  });

  test("the operator approves everything; the console renders hostile text safely", async () => {
    await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator(`.plugin-card[data-plugin-id="${HOSTILE_ID}"]`);
    await card.waitFor({ state: "visible", timeout: 10000 });
    await page.locator('.plugin-card[data-plugin-id="bad_html_name"]').waitFor({ state: "visible", timeout: 5000 }).catch(() => note("bad_html_name card", "not shown"));
    await page.locator('.plugin-card[data-plugin-id="bad_toolbar_html"]').waitFor({ state: "visible", timeout: 5000 }).catch(() => {});
    expect(await page.evaluate(() => (window as any).__pwn)).toBeUndefined();
    expect(await page.locator("#options-modal img[src='x']").count()).toBe(0);
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction((id) => document.querySelector(`.plugin-card[data-plugin-id="${id}"] .plugin-state`)?.textContent?.includes("Approved"), HOSTILE_ID, { timeout: 10000 });
    // Approving the one with HTML in its name must also stay inert.
    const html = page.locator('.plugin-card[data-plugin-id="bad_html_name"] button:has-text("Approve")');
    if (await html.count()) { await html.click(); await page.waitForTimeout(500); }
    expect(await page.evaluate(() => (window as any).__pwn)).toBeUndefined();
    await openFresh();
    expect(await healthy("after approve")).toBe(true);
  }, 60000);

  test("a manifest with CSP-injecting hosts cannot be approved into a header", async () => {
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const card = page.locator('.plugin-card[data-plugin-id="bad_csp_host"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    note("csp card text", (await card.innerText()).slice(0, 300));
    const btn = card.locator("button:has-text('Approve')").first();
    note("csp approve button", String(await btn.count()));
    if (await btn.count()) { await btn.click(); await page.waitForTimeout(1200); }
    note("csp card state", (await card.locator(".plugin-state").first().innerText()));
    const r = await fetch(`${BASE}/plugin-frame/bad_csp_host/index.html`);
    note("csp frame status", String(r.status));
    note("csp header", r.headers.get("content-security-policy") ?? "(none)");
    expect(r.headers.get("x-evil")).toBeNull();
    const csp = r.headers.get("content-security-policy") ?? "";
    expect(csp).not.toMatch(/script-src[^;]*\*/);
    await page.keyboard.press("Escape");
  }, 60000);

  test("sandbox escapes and ungranted requests are refused", async () => {
    const mustBlock = ["parentDom", "directFetch", "directFetchAbs", "xhr", "websocket", "beaconImg", "dynamicScript", "formToTop", "windowOpen"];
    for (const a of mustBlock) {
      const r = await run(a); note(a, r);
      expect(r).not.toMatch(/READ|ALLOWED|OPEN$|LOADED|OPENED/);
    }
    for (const a of ["topNav", "modals", "storage", "inlineEval", "nestedFrame", "serviceWorker", "blobWorker"]) note(a, await run(a));
    expect(await run("storage")).toBe("wrote []");
    await page.waitForTimeout(500);
    expect(page.url()).toBe(`${BASE}/`); // not navigated away by top-nav or form-to-top
    expect(await healthy("after escapes")).toBe(true);

    const cmd = await run("cmdUngranted"); note("cmdUngranted", cmd);
    expect(cmd.split(' | ').filter((x) => x.startsWith('OK') && !x.includes('"success":false')).length).toBe(0);
    note("cmdGranted", await run("cmdGranted"));
    const batch = await run("cmdBatch"); note("cmdBatch", batch);
    expect(batch).not.toMatch(/^OK/);
    const ung = await run("ungrantedMethods"); note("ungrantedMethods", ung);
    expect(ung.split(" | ").filter((x) => x.startsWith("OK")).length).toBe(0); // the clipboard needs ui.clipboard, which it never got
    const kr = await run("keyringOthers"); note("keyringOthers", kr);
    expect(kr).not.toMatch(/OK/);
    note("keyringOwn", await run("keyringOwn"));
    note("keyringAccounts", await run("keyringAccounts"));
    note("badParams", await run("badParams"));
    expect(await healthy("after bridge abuse")).toBe(true);
  }, 90000);

  test("net.fetch: what a plugin granted 127.0.0.1 can reach", async () => {
    const r = await run("fetchUrls", 30000);
    note("fetchUrls", r);
    console.log("--- fetchUrls ---\n" + r);
    // Only the granted host may ever be contacted: nothing but the 127.0.0.1 rows may succeed.
    for (const line of r.split("\n")) if (/evil\.example|file:|ftp:|javascript:|data:|\[::1\]|localhost|2130706433|0x7f/.test(line.split(" => ")[0])) expect(line).not.toMatch(/=> OK/);
    const h = await run("fetchHeaders"); note("fetchHeaders", h);
    expect(await healthy("after fetch abuse")).toBe(true);
  }, 60000);

  test("abusing granted surfaces: floods, XSS, resize, forged messages", async () => {
    note("notifyFlood", await run("notifyFlood", 20000));
    note("notifyHtml", await run("notifyHtml", 20000));
    expect(await page.evaluate(() => (window as any).__pwn)).toBeUndefined();
    // The bell shows them as text.
    expect(await page.locator("img[src='x']").count()).toBe(0);
    note("toolbarAbuse", await run("toolbarAbuse"));
    note("resizeAbuse", await run("resizeAbuse"));
    const h0 = await page.evaluate((id) => (document.querySelector(`iframe[data-plugin-id="${id}"]`) as HTMLElement).getBoundingClientRect().height, HOSTILE_ID);
    note("frame height after resize", String(h0));
    expect(h0).toBeLessThan(5000);
    note("forgeEvents", await run("forgeEvents"));
    note("impersonate", await run("impersonate"));
    const feed: any = await apiGet("/api/notifications");
    const items: any[] = Array.isArray(feed) ? feed : feed.items ?? feed.notifications ?? [];
    note("notification sources", [...new Set(items.map((n) => n.source))].join(","));
    expect(items.every((n) => n.source === `plugin:${HOSTILE_ID}`)).toBe(true); // the host stamps it; params.source/plugin are ignored
    note("notification count", String(items.length));
    expect(items.length).toBeLessThanOrEqual(200);
    expect(await recovers("after granted abuse")).toBe(true);
    note("logFlood", await run("logFlood", 30000));
    expect(await recovers("after log flood")).toBe(true);
    // The bridge caps a plugin's outstanding requests (MAX_IN_FLIGHT in plugin_bridge.ts): a burst
    // past it is refused straight away instead of each starting a fetch on the console's thread.
    const burst = await run("inFlightBurst", 30000);
    note("inFlightBurst", burst);
    const [okN, busyN] = (burst.match(/ok=(\d+) busy=(\d+)/) ?? []).slice(1).map(Number);
    expect(okN).toBeGreaterThan(0); // the plugin still gets answers...
    expect(busyN).toBeGreaterThan(0); // ...but not 200 concurrent fetches
    expect(okN + busyN).toBe(200);
    note("requestFlood", await run("requestFlood", 30000));
    expect(await recovers("after request flood")).toBe(true);
    note("hugeMessage", await run("hugeMessage", 30000));
    note("deepMessage", await run("deepMessage", 30000));
    await page.waitForTimeout(1500);
    expect(await healthy("after huge messages")).toBe(true);
  }, 180000);

  const recovery = () => test("after a hang, the next load pauses plugins and the operator can resume them", async () => {
    // The previous test left the marker behind (its tab never unloaded cleanly).
    await freshPage();
    await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
    await page.waitForSelector("#plugin-safe-mode", { timeout: 15000 });
    expect(await page.locator("iframe[data-plugin-id]").count()).toBe(0);
    expect(await page.locator("#plugin-safe-mode").innerText()).toContain("paused");
    // Nothing was revoked.
    expect((await apiGet("/api/plugins")).find((p: any) => p.id === HOSTILE_ID).state).toBe("active");
    await page.locator("#plugin-safe-mode button:has-text('Resume')").click();
    await page.waitForSelector(`iframe[data-plugin-id="${HOSTILE_ID}"]`, { timeout: 10000 });
    expect(await page.locator("#plugin-safe-mode").count()).toBe(0);
    // A normal reload afterwards starts plugins straight away (the marker was cleared on unload).
    await page.waitForTimeout(500);
    await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
    await page.waitForSelector(`iframe[data-plugin-id="${HOSTILE_ID}"]`, { timeout: 15000 });
    expect(await page.locator("#plugin-safe-mode").count()).toBe(0);
  }, 90000);


  // Denial of service: each one on a fresh page, then ask whether the CONSOLE (not the plugin) still works.
  for (const attack of ["domBomb", "memBomb", "spin", "spinTimers", "crashTab", "resizeLoop"]) {
    test(`DoS: ${attack}`, async () => {
      await freshPage();
      await openFresh();
      const out = await run(attack, 15000); note(`DoS ${attack}`, out);
      await page.waitForTimeout(2500);
      const t0 = Date.now();
      let responsive = false;
      try { responsive = await Promise.race([page.evaluate(() => { const b = document.createElement("i"); document.body.appendChild(b); b.remove(); return true; }), new Promise<boolean>((r) => setTimeout(() => r(false), 5000))]); } catch {}
      note(`DoS ${attack} console`, `responsive=${responsive} in ${Date.now() - t0}ms`);
      const serverOk = (await fetch(`${BASE}/api/state`, { signal: AbortSignal.timeout(4000) }).catch(() => ({ ok: false }))).ok;
      expect(serverOk).toBe(true); // the server never goes down because of a plugin page
      // The main console can still open the options dialog (the operator can always revoke).
      if (responsive) {
        await page.evaluate(() => { (document.activeElement as HTMLElement | null)?.blur?.(); window.focus(); });
        await page.keyboard.press("F2");
        const opened = await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 }).then(() => true, () => false);
        note(`DoS ${attack} F2`, String(opened));
        if (opened) await page.keyboard.press("Escape");
      }
      // Without site isolation a spinning frame shares the console's thread; recovery is tested below.
      if (!["spin", "spinTimers"].includes(attack)) expect(responsive).toBe(true);
    }, 90000);
    if (attack === "spin") recovery(); // right after the hang, before anything clears the marker
  }

  test("load shedding: busy timers in a shared-thread browser switch off the culprit, not the innocent plugin", async () => {
    await freshPage();
    await openFresh();
    // A well-behaved plugin runs alongside the hostile one.
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const demo = page.locator('.plugin-card[data-plugin-id="sandbox_demo"]');
    await demo.waitFor({ state: "visible", timeout: 10000 });
    await demo.locator("button:has-text('Approve')").first().click();
    await page.waitForSelector('iframe[data-plugin-id="sandbox_demo"]', { timeout: 10000 });
    await page.keyboard.press("Escape");
    expect(await page.locator(`iframe[data-plugin-id="${HOSTILE_ID}"]`).count()).toBeGreaterThan(0);

    const t0 = Date.now();
    const out = await run("spinTimers", 15000); note("shed: spinTimers", out);
    await page.waitForFunction((id) => !document.querySelector(`iframe[data-plugin-id="${id}"]`), HOSTILE_ID, { timeout: 90000, polling: 500 });
    note("shed: culprit removed after", `${Math.round((Date.now() - t0) / 1000)}s`);
    // The innocent one is back (or never left) and the console is quick again.
    await page.waitForSelector('iframe[data-plugin-id="sandbox_demo"]', { timeout: 30000 });
    await page.waitForTimeout(2000);
    const t1 = Date.now();
    await page.evaluate(() => 1);
    expect(Date.now() - t1).toBeLessThan(1000);
    expect(await page.locator(".toast, #os-toast-container").first().innerText().catch(() => "")).toContain("slowing the console down");
    expect(await page.locator(`iframe[data-plugin-id="${HOSTILE_ID}"]`).count()).toBe(0);
  }, 180000);

  test("watchdog: with an isolated frame, a spinning plugin is switched off and the console stays usable", async () => {
    const c = await isolated.newContext({ viewport: { width: 1280, height: 900 } });
    const pg = await c.newPage();
    try {
      await pg.goto(`${BASE}/`, { waitUntil: "networkidle" });
      await pg.waitForSelector(`iframe[data-plugin-id="${HOSTILE_ID}"]`, { timeout: 15000 });
      const fr = () => pg.frames().find((f) => f.url().includes(`/plugin-frame/${HOSTILE_ID}/`));
      await fr()!.waitForFunction(() => !!(window as any).__hostile, null, { timeout: 15000 });
      await pg.waitForTimeout(5000); // let it answer a ping at least once
      fr()!.evaluate(() => (window as any).__hostile.run("spin")).catch(() => {});
      const t0 = Date.now();
      await pg.waitForFunction((id) => !document.querySelector(`iframe[data-plugin-id="${id}"]`), HOSTILE_ID, { timeout: 40000 });
      note("watchdog removed spinning plugin after", `${Math.round((Date.now() - t0) / 1000)}s`);
      expect(await pg.locator(".toast, #os-toast-container").first().innerText().catch(() => "")).toContain("stopped responding");
      // The console is responsive and the plugin is still approved.
      await pg.keyboard.press("F2");
      await pg.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
      const state = await pg.evaluate(() => fetch("/api/plugins").then((r) => r.json()).then((l: any[]) => l.find((p) => p.id === "hostile_test").state));
      expect(state).toBe("active");
    } finally { await c.close(); }
  }, 120000);
});
