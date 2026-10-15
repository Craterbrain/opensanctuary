// Red-team: using the hostile plugin fixture, try to widen what it can do by reaching into a SECOND,
// legitimate plugin's scope -- its own keyring secret, a shared keyring namespace it owns but the
// hostile plugin was never granted, a command only the other plugin may send, a library category only
// the other plugin may write to, and its ribbon button. Everything here must fail.
import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page, type Frame } from "playwright";
import { spawnTestServer, teardownTestServer, webDirWithPlugins, pluginBridge, type SpawnedTestServer } from "./e2e_helpers";
import { HOSTILE_ID, hostileManifest, hostileHtml, hostileJs } from "./fixtures/hostile_plugin";

const VICTIM_ID = "victim_plugin";
const VICTIM_SHARED = "OpenSanctuary:Shared:victim_only";
const HOSTILE_SHARED = "OpenSanctuary:Shared:hostile_only"; // hostile IS granted this one, to prove it's namespace-scoped, not just "no grant at all"

const victimManifest = {
  manifest_version: 1, id: VICTIM_ID, name: "Victim Plugin", version: "1.0.0", publisher: "Test",
  web: { entry: "index.html", sandbox: true },
  permissions: [
    { id: "keyring.own", reason: "x" },
    { id: "keyring.service", services: [VICTIM_SHARED], access: "write", reason: "x" },
    { id: "commands.send", commands: ["ToggleBlackout"], reason: "x" },
    { id: "library.write", categories: ["media"], reason: "x" },
    { id: "ui.toolbar", reason: "x" },
  ],
  ui: { toolbar: [{ id: "victimbtn", label: "Victim", icon: "next", tooltip: "x" }] },
};
const victimHtml = `<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="__os/theme.css"></head><body><div id="s"></div>
<script src="__os/icons.js"></script><script src="__os/sdk.js"></script><script src="app.js"></script></body></html>`;
const victimJs = `
(function () {
  window.__ready = OS.call('keyring.set', { account: 'secret', secret: 'victim-own-value' })
    .then(function () { return OS.call('keyring.set', { service: '${VICTIM_SHARED}', account: 'k', secret: 'victim-shared-value' }); })
    .then(function () { return 'ready'; });
})();
`;

describe("E2E Red Team: a hostile plugin cannot widen a different, legitimate plugin's scope", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9087;

  const frameOf = (id: string): Frame | undefined => page.frames().find((f) => f.url().includes(`/plugin-frame/${id}/`));

  beforeAll(async () => {
    const webDir = webDirWithPlugins({
      [HOSTILE_ID]: {
        "plugin.json": JSON.stringify({
          ...hostileManifest,
          permissions: [
            ...hostileManifest.permissions,
            { id: "keyring.service", services: [HOSTILE_SHARED], access: "write", reason: "x" },
          ],
        }),
        "index.html": hostileHtml,
        "app.js": hostileJs,
      },
      [VICTIM_ID]: { "plugin.json": JSON.stringify(victimManifest), "index.html": victimHtml, "app.js": victimJs },
    });
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-cross-escalation-e2e-", webDir });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  }, 45000);
  afterAll(async () => { if (browser) await browser.close(); await teardownTestServer(server); });

  test("approve both, let the victim stock its own secrets, then attack from the hostile plugin", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    for (const id of [HOSTILE_ID, VICTIM_ID]) {
      const card = page.locator(`.plugin-card[data-plugin-id="${id}"]`);
      await card.waitFor({ state: "visible", timeout: 10000 });
      await card.locator("button:has-text('Approve')").first().click();
      await page.waitForFunction((i) => document.querySelector(`.plugin-card[data-plugin-id="${i}"] .plugin-state`)?.textContent?.includes("Approved"), id, { timeout: 10000 });
    }
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.waitForFunction(() => !!(window as any).__APP_READY__, null, { timeout: 15000 });
    await page.waitForSelector(`iframe[data-plugin-id="${VICTIM_ID}"]`, { state: "attached", timeout: 15000 });
    await page.waitForSelector(`iframe[data-plugin-id="${HOSTILE_ID}"]`, { state: "attached", timeout: 15000 });
    await frameOf(VICTIM_ID)!.evaluate(() => (window as any).__ready);
    await frameOf(HOSTILE_ID)!.waitForFunction(() => !!(window as any).__hostile, null, { timeout: 15000 });

    // Capture the hostile plugin's real x-plugin-token (minted legitimately by the console's bridge code)
    // so we can also hit the SERVER directly with a forged body, bypassing the bridge's own client-side
    // gate entirely -- this is what actually proves the server enforces the boundary, not just the
    // trusted console JS that sits between the sandbox and the network.
    let hostileToken: string | null = null;
    await page.route(`**/api/keyring/*`, async (route) => {
      const hdr = route.request().headers()["x-plugin-token"];
      if (hdr) hostileToken = hdr;
      await route.continue();
    });

    const call = pluginBridge(() => frameOf(HOSTILE_ID));

    // 1. Read the victim's own keyring secret directly.
    const ownAttempt = await call("keyring.get", { service: "OpenSanctuary:Plugin:" + VICTIM_ID, account: "secret" });
    expect(ownAttempt.ok).toBe(false);

    // Legitimate call first (own shared namespace), just to capture the token via the route above.
    await call("keyring.set", { service: HOSTILE_SHARED, account: "capture", secret: "x" });
    expect(hostileToken).not.toBeNull();

    // Same attack, server-side, with the bridge's own gate bypassed entirely: a raw HTTP request, carrying
    // the hostile plugin's real (legitimately minted) token, straight to the API the bridge would have called.
    const direct = async (op: string, body: Record<string, unknown>) =>
      fetch(`http://127.0.0.1:${PORT}/api/keyring/${op}`, { method: "POST", headers: { "content-type": "application/json", "x-plugin-token": hostileToken! }, body: JSON.stringify(body) });
    const directReadOwn = await direct("get", { service: `OpenSanctuary:Plugin:${VICTIM_ID}`, account: "secret" });
    const directOwnBody: any = await directReadOwn.json();
    expect(directReadOwn.status === 403 || directOwnBody.secret == null).toBe(true);
    const directReadShared = await direct("get", { service: VICTIM_SHARED, account: "k" });
    const directSharedBody: any = await directReadShared.json();
    expect(directReadShared.status === 403 || directSharedBody.secret == null).toBe(true);
    const directWriteShared = await direct("set", { service: VICTIM_SHARED, account: "k", secret: "pwned-direct" });
    expect(directWriteShared.status).not.toBe(200);
    console.log("direct server-side attempts:", JSON.stringify({ directOwnBody, directSharedBody, directWriteStatus: directWriteShared.status }));

    // 2. Read / write / delete the victim's shared namespace, which the hostile plugin was never granted
    //    (it was granted a *different* shared namespace, so this proves namespace scoping, not just "no grant at all").
    const readVictimShared = await call("keyring.get", { service: VICTIM_SHARED, account: "k" });
    expect(readVictimShared.ok && readVictimShared.result?.secret).toBeFalsy();
    const writeVictimShared = await call("keyring.set", { service: VICTIM_SHARED, account: "k", secret: "pwned" });
    expect(writeVictimShared.ok).toBe(false);
    const deleteVictimShared = await call("keyring.delete", { service: VICTIM_SHARED, account: "k" });
    expect(deleteVictimShared.ok).toBe(false);
    // Its own shared namespace still works -- this isn't "keyring.service is broken", it's scoped per name.
    const ownShared = await call("keyring.set", { service: HOSTILE_SHARED, account: "k", secret: "mine" });
    expect(ownShared.ok).toBe(true);

    // 3. A command only the victim may send.
    const stolenCommand = await call("sendCommand", { command: "ToggleBlackout" });
    expect(stolenCommand.ok).toBe(false);

    // 4. A library category only the victim may write to.
    const stolenCategory = await call("library.addMediaUrl", { url: "https://example.com/x.jpg", title: "x" });
    expect(stolenCategory.ok).toBe(false);

    // 5. The victim's own ribbon button, by id.
    const stolenToolbar = await call("ui.toolbar.update", { id: "victimbtn", pressed: true });
    expect(stolenToolbar.ok).toBe(false);

    // The victim's secrets are provably untouched: it can still read what it wrote (another realm, its own call).
    const victimCall = pluginBridge(() => frameOf(VICTIM_ID));
    const victimOwn = await victimCall("keyring.get", { account: "secret" });
    expect(victimOwn.ok && victimOwn.result?.secret).toBe("victim-own-value");
    const victimShared = await victimCall("keyring.get", { service: VICTIM_SHARED, account: "k" });
    expect(victimShared.ok && victimShared.result?.secret).toBe("victim-shared-value");

    console.log("\n=== CROSS-PLUGIN ESCALATION ATTEMPTS ===\n" + JSON.stringify({
      read_victim_own: ownAttempt, read_victim_shared: readVictimShared, write_victim_shared: writeVictimShared,
      delete_victim_shared: deleteVictimShared, stolen_command: stolenCommand, stolen_category: stolenCategory, stolen_toolbar: stolenToolbar,
    }, null, 2));
  }, 60000);
});
