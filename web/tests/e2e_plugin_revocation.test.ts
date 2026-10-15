// Phase 3 of plugin signing end to end: the release key vouches for the app's own plugins and signs the revocation
// list. Uses the throwaway keys in testdata/signing/ and the real binary started with --test-release-key.
import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync, copyFileSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, webDirWithPlugins, type SpawnedTestServer } from "./e2e_helpers";
import { signedPackage, hasMinisign, manifest, html } from "./fixtures/plugin_packages";

const KEYS = resolve(import.meta.dir, "../../testdata/signing");
const pubB64 = (name: string) => readFileSync(join(KEYS, `${name}.pub`), "utf8").trim().split("\n")[1];

describe.skipIf(!hasMinisign)("E2E: release key, signed built-in plugin and the revocation list", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  let webDir: string;
  const PORT = 9085;
  const work = mkdtempSync(join(tmpdir(), "os-next-revoke-"));
  let artifactDir: string;
  const sh = (args: string[]) => { const r = Bun.spawnSync(args, { cwd: work }); if (r.exitCode !== 0) throw new Error(`${args.join(" ")}: ${r.stderr}`); return r.stdout.toString(); };

  beforeAll(async () => {
    for (const k of ["release", "acme", "rival"]) { copyFileSync(join(KEYS, `${k}.key`), join(work, `${k}.key`)); copyFileSync(join(KEYS, `${k}.pub`), join(work, `${k}.pub`)); }
    // A "bundled" plugin the release key signed, as the release process would (hash, set the key, sign).
    webDir = webDirWithPlugins({ official_demo: { "plugin.json": manifest("official_demo", { name: "Official Demo", publisher: "OpenSanctuary" }), "index.html": html } });
    const folder = join(webDir, "plugins", "official_demo");
    const m = JSON.parse(readFileSync(join(folder, "plugin.json"), "utf8"));
    m.publisher_key = pubB64("release");
    writeFileSync(join(folder, "plugin.json"), JSON.stringify(m));
    sh([resolve(import.meta.dir, "../../target/release/os-next"), "--hash-plugin", folder]);
    sh(["minisign", "-S", "-s", join(work, "release.key"), "-m", join(folder, "plugin.json"), "-x", join(folder, "plugin.json.minisig")]);
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-revoke-e2e-", webDir, extraArgs: ["--test-release-key", pubB64("release")] });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  }, 60000);
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
  const card = (id: string) => page.locator(`.plugin-card[data-plugin-id="${id}"]`);
  const state = (id: string) => card(id).locator(".plugin-state").first().innerText();

  test("release-signed built-in, then a signed revocation list stops a publisher", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await open();

    // The bundled plugin the release key signed says so; the bundled one without a signature (this repo's own) doesn't.
    await card("official_demo").waitFor({ state: "visible", timeout: 10000 });
    expect(await card("official_demo").locator(".plugin-trust").innerText()).toBe("Built-in");
    const line = await card("official_demo").locator(".plugin-id-line").innerText();
    expect(line).toContain("by OpenSanctuary");
    expect(line).not.toContain("says it is by");
    expect(line).toContain("signed by the OpenSanctuary release key");
    expect(await card("sandbox_demo").locator(".plugin-id-line").innerText()).toContain("unsigned");

    // Two third-party publishers, both approved.
    await install(signedPackage(work, "acme1", "acme_one", { listFiles: true, keyName: "acme", publisher: "Acme Ltd" }));
    await install(signedPackage(work, "rival1", "rival_one", { listFiles: true, keyName: "rival", publisher: "Rival Co" }));
    for (const id of ["acme_one", "rival_one"]) {
      await card(id).waitFor({ state: "visible", timeout: 10000 });
      await card(id).locator("button:has-text('Approve')").first().click();
      await page.waitForFunction((i) => document.querySelector(`.plugin-card[data-plugin-id="${i}"] .plugin-state`)?.textContent?.includes("Approved"), id, { timeout: 10000 });
    }
    await page.waitForSelector("iframe[data-plugin-id='rival_one']", { timeout: 10000 });
    expect(await page.locator(".plugin-revocation-status").count()).toBe(1);
    expect(await page.locator(".plugin-revocation-status").textContent()).toContain("No revocation list");

    // A list signed by the wrong key is ignored, and the console says so.
    const list = (seq: number, key: string, reason: string) => JSON.stringify({ sequence: seq, revoked: [{ key, reason, since: "2026-10-14" }] });
    const sign = (keyName: string) => sh(["minisign", "-S", "-s", join(work, `${keyName}.key`), "-m", join(webDir, "revoked_keys.json"), "-x", join(webDir, "revoked_keys.json.minisig")]);
    writeFileSync(join(webDir, "revoked_keys.json"), list(1, pubB64("rival"), "forged"));
    sign("acme");
    await page.reload({ waitUntil: "networkidle" });
    await open();
    expect(await state("rival_one")).toContain("Approved");
    await page.waitForFunction(() => document.body.innerText.includes("A revocation list was ignored"), null, { timeout: 5000 });

    // The genuine list (signed by the release key) revokes the rival publisher.
    writeFileSync(join(webDir, "revoked_keys.json"), list(1, pubB64("rival"), "signing key leaked"));
    sign("release");
    await page.reload({ waitUntil: "networkidle" });
    await open();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="rival_one"] .plugin-state')?.textContent?.includes("Revoked"), null, { timeout: 10000 });
    expect(await card("rival_one").locator(".plugin-trust").innerText()).toBe("Revoked publisher");
    expect(await state("acme_one")).toContain("Approved");
    expect(await card("official_demo").locator(".plugin-trust").innerText()).toBe("Built-in");
    expect(await page.locator("iframe[data-plugin-id='rival_one']").count()).toBe(0); // not started after the reload
    const status = await page.locator(".plugin-revocation-status").innerText();
    expect(status).toContain("Revocation list #1 applied");
    expect(await page.locator(".plugin-publisher").first().innerText()).toContain("signing key leaked");
    await page.locator(".plugin-publishers").scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_revocation.png` });
  }, 120000);
});
