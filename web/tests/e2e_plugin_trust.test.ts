import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnTestServer, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";
import { signedPackage, hasMinisign } from "./fixtures/plugin_packages";

describe.skipIf(!hasMinisign)("E2E: publisher trust, badges and revocation", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9083;
  const work = mkdtempSync(join(tmpdir(), "os-next-trust-"));
  let artifactDir: string;

  beforeAll(async () => {
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-trust-e2e-" });
    artifactDir = ensureArtifactDir();
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1280, height: 1000 } });
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
  const card = (id: string) => page.locator(`.plugin-card[data-plugin-id="${id}"]`);
  const badge = async (id: string) => (await card(id).locator(".plugin-trust").innerText());

  test("badges, trust at approval, revoke, and approving a revoked publisher's plugin", async () => {
    await page.goto(`http://127.0.0.1:${PORT}/`, { waitUntil: "networkidle" });
    await open();

    // Built-in and new/unsigned at first sight.
    expect(await badge("sandbox_demo")).toBe("Built-in");
    await install(signedPackage(work, "acme1", "acme_one", { listFiles: true, keyName: "acme", publisher: "Acme Ltd" }));
    await card("acme_one").waitFor({ state: "visible", timeout: 10000 });
    expect(await badge("acme_one")).toBe("New publisher");
    // A name the plugin wrote about itself is shown as a claim, never as fact.
    expect(await card("acme_one").locator(".plugin-id-line").innerText()).toContain("says it is by Acme Ltd");
    // Built-ins offer no trust choice; a new signed publisher does.
    expect(await card("sandbox_demo").locator(".plugin-trust-box").count()).toBe(0);
    expect(await card("acme_one").locator(".plugin-trust-box").count()).toBe(1);

    // Approve and trust in one go.
    await card("acme_one").locator(".plugin-trust-box").check();
    await card("acme_one").locator("button:has-text('Approve')").first().click();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="acme_one"] .plugin-state')?.textContent?.includes("Approved"), null, { timeout: 10000 });
    expect(await badge("acme_one")).toBe("Trusted publisher");
    const pubs = page.locator(".plugin-publisher");
    await page.locator(".plugin-publishers").waitFor({ state: "visible" });
    expect(await pubs.count()).toBe(1);
    expect(await pubs.first().innerText()).toContain("Acme Ltd");

    // A second plugin from the same key is trusted on sight; another key is not.
    await install(signedPackage(work, "acme2", "acme_two", { listFiles: true, keyName: "acme", publisher: "Acme Ltd" }));
    await card("acme_two").waitFor({ state: "visible", timeout: 10000 });
    expect(await badge("acme_two")).toBe("Trusted publisher");
    // Once you trusted the key, the name is the one you saw then.
    expect(await card("acme_two").locator(".plugin-id-line").innerText()).toMatch(/ · by Acme Ltd/);
    await install(signedPackage(work, "rival", "rival_one", { listFiles: true, keyName: "rival", publisher: "Rival Co" }));
    await card("rival_one").waitFor({ state: "visible", timeout: 10000 });
    expect(await badge("rival_one")).toBe("New publisher");
    await page.screenshot({ path: `${artifactDir}/screenshot_plugin_trust.png`, fullPage: false });

    // Revoke the publisher: what it signed stops, the other card shows why.
    page.once("dialog", (d) => d.accept());
    await pubs.first().locator(".plugin-publisher-revoke").click();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="acme_one"] .plugin-state')?.textContent?.includes("Revoked"), null, { timeout: 10000 });
    expect(await badge("acme_two")).toBe("Revoked publisher");
    expect(await badge("rival_one")).toBe("New publisher");
    expect(await page.locator("iframe[data-plugin-id='acme_one']").count()).toBe(0);

    // Approving one of its plugins asks first, and declining changes nothing.
    // (click() returns only once the confirm is answered, so this checks it was really asked.)
    let asked = "";
    page.once("dialog", (d) => { asked = d.message(); void d.dismiss(); });
    await card("acme_two").locator("button:has-text('Approve')").first().click();
    expect(asked).toContain("revoked");
    expect(await card("acme_two").locator(".plugin-state").first().innerText()).not.toContain("Approved");
    page.once("dialog", (d) => { expect(d.message()).toContain("revoked"); void d.accept(); });
    await card("acme_two").locator("button:has-text('Approve')").first().click();
    await page.waitForFunction(() => document.querySelector('.plugin-card[data-plugin-id="acme_two"] .plugin-state')?.textContent?.includes("Approved"), null, { timeout: 10000 });
    // A revoked key can't be trusted again from the card.
    expect(await card("acme_two").locator(".plugin-trust-now").count()).toBe(0);
  }, 120000);
});
