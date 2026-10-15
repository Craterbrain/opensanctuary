import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

/** Builds a zip with python's zipfile (no zip library in the web tests). */
function zipOf(dir: string, name: string, files: Record<string, string>): string {
  const src = join(dir, name);
  mkdirSync(src, { recursive: true });
  for (const [f, t] of Object.entries(files)) { mkdirSync(join(src, f, ".."), { recursive: true }); writeFileSync(join(src, f), t); }
  const out = join(dir, `${name}.zip`);
  const r = Bun.spawnSync(["python3", "-c", "import sys,zipfile,os\nz=zipfile.ZipFile(sys.argv[2],'w',zipfile.ZIP_DEFLATED)\nfor r,_,fs in os.walk(sys.argv[1]):\n  for f in fs:\n    p=os.path.join(r,f); z.write(p, os.path.relpath(p,sys.argv[1]))\nz.close()", src, out]);
  if (r.exitCode !== 0) throw new Error("zip failed");
  return out;
}

const manifest = (id: string) => JSON.stringify({
  manifest_version: 1, id, name: "Installed Hello", version: "1.0.0", publisher: "Me", description: "e2e",
  web: { entry: "index.html", sandbox: true },
  permissions: [{ id: "ui.panel", reason: "x" }], ui: { placements: [{ slot: "dock" }] },
});
const html = `<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="__os/theme.css"></head><body><div id="hi">hello from an installed plugin</div><script src="__os/sdk.js"></script></body></html>`;

const hasMinisign = Bun.which("minisign") !== null;

/** A signed package: files listed in the manifest (or not), manifest signed with a throwaway minisign key. */
function signedPackage(dir: string, name: string, id: string, opts: { listFiles: boolean; tamper?: boolean }): string {
  const src = join(dir, `${name}-src`);
  mkdirSync(src, { recursive: true });
  writeFileSync(join(src, "index.html"), html);
  writeFileSync(join(src, "app.js"), "console.log('hello');");
  const sh = (args: string[]) => { const r = Bun.spawnSync(args, { cwd: dir }); if (r.exitCode !== 0) throw new Error(`${args[0]} failed: ${r.stderr}`); };
  const pub = join(dir, `${name}.pub`), sec = join(dir, `${name}.key`);
  sh(["minisign", "-G", "-W", "-f", "-p", pub, "-s", sec]);
  const publisher_key = readFileSync(pub, "utf8").trim().split("\n")[1];
  const m: any = JSON.parse(manifest(id));
  m.publisher_key = publisher_key;
  if (opts.listFiles) {
    m.files = Object.fromEntries(["index.html", "app.js"].map((f) => [f, new Bun.CryptoHasher("sha256").update(readFileSync(join(src, f))).digest("hex")]));
  }
  writeFileSync(join(src, "plugin.json"), JSON.stringify(m, null, 2));
  sh(["minisign", "-S", "-s", sec, "-m", join(src, "plugin.json"), "-x", join(src, "plugin.json.minisig")]);
  if (opts.tamper) writeFileSync(join(src, "app.js"), "steal();");
  const out = join(dir, `${name}.zip`);
  const r = Bun.spawnSync(["python3", "-c", "import sys,zipfile,os\nz=zipfile.ZipFile(sys.argv[2],'w',zipfile.ZIP_DEFLATED)\nfor r,_,fs in os.walk(sys.argv[1]):\n  for f in fs:\n    p=os.path.join(r,f); z.write(p, os.path.relpath(p,sys.argv[1]))\nz.close()", src, out]);
  if (r.exitCode !== 0) throw new Error("zip failed");
  return out;
}

describe("E2E: install, review, update and remove a plugin from a package", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9081;
  const work = mkdtempSync(join(tmpdir(), "os-next-pkg-"));

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-install-e2e-" });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  const open = async () => {
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    await page.locator(".plugin-install").waitFor({ state: "visible", timeout: 10000 });
  };
  const install = async (zip: string) => {
    const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.locator(".plugin-install").click()]);
    await chooser.setFiles(zip);
  };
  const card = () => page.locator('.plugin-card[data-plugin-id="pkg_hello"]');

  test("install -> needs review -> approve -> runs -> update -> remove", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await open();
    // Built-ins can be revoked, not removed.
    expect(await page.locator('.plugin-card[data-plugin-id="sandbox_demo"] .plugin-remove').count()).toBe(0);

    await install(zipOf(work, "v1", { "plugin.json": manifest("pkg_hello"), "index.html": html }));
    await card().waitFor({ state: "visible", timeout: 10000 });
    expect(await card().locator(".plugin-state").first().innerText()).toContain("Needs review");
    expect(await page.locator("iframe[data-plugin-id='pkg_hello']").count()).toBe(0); // nothing runs before approval
    await card().locator("button:has-text('Approve')").first().click();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="pkg_hello"] .plugin-state')?.textContent?.includes("Approved"), null, { timeout: 10000 });
    await page.waitForSelector("iframe[data-plugin-id='pkg_hello']", { timeout: 10000 });
    const frame = page.frames().find((f) => f.url().includes("/plugin-frame/pkg_hello/"))!;
    await frame.waitForFunction(() => document.getElementById("hi")?.textContent?.includes("installed plugin"), null, { timeout: 10000 });

    // Installing the same id again without "update" is refused with a readable reason.
    await install(zipOf(work, "v1b", { "plugin.json": manifest("pkg_hello"), "index.html": html }));
    await page.waitForFunction(() => document.body.innerText.includes("already installed"), null, { timeout: 5000 });

    // Update: the plugin stops until its new files are reviewed.
    const [chooser] = await Promise.all([page.waitForEvent("filechooser"), card().locator(".plugin-update").click()]);
    await chooser.setFiles(zipOf(work, "v2", { "plugin.json": manifest("pkg_hello"), "index.html": html.replace("hello from", "UPDATED hello from") }));
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="pkg_hello"] .plugin-state')?.textContent?.includes("Changed"), null, { timeout: 10000 });
    // The review says what is different from the version you approved.
    const changes = await card().locator(".plugin-changes").innerText();
    expect(changes).toContain("edited: index.html");
    expect(changes).toContain("No new permissions");
    expect(changes).toContain("version number didn't change");

    // Remove: the files, the card and the running frame all go.
    page.once("dialog", (d) => d.accept());
    await card().locator(".plugin-remove").click();
    await card().waitFor({ state: "detached", timeout: 10000 });
    expect(await page.locator("iframe[data-plugin-id='pkg_hello']").count()).toBe(0);
    expect(existsSync(join(server.testDir, "plugins", "pkg_hello"))).toBe(false);
    expect((await fetch(`http://127.0.0.1:${PORT}/plugin-frame/pkg_hello/index.html`)).status).toBe(404);
  }, 90000);

  test("a broken package is refused with a reason and installs nothing", async () => {
    const bad = join(work, "not-a-zip.zip");
    writeFileSync(bad, "this is not a zip");
    await install(bad);
    await page.waitForFunction(() => document.body.innerText.includes("isn't a zip"), null, { timeout: 5000 });
    await install(zipOf(work, "nomanifest", { "index.html": html }));
    await page.waitForFunction(() => document.body.innerText.includes("no plugin.json"), null, { timeout: 5000 });
    const dir = join(server.testDir, "plugins");
    expect(existsSync(dir) ? readdirSync(dir) : []).toEqual([]);
  }, 30000);

  test.skipIf(!hasMinisign)("signature labels say what the signature covers, and a tampered signed package is refused", async () => {
    await page.reload({ waitUntil: "networkidle" });
    await open();
    const label = async (id: string) => (await page.locator(`.plugin-card[data-plugin-id="${id}"] .plugin-id-line`).innerText());

    await install(signedPackage(work, "full", "sig_full", { listFiles: true }));
    await page.locator('.plugin-card[data-plugin-id="sig_full"]').waitFor({ state: "visible", timeout: 10000 });
    const full = await label("sig_full");
    expect(full).toContain("signed (publisher key");
    expect(full).toContain("covers every file");
    expect(full).not.toContain("manifest signed");

    await install(signedPackage(work, "manifestonly", "sig_manifest", { listFiles: false }));
    await page.locator('.plugin-card[data-plugin-id="sig_manifest"]').waitFor({ state: "visible", timeout: 10000 });
    const weak = await label("sig_manifest");
    expect(weak).toContain("manifest signed");
    expect(weak).toContain("aren't covered");

    // A file edited after signing: nothing is installed, and the reason names what is wrong.
    await install(signedPackage(work, "tampered", "sig_tampered", { listFiles: true, tamper: true }));
    await page.waitForFunction(() => document.body.innerText.includes("doesn't match its manifest"), null, { timeout: 5000 });
    expect(await page.locator('.plugin-card[data-plugin-id="sig_tampered"]').count()).toBe(0);
    expect(existsSync(join(server.testDir, "plugins", "sig_tampered"))).toBe(false);
  }, 60000);
});
