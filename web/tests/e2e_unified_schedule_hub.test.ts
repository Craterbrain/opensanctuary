import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn, type Subprocess } from "bun";
import { resolve, join } from "path";
import { mkdtempSync, rmSync } from "fs";
import { tmpdir } from "os";

describe("E2E Live Test: Unified Schedule Ingestion & Desktop Drag-and-Drop", () => {
    let serverProc: Subprocess;
    let browser: Browser;
    let page: Page;
    let testDir: string;
    let DB_PATH: string;
    const PORT = 9001;
    const EWSX_PATH = resolve(__dirname, "../../test1.ewsx");

    beforeAll(async () => {
        // Isolated OS temp dir: Database::new derives songs/bibles folders
        // from the db path's parent, so a project-root db path (the old
        // behavior here) meant every e2e run silently mutated the real
        // song library.
        testDir = mkdtempSync(join(tmpdir(), "os-next-e2e-"));
        DB_PATH = join(testDir, "test.db");

        // Launch headless OS-Next server
        const binaryPath = resolve(__dirname, "../../target/release/os-next");
        serverProc = spawn([
            binaryPath,
            "--headless",
            "--port", PORT.toString(),
            "--db-path", DB_PATH,
            "--web-dir", resolve(__dirname, "../")
        ], {
            cwd: resolve(__dirname, "../../"),
            stdout: "pipe",
            stderr: "pipe"
        });

        // Wait for server ready
        let ready = false;
        for (let i = 0; i < 40; i++) {
            try {
                const res = await fetch(`http://127.0.0.1:${PORT}/`);
                if (res.ok) {
                    ready = true;
                    break;
                }
            } catch (_) {}
            await new Promise(r => setTimeout(r, 250));
        }
        if (!ready) throw new Error("Server failed to start in 10s");

        // Launch Chromium
        browser = await chromium.launch({ headless: true });
        page = await browser.newPage();
        await page.goto(`http://127.0.0.1:${PORT}/`);
        await page.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });
        await page.waitForLoadState("networkidle");
    }, 45000);

    afterAll(async () => {
        if (browser) await browser.close();
        if (serverProc) {
            serverProc.kill();
            await serverProc.exited;
        }
        try { rmSync(testDir, { recursive: true, force: true }); } catch (_) {}
    });

    test("Unified Schedule Hub: inline mode toggle and single-step file ingestion", async () => {
        // 1. Open Schedule Hub via header button
        await page.click("#btn-sched-open");
        await page.waitForSelector("#open-modal", { state: "visible" });

        // Verify title and inline mode radio toggles
        const titleText = await page.$eval("#open-modal .modal-header span", el => el.textContent);
        expect(titleText).toContain("Open & Ingest Schedule");

        const replaceRadioChecked = await page.$eval("#sched-mode-replace", (el: any) => el.checked);
        expect(replaceRadioChecked).toBe(true);

        // 2. Select file with Replace mode
        const fileInput = await page.$("#file-schedule-input");
        expect(fileInput).not.toBeNull();
        await fileInput!.setInputFiles(EWSX_PATH);

        // In the unified hub, file loads in a single step without popping up secondary prompt modal
        await page.waitForSelector("#open-modal", { state: "hidden", timeout: 8000 });

        // Verify schedule items loaded
        await page.waitForSelector(".schedule-item", { timeout: 8000 });
        const items = await page.$$(".schedule-item");
        expect(items.length).toBeGreaterThan(0);
        const countAfterReplace = items.length;

        // 3. Re-open Schedule Hub and test Append mode
        await page.click("#btn-sched-open");
        await page.waitForSelector("#open-modal", { state: "visible" });

        // Switch to Append mode
        await page.click("#sched-mode-append");
        const appendRadioChecked = await page.$eval("#sched-mode-append", (el: any) => el.checked);
        expect(appendRadioChecked).toBe(true);

        // Load sample EasyWorship service button while Append mode is active
        await page.click("#btn-load-sample-ewsx");
        await page.waitForSelector("#open-modal", { state: "hidden", timeout: 8000 });

        // Items should now be doubled
        await page.waitForFunction((expected) => {
            return document.querySelectorAll('.schedule-item').length === expected;
        }, countAfterReplace * 2, { timeout: 8000 });

        const itemsAfterAppend = await page.$$(".schedule-item");
        expect(itemsAfterAppend.length).toBe(countAfterReplace * 2);
    });

    test("Resource Importer: 'Open Schedule Hub' cross-links directly to unified dialog", async () => {
        // Open Resource Importer
        await page.click("#btn-import");
        await page.waitForSelector("#import-modal", { state: "visible" });

        // Switch to Local tab
        await page.click("#tab-btn-local");
        await page.waitForSelector("#import-view-local", { state: "visible" });

        // Click Open Schedule Hub button
        const hubBtn = await page.$("#btn-open-schedule-hub");
        expect(hubBtn).not.toBeNull();
        await page.click("#btn-open-schedule-hub");

        // Verify Import modal closes and Schedule Hub opens
        await page.waitForSelector("#import-modal", { state: "hidden" });
        await page.waitForSelector("#open-modal", { state: "visible" });

        // Close Schedule Hub
        await page.click("#btn-close-open");
        await page.waitForSelector("#open-modal", { state: "hidden" });
    });

    test("Native Desktop Drag-and-Drop: split replace/append overlay responds to file drag events", async () => {
        // Simulate dragging external files over #schedule-panel
        await page.evaluate(() => {
            const panel = document.getElementById("schedule-panel");
            if (!panel) return;

            const dragEnterEvent = new DragEvent("dragenter", {
                bubbles: true,
                cancelable: true,
                dataTransfer: new DataTransfer()
            });
            dragEnterEvent.dataTransfer?.items.add(new File([""], "dummy.ewsx"));
            panel.dispatchEvent(dragEnterEvent);
        });

        // Overlay should appear
        await page.waitForSelector("#schedule-drop-overlay", { state: "visible" });
        const overlayVisible = await page.$eval("#schedule-drop-overlay", el => (el as HTMLElement).style.display);
        expect(overlayVisible).toBe("flex");

        // Split drop targets must exist
        const replaceZone = await page.$("#schedule-drop-zone-replace");
        const appendZone = await page.$("#schedule-drop-zone-append");
        expect(replaceZone).not.toBeNull();
        expect(appendZone).not.toBeNull();

        // Simulate hover on top half vs bottom half
        await page.evaluate(() => {
            const panel = document.getElementById("schedule-panel");
            const overlay = document.getElementById("schedule-drop-overlay");
            if (!panel || !overlay) return;
            const rect = overlay.getBoundingClientRect();

            // Hover top half (replace)
            const topOver = new DragEvent("dragover", {
                bubbles: true,
                cancelable: true,
                clientX: rect.left + rect.width / 2,
                clientY: rect.top + rect.height * 0.25,
                dataTransfer: new DataTransfer()
            });
            topOver.dataTransfer?.items.add(new File([""], "dummy.ewsx"));
            panel.dispatchEvent(topOver);
        });

        const replaceActive = await page.$eval("#schedule-drop-zone-replace", el => el.classList.contains("active-zone"));
        expect(replaceActive).toBe(true);

        // Escape cancels overlay
        await page.keyboard.press("Escape");
        await page.waitForFunction(() => {
            const overlay = document.getElementById("schedule-drop-overlay");
            return !overlay || overlay.style.display === "none";
        });
    });
});
