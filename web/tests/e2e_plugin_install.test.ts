import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, writeFileSync, existsSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";
import { zipOf, signedPackage, manifest, html, hasMinisign } from "./fixtures/plugin_packages";

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
