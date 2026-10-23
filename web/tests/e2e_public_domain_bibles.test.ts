import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";
import { spawnTestServer, teardownTestServer, pollUntil, type SpawnedTestServer } from "./e2e_helpers";

// The public-domain Bibles the project hosts (github.com/Craterbrain/opensanctuary-bibles) must be
// reachable from a fresh install with no Bible in it: the empty Scriptures tab points at the plugin,
// the plugin lists our translations first, and installing one stores the real text. Uses the live
// repository (like the other plugin e2e tests use their live hosts).
describe("E2E: public-domain Bibles from the OpenSanctuary Bibles repository", () => {
  let server: SpawnedTestServer;
  let browser: Browser;
  let page: Page;
  const PORT = 9032;
  const base = `http://127.0.0.1:${PORT}`;
  const frameOf = () => page.frames().find((f) => f.url().includes("/plugin-frame/online_bibles/"));

  beforeAll(async () => {
    const biblesDir = mkdtempSync(join(tmpdir(), "os-next-pd-bibles-"));
    server = await spawnTestServer({ port: PORT, tempPrefix: "os-next-pd-bibles-e2e-", extraArgs: ["--bibles-dir", biblesDir] });
    browser = await chromium.launch({ headless: true });
    page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${base}/`);
    await page.waitForFunction(() => (window as any).__APP_READY__ === true, null, { timeout: 20000 });
  }, 45000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(server);
  });

  test("an empty Scriptures tab leads to the plugin, and from there to a public-domain Bible", async () => {
    await page.click('.tab-btn[data-tab="scriptures"]');
    await page.waitForSelector("text=No Bible versions installed.", { timeout: 10000 });

    // Not turned on yet: the button opens the plugin's review card.
    const getBibles = page.locator("#btn-get-bibles");
    await getBibles.waitFor({ state: "visible", timeout: 10000 });
    expect(await getBibles.innerText()).toContain("Online Bibles");
    await getBibles.click();
    await page.waitForSelector("#options-modal", { state: "visible", timeout: 8000 });
    const card = page.locator('.plugin-card[data-plugin-id="online_bibles"]');
    await card.waitFor({ state: "visible", timeout: 10000 });
    await card.locator("button:has-text('Approve')").click();
    await page.waitForFunction(() => document.querySelector('iframe[data-plugin-id="online_bibles"]'), null, { timeout: 15000 });
    await page.keyboard.press("Escape");

    // Turned on: the same button now opens the plugin's page.
    await page.locator("#btn-get-bibles").click();
    const frame = await pollUntil(async () => frameOf(), (f) => !!f, 10000, "the plugin page");
    await frame!.waitForSelector("#catalog-list li", { state: "visible", timeout: 40000 });

    // Our translations are listed first, marked as public domain from this repository.
    await frame!.waitForFunction(() => /OpenSanctuary Bibles/.test(document.querySelector("#catalog-list li .sub")?.textContent ?? ""), null, { timeout: 40000 });
    const first = await frame!.locator("#catalog-list li").first().innerText();
    expect(first).toContain("OpenSanctuary Bibles");
    expect(first).toContain("public domain");

    // Install the World English Bible from it.
    await frame!.fill("#catalog-q", "WEB");
    const row = frame!.locator('#catalog-list li:has(.abbr:text-is("WEB")):has-text("OpenSanctuary Bibles")').first();
    await row.locator("button").click();
    // (The Bible shows up after the first batch of chapters; the plugin says when the last one is saved.)
    await frame!.waitForFunction(() => /^Installed WEB: /.test(document.getElementById("status")?.textContent ?? ""), null, { timeout: 120000 });
    const bibles = (await (await fetch(`${base}/api/bibles`)).json()) as any[];
    // (`verse_count` counts stored chapters: all 1,189 of the 66 books.)
    expect(bibles.find((b) => b.id === "WEB").verse_count).toBe(1189);

    // The real text is there, readable like any installed Bible.
    const john = await (await fetch(`${base}/api/bibles/passage?book=John&chapter=3&version=WEB&verse_start=16&verse_end=16`)).json();
    expect(JSON.stringify(john)).toContain("For God so loved the world");
    const psalms = await (await fetch(`${base}/api/bibles/passage?book=Psalms&chapter=23&version=WEB&verse_start=1&verse_end=1`)).json();
    expect(JSON.stringify(psalms)).toContain("shepherd");
  }, 240000);

  // The Douay-Rheims carries the deuterocanonical books. Their verses can be long (the letters in the Greek Esther
  // additions are past 4600 bytes in some translations), so this also covers the host's per-verse limit.
  test("a Bible with the deuterocanon installs whole, books and all", async () => {
    const frame = frameOf()!;
    await frame.fill("#catalog-q", "DRB");
    const row = frame.locator('#catalog-list li:has(.abbr:text-is("DRB")):has-text("OpenSanctuary Bibles")').first();
    await row.locator("button").evaluate((el: HTMLElement) => el.click());
    await frame.waitForFunction(() => /^Installed DRB: /.test(document.getElementById("status")?.textContent ?? ""), null, { timeout: 120000 });
    const tobit = await (await fetch(`${base}/api/bibles/passage?book=Tobit&chapter=1&version=DRB&verse_start=1&verse_end=1`)).json();
    expect(JSON.stringify(tobit)).toContain("Tobias");
    const bibles = (await (await fetch(`${base}/api/bibles`)).json()) as any[];
    expect(bibles.find((b) => b.id === "DRB").verse_count).toBe(1334); // 1,189 chapters plus the deuterocanon
  }, 180000);
});
