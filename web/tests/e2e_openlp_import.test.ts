import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { resolve } from "path";
import { spawnTestServer, teardownTestServer, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Live Test: OpenLP File & SQLite Ingestion", () => {
    let server: SpawnedTestServer;
    let browser: Browser;
    let page: Page;
    const PORT = 9005;
    const OSZ_PATH = resolve(__dirname, "../../tests/fixtures/openlp/test.osz");
    const OSJ_PATH = resolve(__dirname, "../../tests/fixtures/openlp/serviceitem-song.osj");
    const SONGS_SQLITE_PATH = resolve(__dirname, "../../tests/fixtures/openlp/songs-2.4.6.sqlite");

    beforeAll(async () => {
        // Isolated OS temp dir: Database::new derives songs/bibles folders
        // from the db path's parent, so a project-root db path (the old
        // behavior here) meant every e2e run silently mutated the real
        // song library.
        server = await spawnTestServer({ port: PORT });

        // Launch Chromium
        browser = await chromium.launch({ headless: true });
        page = await browser.newPage();
        page.on('console', msg => console.log('PAGE LOG:', msg.type(), msg.text()));
        page.on('pageerror', err => console.log('PAGE ERROR:', err));
        page.on('requestfailed', req => console.log('REQ FAILED:', req.url(), req.failure()?.errorText));
        page.on('response', async res => {
            if (res.url().includes('/api/schedule/open')) {
                const text = await res.text().catch(() => '');
                console.log('SCHEDULE OPEN RES:', text);
            }
        });

        await page.goto(`http://127.0.0.1:${PORT}/`);
        await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
        // Not waitForLoadState("networkidle") -- the app's persistent
        // time-sync WebSocket means "idle" may never fire; __APP_READY__
        // above is the real readiness signal (see main.ts).
    }, 45000);

    afterAll(async () => {
        if (browser) await browser.close();
        await teardownTestServer(server);
    });

    test("Unified Schedule Hub: Ingest OpenLP .osz service archive (Replace mode)", async () => {
        // 1. Open Schedule Hub modal
        await page.click("#btn-sched-open");
        await page.waitForSelector("#open-modal", { state: "visible" });

        // Verify Replace mode is default
        const replaceRadioChecked = await page.$eval("#sched-mode-replace", (el: any) => el.checked);
        expect(replaceRadioChecked).toBe(true);

        // 2. Select OpenLP .osz file
        const fileInput = await page.$("#file-schedule-input");
        expect(fileInput).not.toBeNull();
        await fileInput!.setInputFiles(OSZ_PATH);

        // Modal should close upon loading
        await page.waitForSelector("#open-modal", { state: "hidden", timeout: 10000 });

        // Verify schedule items loaded from test.osz
        await page.waitForFunction(() => document.querySelectorAll(".schedule-item").length >= 2, { timeout: 10000 });
        const itemTitles = await page.$$eval(".schedule-item .title", els => els.map(e => e.textContent?.trim()));
        expect(itemTitles.length).toBeGreaterThanOrEqual(2);
        expect(itemTitles.some(t => t?.includes("Safe Stronghold") || t?.includes("All the Way"))).toBe(true);
    }, 20000);

    test("Unified Schedule Hub: Append OpenLP .osj service item JSON", async () => {
        const initialCount = (await page.$$(".schedule-item")).length;

        // 1. Open Schedule Hub modal
        await page.click("#btn-sched-open");
        await page.waitForSelector("#open-modal", { state: "visible" });

        // 2. Switch to Append mode
        await page.click("#sched-mode-append");
        const appendRadioChecked = await page.$eval("#sched-mode-append", (el: any) => el.checked);
        expect(appendRadioChecked).toBe(true);

        // 3. Select OpenLP .osj service item file
        const fileInput = await page.$("#file-schedule-input");
        expect(fileInput).not.toBeNull();
        await fileInput!.setInputFiles(OSJ_PATH);

        await page.waitForSelector("#open-modal", { state: "hidden", timeout: 10000 });

        // Verify new count is initial + 1
        await page.waitForFunction(
            (expected) => document.querySelectorAll(".schedule-item").length >= expected,
            initialCount + 1,
            { timeout: 10000 }
        );
        const finalTitles = await page.$$eval(".schedule-item .title", els => els.map(e => e.textContent?.trim()));
        expect(finalTitles.some(t => t?.includes("Amazing Grace"))).toBe(true);
    }, 20000);

    test("Resource Importer: Import OpenLP SQLite database into Song Library", async () => {
        // 1. Open Resource Importer modal
        await page.evaluate(() => {
            if (typeof (window as any).openImportModal === "function") {
                (window as any).openImportModal("local");
            }
        });
        await page.waitForSelector("#import-modal", { state: "visible" });

        // Switch to Local tab
        await page.click("#tab-btn-local");
        await page.waitForSelector("#import-view-local", { state: "visible" });

        // 2. Upload OpenLP songs.sqlite
        const openlpInput = await page.$("#file-openlp-import");
        expect(openlpInput).not.toBeNull();
        await openlpInput!.setInputFiles(SONGS_SQLITE_PATH);

        // 3. Wait for success status
        await page.waitForFunction(() => {
            const status = document.getElementById("openlp-import-status");
            return status && status.style.display !== "none" && status.textContent?.includes("Successful");
        }, { timeout: 15000 });

        const statusText = await page.$eval("#openlp-import-status", el => el.textContent);
        expect(statusText).toContain("OpenLP Import Successful");

        // Close import modal
        await page.click("#btn-close-import");
        await page.waitForSelector("#import-modal", { state: "hidden" });
    }, 20000);

    test("Resource Importer: Auto-Detect Local OpenLP Installation", async () => {
        // Open Resource Importer modal
        await page.evaluate(() => {
            if (typeof (window as any).openImportModal === "function") {
                (window as any).openImportModal("local");
            }
        });
        await page.waitForSelector("#import-modal", { state: "visible" });

        // Switch to Local tab
        await page.click("#tab-btn-local");
        await page.waitForSelector("#import-view-local", { state: "visible" });

        // Click Auto-Detect button
        await page.click("#btn-openlp-autodetect");

        // Wait for status message
        await page.waitForFunction(() => {
            const status = document.getElementById("openlp-import-status");
            return status && status.style.display !== "none" && (
                status.textContent?.includes("Imported") || status.textContent?.includes("OpenLP")
            );
        }, { timeout: 15000 });

        const statusText = await page.$eval("#openlp-import-status", el => el.textContent);
        expect(statusText?.length).toBeGreaterThan(5);

        // Close modal
        await page.click("#btn-close-import");
    }, 20000);
});
