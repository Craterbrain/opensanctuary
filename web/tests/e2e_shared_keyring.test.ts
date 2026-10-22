import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, webDirWithPlugins, pluginBridge, type SpawnedTestServer } from "./e2e_helpers";

const SVC = "OpenSanctuary:Shared:e2e-pexels";
const plugin = (id: string, perms: unknown[]) => ({
  "index.html": "<!doctype html><html><body></body></html>",
  "plugin.json": JSON.stringify({ manifest_version: 1, id, name: `Plugin ${id}`, web: { entry: "index.html", sandbox: true }, permissions: perms }),
});

describe("E2E Live Test: keyring.service shares a keyring namespace between plugins", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9079;
  let artifactDir: string;

  beforeAll(async () => {
    const webDir = webDirWithPlugins({
      kw_writer: plugin("kw_writer", [
        { id: "keyring.service", services: [SVC], access: "write", reason: "Stores the shared key" },
        { id: "network.outbound", hosts: ["example.com"] },
      ]),
      kw_reader: plugin("kw_reader", [{ id: "keyring.service", services: [SVC], access: "read" }]),
    });
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-shared-keyring-e2e-", webDir });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  const frameOf = (id: string) => () => page.frames().find((f) => f.url().includes(`/plugin-frame/${id}/`));

  test("the review screen is blunt about it, and the bridge enforces who may read, write and delete", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();

    const writer = page.locator('.plugin-card[data-plugin-id="kw_writer"]');
    await writer.waitFor({ state: "visible", timeout: 10000 });
    const text = await writer.innerText();
    expect(text).toContain("keyring.service");
    expect(text).toContain("Critical");
    expect(text).toContain(`Can read secrets stored in the shared keyring area "e2e-pexels" (${SVC}), and store its own secrets there for other plugins to read`);
    expect(text).toContain("Shared with: Plugin kw_reader (read, not approved yet)");
    expect(text).toContain("It can also reach example.com. A shared secret could be sent there");
    await page.screenshot({ path: `${artifactDir}/screenshot_shared_keyring_review.png` });
    await writer.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="kw_writer"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );
    const reader = page.locator('.plugin-card[data-plugin-id="kw_reader"]');
    await reader.locator("button:has-text('Approve')").click();
    await page.waitForFunction(
      () => document.querySelector('.plugin-card[data-plugin-id="kw_reader"] .plugin-state')?.textContent?.includes("Approved"),
      null, { timeout: 10000 },
    );
    await page.keyboard.press("Escape");

    // Both plugins run (hidden frames); drive the bridge the way their pages would.
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="kw_writer"]') && document.querySelector('iframe[data-plugin-id="kw_reader"]'), null, { timeout: 15000 });
    await Promise.all(["kw_writer", "kw_reader"].map((id) => frameOf(id)()!.waitForLoadState("load")));
    const w = pluginBridge(frameOf("kw_writer"));
    const r = pluginBridge(frameOf("kw_reader"));

    const put = await w("keyring.set", { service: SVC, account: "api_key", secret: "shared-secret-123" });
    expect(put.error ?? "").toBe("");
    expect(put.ok).toBe(true);
    const got = await r("keyring.get", { service: SVC, account: "api_key" });
    expect(got.result.secret).toBe("shared-secret-123");

    // The reader has read access only; and no plugin may use a namespace it wasn't granted or its own without keyring.own.
    const swap = await r("keyring.set", { service: SVC, account: "api_key", secret: "swapped" });
    expect(swap.ok).toBe(false);
    expect(swap.error).toContain("read-only");
    expect((await r("keyring.delete", { service: SVC, account: "api_key" })).ok).toBe(false);
    const other = await w("keyring.get", { service: "OpenSanctuary:Shared:not-granted", account: "x" });
    expect(other.ok).toBe(false);
    expect(other.error).toContain("not granted");
    const own = await w("keyring.get", { account: "x" }); // no service: its own namespace, which needs keyring.own
    expect(own.ok).toBe(false);
    expect(own.error).toContain("keyring.own");
    expect((await r("keyring.get", { service: "OpenSanctuary:System", account: "x" })).ok).toBe(false);
    expect((await r("keyring.get", { service: "OpenSanctuary:Plugin:kw_writer", account: "api_key" })).ok).toBe(false);
    // Still the writer's value.
    expect((await r("keyring.get", { service: SVC, account: "api_key" })).result.secret).toBe("shared-secret-123");

    // The console lists the shared entry (never its value) and can clear it.
    await page.keyboard.press("F2");
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await page.locator(".settings-category-btn:has-text('Plugins')").click();
    const shared = page.locator(".plugin-shared-secrets");
    await shared.waitFor({ state: "visible", timeout: 10000 });
    await shared.locator("summary").click();
    const row = shared.locator(".plugin-shared-secret").first();
    expect(await row.innerText()).toContain(`${SVC} / api_key`);
    expect(await row.innerText()).toContain("shared:plugin:kw_writer");
    expect(await shared.innerText()).not.toContain("shared-secret-123");
    await page.screenshot({ path: `${artifactDir}/screenshot_shared_secrets.png` });
    await row.locator("button:has-text('Delete')").click();
    await page.waitForFunction(() => !document.querySelector(".plugin-shared-secrets"), null, { timeout: 10000 });
    expect((await r("keyring.get", { service: SVC, account: "api_key" })).result.secret).toBeNull();
  }, 90000);
});
