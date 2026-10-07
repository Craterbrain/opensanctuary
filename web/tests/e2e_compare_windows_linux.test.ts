import { test, expect, beforeAll, afterAll, describe } from "bun:test";
import { chromium, type Browser, type Page } from "playwright";
import { spawn } from "bun";
import { resolve, join } from "path";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { spawnTestServer, waitForServerReady, teardownTestServer, ensureArtifactDir, type SpawnedTestServer } from "./e2e_helpers";

describe("E2E Visual Comparison: Linux vs Windows Release Binaries", () => {
  let linuxServer: SpawnedTestServer;
  let winServer: SpawnedTestServer;
  let browser: Browser;
  let linuxPage: Page;
  let winPage: Page;
  let linuxHostToken: string;
  let winHostToken: string;

  const LINUX_PORT = 9061;
  const WIN_PORT = 9062;
  let artifactDir: string;

  const sampleHymn = {
    id: "hymn_comparison",
    title: "Crown Him with Many Crowns",
    subtitle: "Diademata",
    notes: "Festive Easter Hymn",
    item_type: "song",
    author_or_ref: "Matthew Bridges & Godfrey Thring",
    slides: [
      {
        text: "Crown Him with many crowns,\nThe Lamb upon His throne;\nHark! how the heavenly anthem drowns\nAll music but its own!",
        header: null,
        label: "Verse 1",
        tag: "V1",
        background: "#0d1b2a",
        notes: null,
        elements: []
      },
      {
        text: "Awake, my soul, and sing\nOf Him who died for thee,\nAnd hail Him as thy matchless King\nThrough all eternity.",
        header: null,
        label: "Chorus",
        tag: "C",
        background: "#1b263b",
        notes: null,
        elements: []
      },
      {
        text: "Crown Him the Lord of peace,\nWhose power a sceptre sways\nFrom pole to pole, that wars may cease,\nAnd all be prayer and praise.",
        header: null,
        label: "Verse 2",
        tag: "V2",
        background: "#415a77",
        notes: null,
        elements: []
      }
    ],
    background: null,
    theme_name: null,
    is_expanded: false,
    is_section_header: false,
    arrangement: []
  };

  beforeAll(async () => {
    artifactDir = ensureArtifactDir();

    // 1. Spawn native Linux server -- the standard shape `spawnTestServer` covers.
    linuxServer = await spawnTestServer({
      port: LINUX_PORT,
      tempPrefix: "os-next-linux-comp-",
      readyPath: "/api/network/info",
    });

    // 2. Spawn Windows server via Wine -- bespoke (a different binary run
    // through `wine`, with its own relative --web-dir), so this one builds
    // its own temp dir/process rather than going through `spawnTestServer`;
    // still uses the shared `waitForServerReady` poll.
    const winTestDir = mkdtempSync(join(tmpdir(), "os-next-win-comp-"));
    const winBin = resolve(__dirname, "../../target/x86_64-pc-windows-gnu/release/os-next.exe");
    const winProc = spawn([
      "wine",
      winBin,
      "--headless",
      "--skip-first-time-setup",
      "--port", WIN_PORT.toString(),
      "--db-path", join(winTestDir, "win.db"),
      "--web-dir", "web"
    ], {
      cwd: resolve(__dirname, "../../"),
      stdout: "pipe",
      stderr: "pipe"
    });
    try {
      await waitForServerReady(WIN_PORT, { path: "/api/network/info" });
    } catch (e) {
      try { winProc.kill(); } catch (_) {}
      throw e;
    }
    winServer = { proc: winProc, testDir: winTestDir, dbPath: join(winTestDir, "win.db"), port: WIN_PORT };

    // /api/command now requires the host token for callers with no paired-
    // device token (docs/CLIENT_PAIRING.md) -- each server instance has its
    // own token, fetched here the same way a real console tab on 127.0.0.1
    // would (see web/tests/e2e_remote_mobile.test.ts for the same pattern).
    linuxHostToken = (await (await fetch(`http://127.0.0.1:${LINUX_PORT}/api/internal/host-token`)).json()).host_token;
    winHostToken = (await (await fetch(`http://127.0.0.1:${WIN_PORT}/api/internal/host-token`)).json()).host_token;

    // Seed schedule on both servers -- before either console page below
    // exists, so there's no real console yet to contend with for the "one
    // console at a time" lock (docs/CLIENT_PAIRING.md); any session id works.
    await fetch(`http://127.0.0.1:${LINUX_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "x-host-token": linuxHostToken, "x-console-session-id": "e2e-compare-seed" },
      body: JSON.stringify({ AddToSchedule: sampleHymn })
    });

    await fetch(`http://127.0.0.1:${WIN_PORT}/api/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "x-host-token": winHostToken, "x-console-session-id": "e2e-compare-seed" },
      body: JSON.stringify({ AddToSchedule: sampleHymn })
    });

    browser = await chromium.launch({ headless: true });

    // Open Linux page
    linuxPage = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    linuxPage.on("pageerror", err => console.log("LINUX PAGE ERR:", err.message));

    // Open Windows page
    winPage = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    winPage.on("pageerror", err => console.log("WIN PAGE ERR:", err.message));
  }, 60000);

  afterAll(async () => {
    if (browser) await browser.close();
    await teardownTestServer(linuxServer);
    await teardownTestServer(winServer);
    // Explicit timeout: bun's default hook timeout (5000ms) can be tight
    // for killing two real server processes -- one of them wine-wrapped,
    // which can take longer than the native binary to actually exit -- plus
    // removing two temp dirs.
  }, 15000);

  test("Action 1: Home Console & Presentation / Live State Display", async () => {
    // Navigate both pages
    await linuxPage.goto(`http://127.0.0.1:${LINUX_PORT}/`, { waitUntil: "networkidle" });
    await linuxPage.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });

    await winPage.goto(`http://127.0.0.1:${WIN_PORT}/`, { waitUntil: "networkidle" });
    await winPage.waitForFunction(() => (window as any).__APP_READY__ === true, { timeout: 15000 });

    await linuxPage.waitForTimeout(500);
    await winPage.waitForTimeout(500);

    // Capture Home Console Screenshots
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_console.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_console.png` });

    console.log("Action 1 captured: Home Console & Live Display");
  }, 30000);

  test("Action 2: Settings Dialog — Network Tab & Detected Interfaces", async () => {
    // Open Settings on Linux (F2)
    await linuxPage.keyboard.press("F2");
    await linuxPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await linuxPage.locator(".settings-category-btn:has-text('Network')").click();
    await linuxPage.waitForSelector(".network-overview-card", { state: "visible", timeout: 5000 });

    // Open Settings on Windows (F2)
    await winPage.keyboard.press("F2");
    await winPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await winPage.locator(".settings-category-btn:has-text('Network')").click();
    await winPage.waitForSelector(".network-overview-card", { state: "visible", timeout: 5000 });

    // Wait for LAN conflict check badges to settle on both
    await linuxPage.waitForFunction(() => {
      const el = document.getElementById("network-hostname-badge");
      return el && el.textContent && (el.textContent.includes("Available") || el.textContent.includes("LAN Conflict"));
    }, { timeout: 15000 });

    await winPage.waitForFunction(() => {
      const el = document.getElementById("network-hostname-badge");
      return el && el.textContent && (el.textContent.includes("Available") || el.textContent.includes("LAN Conflict"));
    }, { timeout: 15000 });

    // Screenshot top half (Overview + Hostname + Option 12 + Port)
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_network_tab.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_network_tab.png` });

    // Scroll down to reveal Dedicated Virtual Adapter and Detected Interfaces
    await linuxPage.locator("#settings-content").evaluate((el: HTMLElement) => {
      el.scrollTop = el.scrollHeight;
    });
    await winPage.locator("#settings-content").evaluate((el: HTMLElement) => {
      el.scrollTop = el.scrollHeight;
    });

    await linuxPage.waitForTimeout(400);
    await winPage.waitForTimeout(400);

    // Screenshot bottom half (Virtual Adapter + Interface Cards)
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_network_interfaces.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_network_interfaces.png` });

    // Close settings dialogs
    await linuxPage.keyboard.press("Escape");
    await winPage.keyboard.press("Escape");
    await linuxPage.waitForSelector("#options-modal", { state: "hidden", timeout: 5000 });
    await winPage.waitForSelector("#options-modal", { state: "hidden", timeout: 5000 });

    console.log("Action 2 captured: Settings Dialog Network Tab");
  }, 30000);

  test("Action 3: Schedule & Slide Navigation (Live Presentation Output)", async () => {
    // By this point both linuxPage/winPage are real, already-authenticated
    // consoles each holding the "one console at a time" lock on their own
    // server (docs/CLIENT_PAIRING.md) via their own real WS connection --
    // route through their own `window.sendCommand` (exposed by app_core.ts)
    // rather than a separate Node-side fetch with a different session id,
    // which would just get rejected as a second, different console.
    await linuxPage.evaluate(() => (window as any).sendCommand({ GoLive: { item_index: 0, slide_index: 0 } }));
    await winPage.evaluate(() => (window as any).sendCommand({ GoLive: { item_index: 0, slide_index: 0 } }));

    await linuxPage.waitForTimeout(400);
    await winPage.waitForTimeout(400);

    // Advance to next slide (Chorus)
    await linuxPage.evaluate(() => (window as any).sendCommand({ NextSlide: null }));
    await winPage.evaluate(() => (window as any).sendCommand({ NextSlide: null }));

    await linuxPage.waitForTimeout(500);
    await winPage.waitForTimeout(500);

    // Capture Live Slide Navigation Screenshots
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_slide_nav.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_slide_nav.png` });

    console.log("Action 3 captured: Schedule & Slide Navigation");
  }, 30000);

  test("Action 4: Desktop Remote QR Barcode Modal", async () => {
    // Click #btn-remote on Linux
    const linuxBtnRemote = linuxPage.locator("#btn-remote");
    await linuxBtnRemote.click();
    await linuxPage.waitForSelector("#remote-modal", { state: "visible", timeout: 5000 });
    await linuxPage.waitForSelector("#remote-qr-canvas", { state: "visible", timeout: 5000 });

    // Click #btn-remote on Windows
    const winBtnRemote = winPage.locator("#btn-remote");
    await winBtnRemote.click();
    await winPage.waitForSelector("#remote-modal", { state: "visible", timeout: 5000 });
    await winPage.waitForSelector("#remote-qr-canvas", { state: "visible", timeout: 5000 });

    // Give QR Canvas time to paint
    await linuxPage.waitForTimeout(400);
    await winPage.waitForTimeout(400);

    // Verify QR Canvas pixel rendering on both
    const checkQrCanvas = (canvas: HTMLCanvasElement) => {
      const ctx = canvas.getContext("2d");
      if (!ctx) return false;
      const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);
      let darkCount = 0;
      let lightCount = 0;
      for (let i = 0; i < imgData.data.length; i += 4) {
        if (imgData.data[i] < 50) darkCount++;
        else if (imgData.data[i] > 200) lightCount++;
      }
      return darkCount > 50 && lightCount > 50;
    };

    expect(await linuxPage.locator("#remote-qr-canvas").evaluate(checkQrCanvas)).toBe(true);
    expect(await winPage.locator("#remote-qr-canvas").evaluate(checkQrCanvas)).toBe(true);

    // Capture QR Modal Screenshots
    await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_remote_qr.png` });
    await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_remote_qr.png` });

    // Close remote modal
    await linuxPage.keyboard.press("Escape");
    await winPage.keyboard.press("Escape");
    await linuxPage.waitForSelector("#remote-modal", { state: "hidden", timeout: 5000 });
    await winPage.waitForSelector("#remote-modal", { state: "hidden", timeout: 5000 });

    console.log("Action 4 captured: Desktop Remote QR Barcode Modal");
  }, 30000);

  test("Action 5: DHCP Option 12 Broadcast Trigger & Conflict Validation", async () => {
    // Open Settings on Linux -> Network
    await linuxPage.keyboard.press("F2");
    await linuxPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await linuxPage.locator(".settings-category-btn:has-text('Network')").click();
    await linuxPage.waitForSelector("#btn-broadcast-option12", { state: "visible", timeout: 5000 });

    // Open Settings on Windows -> Network
    await winPage.keyboard.press("F2");
    await winPage.waitForSelector("#options-modal", { state: "visible", timeout: 5000 });
    await winPage.locator(".settings-category-btn:has-text('Network')").click();
    await winPage.waitForSelector("#btn-broadcast-option12", { state: "visible", timeout: 5000 });

    // Trigger Broadcast Option 12 Now on both
    await linuxPage.locator("#btn-broadcast-option12").click();
    await winPage.locator("#btn-broadcast-option12").click();

    // Verify broadcast badge appears with status on both
    await linuxPage.waitForFunction(() => {
      const badge = document.getElementById("network-broadcast-status");
      return badge && badge.style.display !== "none" && badge.textContent && badge.textContent.includes("Option 12");
    }, { timeout: 8000 });

    await winPage.waitForFunction(() => {
      const badge = document.getElementById("network-broadcast-status");
      return badge && badge.style.display !== "none" && badge.textContent && badge.textContent.includes("Option 12");
    }, { timeout: 8000 });

    if (!linuxPage.isClosed()) {
      await linuxPage.waitForTimeout(300);
      await linuxPage.screenshot({ path: `${artifactDir}/screenshot_linux_option12_broadcast.png` });
    }
    if (!winPage.isClosed()) {
      await winPage.waitForTimeout(300);
      await winPage.screenshot({ path: `${artifactDir}/screenshot_windows_option12_broadcast.png` });
    }

    console.log("Action 5 captured: DHCP Option 12 Broadcast Trigger & Conflict Validation");
  }, 30000);
});
